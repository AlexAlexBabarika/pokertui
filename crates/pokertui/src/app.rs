use std::hash::{Hash, Hasher};

use crossterm::event::KeyCode;
use poker_core::Card;
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

pub struct App {
    // `Option` so a turn can move the engine out (apply consumes it by value)
    // and move the result back without a placeholder. It is always `Some`
    // outside of `handle_key`; access it through `engine()`.
    engine: Option<HandState>,
    pub names: NameRegistry,
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
        let names = NameRegistry::demo_six();
        Self {
            engine: Some(engine),
            names,
            raise_to: None,
            game_over: false,
            equity: None,
        }
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
        let eq = hero_equity(
            key.hole,
            &key.board,
            key.live_opponents,
            EQUITY_ITERS,
            seed,
        );
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
            return true;
        }
        if self.engine().to_act.is_none() {
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
        // Only the hero (the human at the keyboard for pass-and-play) acts via keys.
        // In hot-seat, every active player IS the hero from their own perspective —
        // we accept input for whoever is to_act. This makes pass-and-play work.

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
                true
            }
            Err((restored, _)) => {
                self.engine = Some(restored);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        while app.engine().phase != Phase::Complete {
            app.handle_key(KeyCode::Char('f'));
        }
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

    fn fold_to_completion(app: &mut App) {
        // Fold every actor in turn until one player remains and the hand ends.
        while app.engine().phase != Phase::Complete {
            assert!(
                app.handle_key(KeyCode::Char('f')),
                "fold should be accepted"
            );
        }
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
