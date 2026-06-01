use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use poker_core::Card;
use poker_core::bot::{BotProfile, OpponentModel, decide};
use poker_core::equity::hero_equity;
use poker_core::holdem::{Action, HandConfig, HandState, Phase, PlayerId, apply, legal_actions};

use crate::adapter::{NameRegistry, to_presentation};
use crate::state::GameState;

/// Monte Carlo trials per equity estimate. ~10k gives ±~1% accuracy and runs
/// in a few milliseconds, far below the 50ms frame tick.
const EQUITY_ITERS: u32 = 10_000;

/// The inputs that determine the hero's equity. When this is unchanged between
/// frames the cached estimate is reused, keeping the simulation off the
/// per-frame repaint path.
#[derive(Clone, PartialEq, Eq, Hash)]
struct EquityKey {
    hole: [Card; 2],
    board: Vec<Card>,
    live_opponents: usize,
    /// False when the hero is folded/busted or the hand is complete — there is
    /// no live equity to show.
    hero_live: bool,
}

struct EquityCache {
    key: EquityKey,
    pct: u8,
}

/// Who drives a seat. The human plays the hero seat from the keyboard; every bot
/// seat is driven by the decision engine. `Remote` (networked play) slots in
/// here later without touching the rest of the table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Controller {
    Human,
    Bot(BotProfile),
}

impl Controller {
    pub fn is_human(self) -> bool {
        matches!(self, Controller::Human)
    }

    pub fn is_bot(self) -> bool {
        matches!(self, Controller::Bot(_))
    }
}

/// Visible delay before a bot acts, so a human can follow the table at a natural
/// pace rather than seeing the bots blink through their turns.
const BOT_ACTION_DELAY: Duration = Duration::from_millis(700);

/// The hero seat the UI renders cards for: the first human-controlled seat. An
/// all-bot table (allowed, e.g. a demo that just watches the bots play) has no
/// human, so it falls back to seat 0 as the spectator viewpoint.
fn hero_seat(seats: &[Controller]) -> PlayerId {
    seats
        .iter()
        .position(|c| c.is_human())
        .map(PlayerId)
        .unwrap_or(PlayerId(0))
}

pub struct App {
    // `Option` so a turn can move the engine out (apply consumes it by value)
    // and move the result back without a placeholder. It is always `Some`
    // outside of `handle_key`; access it through `engine()`.
    engine: Option<HandState>,
    pub names: NameRegistry,
    /// Who drives each seat, indexed by `PlayerId`. The hero is the first
    /// `Human` seat; the rest are bots (until `Remote` is added). `step` reads
    /// it to drive bot turns; `handle_key` reads it to ignore input on bot turns.
    seats: Vec<Controller>,
    /// Cross-hand opponent fold-frequency model fed to the bot decision engine.
    /// Folded the completed hand's action log into on completion (see
    /// `record_if_complete`); priors carry it until enough hands are seen.
    model: OpponentModel,
    /// Whether the current hand's log has already been recorded into `model`,
    /// so a completed hand is folded in exactly once. Reset when a hand is dealt.
    hand_recorded: bool,
    /// How long a bot waits before acting, so a human can follow the table.
    bot_delay: Duration,
    /// Earliest instant the bot whose turn it is may act. Armed when a bot turn
    /// is first seen and cleared once it acts (or the turn passes to a human).
    next_bot_action: Option<Instant>,
    /// Currently selected raise/bet to-level. `None` = untouched → use min.
    /// Reset to `None` after every committed action.
    raise_to: Option<u64>,
    /// Set once only one funded player remains: no further hands are dealt.
    game_over: bool,
    /// Cached hero equity for the current engine state. Recomputed only when
    /// the inputs change (a new street, a fold, a new hand), never per frame.
    equity: Option<EquityCache>,
}

impl App {
    pub fn new_demo_hand() -> Self {
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            // Seed from the wall clock so each launch deals a different hand.
            seed: Self::fresh_seed(),
        };
        let engine = HandState::new_hand(cfg, vec![10_000; 6]);
        let seats = Self::demo_seats();
        let mut names = NameRegistry::demo_six();
        // Keep the rendered hero consistent with the controller layout.
        names.hero = hero_seat(&seats);
        let model = OpponentModel::new(seats.len());
        Self {
            engine: Some(engine),
            names,
            model,
            hand_recorded: false,
            bot_delay: BOT_ACTION_DELAY,
            next_bot_action: None,
            seats,
            raise_to: None,
            game_over: false,
            equity: None,
        }
    }

    /// The demo table: the human sits at seat 3 ("you"), surrounded by a spread
    /// of bot personalities.
    fn demo_seats() -> Vec<Controller> {
        vec![
            Controller::Bot(BotProfile::tag()),
            Controller::Bot(BotProfile::calling_station()),
            Controller::Bot(BotProfile::balanced()),
            Controller::Human,
            Controller::Bot(BotProfile::rock()),
            Controller::Bot(BotProfile::tag()),
        ]
    }

    /// A fresh seed off the wall clock so each hand deals differently.
    fn fresh_seed() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xC0FFEE)
    }

    /// The current engine state. Always present between turns.
    pub fn engine(&self) -> &HandState {
        self.engine.as_ref().expect("engine present between turns")
    }

    /// `(min, max)` raise/bet *to-level* for the player currently to act, or
    /// `None` when no legal raise/bet exists (no actor, or stack too small to
    /// make a full min-raise — only an all-in would be legal there).
    fn raise_bounds(engine: &HandState) -> Option<(u64, u64)> {
        let actor = engine.to_act?;
        let min = if engine.current_bet == 0 {
            engine.config.big_blind
        } else {
            engine.current_bet + engine.min_raise
        };
        // Going all-in defines the ceiling for the to-level.
        let max = engine.round_bet[actor.0] + engine.stacks[actor.0];
        (max >= min).then_some((min, max))
    }

    pub fn view(&mut self) -> GameState {
        let equity = self.equity_pct();
        let engine = self.engine();
        let mut gs = to_presentation(engine, &self.names);
        gs.phase.raise_to = Self::raise_bounds(engine)
            .map(|(min, max)| self.raise_to.unwrap_or(min).clamp(min, max));
        gs.phase.equity = equity;
        gs.notice = self.notice();
        gs
    }

    /// The hero's cached win-chance percentage for the current state, refreshing
    /// the Monte Carlo estimate only when the inputs have changed.
    fn equity_pct(&mut self) -> u8 {
        let key = self.equity_key();
        if let Some(cache) = &self.equity
            && cache.key == key
        {
            return cache.pct;
        }
        let pct = Self::compute_equity(&key);
        self.equity = Some(EquityCache { key, pct });
        pct
    }

    /// Fingerprint of everything that moves the hero's equity. Owns its data so
    /// the engine borrow is released before the cache is written.
    fn equity_key(&self) -> EquityKey {
        let engine = self.engine();
        let hero = self.names.hero;
        let hero_live = engine.phase != Phase::Complete && !engine.folded[hero.0];
        let live_opponents = if hero_live {
            (0..engine.config.num_players)
                .filter(|&i| PlayerId(i) != hero && !engine.folded[i])
                .count()
        } else {
            0
        };
        EquityKey {
            hole: engine.hole[hero.0],
            board: engine.board.clone(),
            live_opponents,
            hero_live,
        }
    }

    /// Run (or short-circuit) the equity simulation for a key. Deterministic:
    /// the seed is derived from the key, so an unchanged situation always shows
    /// the same number.
    fn compute_equity(key: &EquityKey) -> u8 {
        if !key.hero_live {
            return 0;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        let seed = hasher.finish();
        let eq = hero_equity(key.hole, &key.board, key.live_opponents, EQUITY_ITERS, seed);
        eq.pct().round() as u8
    }

    /// End-of-hand banner: the game-over message once one player remains, or the
    /// "press a key" prompt while a finished hand is on screen. `None` mid-hand.
    fn notice(&self) -> Option<String> {
        let engine = self.engine();
        if engine.phase != Phase::Complete {
            return None;
        }
        if self.game_over || engine.funded_seats() < 2 {
            let winner = engine
                .stacks
                .iter()
                .position(|&s| s > 0)
                .map(|i| self.names.names[i].as_str())
                .unwrap_or("nobody");
            return Some(format!("GAME OVER · {winner} wins — press Q to quit"));
        }
        Some("hand complete · press any key for the next hand".into())
    }

    /// Returns true if the key was consumed (engine state may have changed).
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        // Once a hand is complete, any key deals the next one — unless only one
        // player still has chips, in which case the game is over.
        if self.engine().phase == Phase::Complete {
            if self.game_over {
                return false;
            }
            if self.engine().funded_seats() < 2 {
                self.game_over = true;
                return false;
            }
            let next = self.engine().next_hand(Self::fresh_seed());
            self.engine = Some(next);
            self.raise_to = None;
            self.hand_recorded = false;
            return true;
        }
        let Some(to_act) = self.engine().to_act else {
            return false;
        };
        // Bots act through `step`, not the keyboard — ignore input on their turn.
        if self.seats[to_act.0].is_bot() {
            return false;
        }

        // Bet-size selection: UI-only, mutates the selection, no engine change.
        if matches!(key, KeyCode::Up | KeyCode::Down) {
            if let Some((min, max)) = Self::raise_bounds(self.engine()) {
                let step = self.engine().config.small_blind;
                let cur = self.raise_to.unwrap_or(min);
                let next = if matches!(key, KeyCode::Up) {
                    cur.saturating_add(step).min(max)
                } else {
                    cur.saturating_sub(step).max(min)
                };
                self.raise_to = Some(next);
            }
            // Selection changed but the engine did not; the loop repaints every tick.
            return false;
        }

        let engine = self.engine();
        // Reaching here means a human seat is to act; map the key to its action.
        let action = match key {
            KeyCode::Char('f') | KeyCode::Char('F') => Some(Action::Fold),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if legal_actions(engine).contains(&Action::Check) {
                    Some(Action::Check)
                } else {
                    Some(Action::Call)
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                Self::raise_bounds(engine).map(|(min, max)| {
                    let to = self.raise_to.unwrap_or(min).clamp(min, max);
                    if engine.current_bet == 0 {
                        Action::Bet { to }
                    } else {
                        Action::Raise { to }
                    }
                })
            }
            KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::AllIn),
            _ => None,
        };

        let Some(action) = action else {
            return false;
        };
        // Move the engine out, apply, and move the result (or the unchanged
        // state on an illegal move) back. No allocation, no placeholder.
        let taken = self.engine.take().expect("engine present between turns");
        match apply(taken, action) {
            Ok(next) => {
                self.engine = Some(next);
                self.raise_to = None;
                self.record_if_complete();
                true
            }
            Err((restored, _)) => {
                self.engine = Some(restored);
                false
            }
        }
    }

    /// Advance the table by at most one bot action. Drives whichever seat is to
    /// act if it is a bot and its pacing delay has elapsed; a human turn, no
    /// actor, or a finished hand is a no-op. Returns true if a bot acted.
    ///
    /// Called once per tick by the main loop after key handling.
    pub fn step(&mut self) -> bool {
        self.step_at(Instant::now())
    }

    /// `step` with an injectable clock, so pacing is testable without waiting.
    fn step_at(&mut self, now: Instant) -> bool {
        let (to_act, complete) = {
            let e = self.engine();
            (e.to_act, e.phase == Phase::Complete)
        };
        // Nothing to drive, or it's the human's turn — keys handle that.
        let Some(to_act) = to_act else {
            self.next_bot_action = None;
            return false;
        };
        if complete || self.seats[to_act.0].is_human() {
            self.next_bot_action = None;
            return false;
        }

        // A bot is to act: arm the pacing timer the first time we see its turn,
        // then act once the delay has elapsed. At most one action per call.
        match self.next_bot_action {
            None => {
                self.next_bot_action = Some(now + self.bot_delay);
                false
            }
            Some(at) if now >= at => {
                self.act_bot(to_act);
                self.next_bot_action = None;
                true
            }
            Some(_) => false,
        }
    }

    /// Compute a bot's action with the decision engine and apply it, using the
    /// same take/apply/restore pattern as `handle_key`. An illegal action (which
    /// should be unreachable for a legal `decide`) restores state unchanged.
    fn act_bot(&mut self, seat: PlayerId) {
        let Controller::Bot(profile) = self.seats[seat.0] else {
            return;
        };
        let engine = self.engine();
        let rng_seed = Self::bot_seed(engine, seat);
        let action = decide(engine, seat, &profile, &self.model, rng_seed);

        let taken = self.engine.take().expect("engine present between turns");
        match apply(taken, action) {
            Ok(next) => {
                self.engine = Some(next);
                self.raise_to = None;
                self.record_if_complete();
            }
            Err((restored, _)) => {
                self.engine = Some(restored);
            }
        }
    }

    /// When the hand has just completed, fold its action log into the opponent
    /// model — exactly once, before the next hand is dealt — so the bots' fold
    /// equity estimates learn from how each seat actually played. A no-op until
    /// the hand reaches `Complete` and while it stays there after recording.
    fn record_if_complete(&mut self) {
        let engine = self.engine.as_ref().expect("engine present between turns");
        if engine.phase == Phase::Complete && !self.hand_recorded {
            self.model.record_hand(&engine.log);
            self.hand_recorded = true;
        }
    }

    /// Deterministic per-decision seed from `(hand seed, seat, log length)`, so a
    /// given spot always plays the same way — matching the codebase's seeded
    /// ethos and keeping bot play reproducible.
    fn bot_seed(engine: &HandState, seat: PlayerId) -> u64 {
        engine.config.seed
            ^ (seat.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (engine.log.len() as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero_is_the_first_human_seat() {
        let seats = vec![
            Controller::Bot(BotProfile::tag()),
            Controller::Bot(BotProfile::rock()),
            Controller::Human,
            Controller::Human,
        ];
        assert_eq!(
            hero_seat(&seats),
            PlayerId(2),
            "first human, not the second"
        );
    }

    #[test]
    fn an_all_bot_table_falls_back_to_seat_zero() {
        let seats = vec![
            Controller::Bot(BotProfile::tag()),
            Controller::Bot(BotProfile::rock()),
        ];
        assert_eq!(
            hero_seat(&seats),
            PlayerId(0),
            "no human → seat 0 is the spectator viewpoint"
        );
    }

    #[test]
    fn demo_table_seats_one_human_among_bots() {
        let app = App::new_demo_hand();
        assert_eq!(
            app.seats.len(),
            app.engine().config.num_players,
            "one controller per seat"
        );
        let humans = app.seats.iter().filter(|c| c.is_human()).count();
        assert_eq!(humans, 1, "exactly one human at the demo table");
        assert_eq!(
            app.seats.iter().filter(|c| !c.is_human()).count(),
            5,
            "the other five seats are bots"
        );
    }

    #[test]
    fn demo_hero_is_the_human_seat_and_matches_the_name_registry() {
        let app = App::new_demo_hand();
        let hero = app.names.hero;
        assert!(
            app.seats[hero.0].is_human(),
            "the hero seat must be human-controlled"
        );
        assert_eq!(
            hero,
            hero_seat(&app.seats),
            "rendered hero matches the controller layout"
        );
        assert_eq!(hero, PlayerId(3), "the demo human sits at seat 3 (\"you\")");
    }

    #[test]
    fn equity_tracks_whether_the_hero_is_live() {
        let mut app = App::new_demo_hand();
        // Preflop, the hero is in the hand against several opponents: a real,
        // non-trivial estimate.
        let live = app.view().phase.equity;
        assert!(
            live > 0 && live < 100,
            "a live hero has a real equity estimate, got {live}"
        );
        // The hero (UTG) is first to act preflop; folding sits them out and the
        // estimate collapses to zero.
        let hero = app.names.hero;
        app.handle_key(KeyCode::Char('f'));
        assert!(app.engine().folded[hero.0], "hero folded");
        assert_eq!(
            app.view().phase.equity,
            0,
            "a folded hero has no equity to show"
        );
    }

    #[test]
    fn equity_is_zero_at_showdown() {
        let mut app = App::new_demo_hand();
        fold_to_completion(&mut app);
        assert_eq!(
            app.view().phase.equity,
            0,
            "the panel stays quiet once the hand is complete"
        );
    }

    #[test]
    fn pressing_f_folds_current_actor() {
        let mut app = App::new_demo_hand();
        let actor_before = app.engine().to_act.unwrap();
        app.handle_key(KeyCode::Char('f'));
        assert!(app.engine().folded[actor_before.0]);
    }

    #[test]
    fn pressing_c_calls_when_facing_bet() {
        let mut app = App::new_demo_hand();
        let actor = app.engine().to_act.unwrap();
        app.handle_key(KeyCode::Char('c'));
        assert_eq!(app.engine().round_bet[actor.0], 100);
    }

    #[test]
    fn pressing_r_raises_to_min() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.engine().current_bet, 200);
    }

    #[test]
    fn up_arrow_raises_selection_by_one_small_blind() {
        let mut app = App::new_demo_hand();
        // Preflop, facing BB: min raise-to is 200, small blind is 50.
        app.handle_key(KeyCode::Up);
        assert_eq!(app.view().phase.raise_to, Some(250));
        app.handle_key(KeyCode::Up);
        assert_eq!(app.view().phase.raise_to, Some(300));
    }

    #[test]
    fn down_arrow_clamps_at_min() {
        let mut app = App::new_demo_hand();
        // Already at min (200); stepping down must not go below it.
        app.handle_key(KeyCode::Down);
        assert_eq!(app.view().phase.raise_to, Some(200));
    }

    #[test]
    fn up_arrow_clamps_at_all_in() {
        let mut app = App::new_demo_hand();
        for _ in 0..1000 {
            app.handle_key(KeyCode::Up);
        }
        let actor = app.engine().to_act.unwrap();
        let max = app.engine().round_bet[actor.0] + app.engine().stacks[actor.0];
        assert_eq!(app.view().phase.raise_to, Some(max));
    }

    #[test]
    fn r_commits_at_selected_amount() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Up); // 250
        app.handle_key(KeyCode::Up); // 300
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.engine().current_bet, 300);
    }

    #[test]
    fn selection_resets_after_a_committed_action() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Up); // 250
        app.handle_key(KeyCode::Char('r')); // commit raise to 250
        let min = app.engine().current_bet + app.engine().min_raise;
        assert_eq!(app.view().phase.raise_to, Some(min));
    }

    #[test]
    fn step_is_a_no_op_on_a_human_turn() {
        let mut app = App::new_demo_hand();
        // Seat 3 (the human) is UTG, first to act preflop.
        let hero = app.engine().to_act.unwrap();
        assert!(app.seats[hero.0].is_human());
        let log_before = app.engine().log.len();

        assert!(!app.step(), "step must not act on a human turn");
        assert_eq!(app.engine().to_act, Some(hero), "still the human's turn");
        assert_eq!(app.engine().log.len(), log_before, "no action applied");
    }

    #[test]
    fn step_paces_a_bot_action_until_the_delay_elapses() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Char('f')); // human folds → a bot is next to act
        let bot = app.engine().to_act.unwrap();
        assert!(app.seats[bot.0].is_bot());
        let log_before = app.engine().log.len();

        let t0 = Instant::now();
        // First sighting only arms the pacing timer.
        assert!(!app.step_at(t0));
        assert_eq!(
            app.engine().log.len(),
            log_before,
            "bot waits out the delay"
        );
        // Still inside the delay window.
        assert!(!app.step_at(t0 + Duration::from_millis(100)));
        assert_eq!(app.engine().log.len(), log_before);
        // Past the delay: the bot acts.
        assert!(app.step_at(t0 + BOT_ACTION_DELAY + Duration::from_millis(1)));
        assert!(
            app.engine().log.len() > log_before,
            "the bot acted once the delay elapsed"
        );
    }

    #[test]
    fn handle_key_is_ignored_on_a_bot_turn() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Char('f')); // human folds → a bot is next to act
        let bot = app.engine().to_act.unwrap();
        assert!(app.seats[bot.0].is_bot());
        let log_before = app.engine().log.len();

        let consumed = app.handle_key(KeyCode::Char('f'));
        assert!(!consumed, "keys do nothing while a bot is to act");
        assert_eq!(app.engine().to_act, Some(bot), "still the bot's turn");
        assert_eq!(app.engine().log.len(), log_before, "no action applied");
    }

    #[test]
    fn an_all_bot_table_plays_a_full_hand_to_completion() {
        let mut app = App::new_demo_hand();
        // Replace every seat with a bot; the step loop must play unaided.
        app.seats = vec![Controller::Bot(BotProfile::balanced()); 6];
        app.names.hero = hero_seat(&app.seats);

        let mut clock = Instant::now();
        for _ in 0..10_000 {
            if app.engine().phase == Phase::Complete {
                break;
            }
            clock += Duration::from_secs(1);
            app.step_at(clock);
        }
        assert_eq!(
            app.engine().phase,
            Phase::Complete,
            "an all-bot table must play a full hand to completion without stalling"
        );
    }

    #[test]
    fn completing_a_hand_records_folds_into_the_model() {
        let mut app = App::new_demo_hand();
        // Nothing observed yet: the human (UTG, seat 3) has not acted.
        assert_eq!(app.model.observed_fold_rate(PlayerId(3)), None);

        fold_to_completion(&mut app);
        assert_eq!(app.engine().phase, Phase::Complete);

        // The human folded UTG facing the big blind — a fold to a bet.
        assert_eq!(
            app.model.observed_fold_rate(PlayerId(3)),
            Some(1.0),
            "the human's fold to the BB is folded into the model"
        );
    }

    #[test]
    fn a_completed_hand_is_recorded_exactly_once() {
        let mut app = App::new_demo_hand();
        fold_to_completion(&mut app);

        // Seat 3 faced exactly one bet and folded it. With a zero prior and a
        // pot-sized bet (scaling 1.0), p_fold = folded/(K + faced) = 1/(4+1) =
        // 0.2. A double-record would make it 2/(4+2) = 0.333…, so this pins the
        // recording to a single pass over the log.
        let p = app.model.p_fold(PlayerId(3), 100, 100, 0.0);
        assert!(
            (p - 0.2).abs() < 1e-9,
            "expected a single record (0.2), got {p}"
        );

        // Idling at `Complete` (extra steps/redraws) must not re-record.
        app.step();
        app.step();
        let p_again = app.model.p_fold(PlayerId(3), 100, 100, 0.0);
        assert!((p_again - p).abs() < 1e-9, "completion must not re-record");
    }

    #[test]
    fn the_model_persists_across_hands() {
        let mut app = App::new_demo_hand();
        fold_to_completion(&mut app);
        assert_eq!(app.model.observed_fold_rate(PlayerId(3)), Some(1.0));

        // Deal the next hand; the accumulated observations must carry over.
        let dealt = app.handle_key(KeyCode::Char(' '));
        assert!(dealt, "a key at COMPLETE deals the next hand");
        assert_eq!(app.engine().phase, Phase::Preflop);
        assert_eq!(
            app.model.observed_fold_rate(PlayerId(3)),
            Some(1.0),
            "the opponent model is cross-hand, not reset between hands"
        );
    }

    fn fold_to_completion(app: &mut App) {
        // Drive the hand to completion: the human folds at every turn, the bots
        // play their own lines through the paced step loop. Either way the hand
        // reaches `Complete`.
        let mut clock = Instant::now();
        for _ in 0..10_000 {
            match app.engine().to_act {
                _ if app.engine().phase == Phase::Complete => return,
                Some(p) if app.seats[p.0].is_human() => {
                    app.handle_key(KeyCode::Char('f'));
                }
                Some(_) => {
                    clock += Duration::from_secs(1);
                    app.step_at(clock);
                }
                None => return,
            }
        }
        panic!("hand did not complete");
    }

    #[test]
    fn any_key_at_complete_deals_next_hand_carrying_stacks() {
        let mut app = App::new_demo_hand();
        fold_to_completion(&mut app);
        assert_eq!(app.engine().phase, Phase::Complete);
        let total_before: u64 = app.engine().stacks.iter().sum();

        let consumed = app.handle_key(KeyCode::Char(' ')); // any key advances
        assert!(consumed, "a key at COMPLETE should deal the next hand");
        assert_eq!(app.engine().phase, Phase::Preflop, "fresh hand dealt");
        assert_eq!(app.engine().board.len(), 0, "new board");
        // Chips are conserved: the new blinds just moved from stacks into the pot.
        let total_after: u64 =
            app.engine().stacks.iter().sum::<u64>() + app.engine().contributed.iter().sum::<u64>();
        assert_eq!(total_after, total_before, "chips carry over between hands");
    }

    #[test]
    fn key_does_nothing_once_only_one_player_is_funded() {
        let mut app = App::new_demo_hand();
        fold_to_completion(&mut app);
        // Force a busted table: a single funded seat remains.
        app.engine.as_mut().unwrap().stacks = vec![0, 0, 0, 60_000, 0, 0];

        let consumed = app.handle_key(KeyCode::Char(' '));
        assert!(!consumed, "no next hand with fewer than two funded seats");
        assert_eq!(
            app.engine().phase,
            Phase::Complete,
            "stays on the last hand"
        );
        assert!(
            app.view().notice.unwrap().contains("GAME OVER"),
            "game-over banner shown"
        );
    }

    #[test]
    fn game_over_notice_names_the_surviving_player() {
        let mut app = App::new_demo_hand();
        fold_to_completion(&mut app);
        app.engine.as_mut().unwrap().stacks = vec![0, 0, 0, 60_000, 0, 0];
        app.handle_key(KeyCode::Char(' '));
        let notice = app.view().notice.expect("game-over notice present");
        assert!(
            notice.contains("you"),
            "names the surviving player (hero idx 3)"
        );
    }

    #[test]
    fn complete_hand_shows_a_continue_hint() {
        let mut app = App::new_demo_hand();
        fold_to_completion(&mut app);
        let notice = app
            .view()
            .notice
            .expect("continue hint present at COMPLETE");
        assert!(
            notice.to_lowercase().contains("key"),
            "hint mentions pressing a key"
        );
    }
}
