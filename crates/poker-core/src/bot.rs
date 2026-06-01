use crate::equity::hero_equity;
use crate::holdem::{Action, HandState, LogEntry, LogKind, PlayerId, legal_actions};
use rand::RngExt;
use rand::SeedableRng;
use rand::rngs::StdRng;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotProfile {
    /// Display name of the personality.
    pub name: &'static str,
    /// Extra equity, above raw pot odds, required to call a bet. Higher means
    /// the bot folds more marginal hands.
    pub call_threshold: f64,
    /// Equity at or above which the bot wants to value bet/raise an unbet or
    /// already-bet pot.
    pub value_threshold: f64,
    /// How often the bot will fire a bluff/semi-bluff in close spots (0..=1).
    pub bluff_freq: f64,
    /// Bet/raise sizing multiplier applied to the pot-fraction sizing.
    pub aggression: f64,
    /// Softmax temperature for mixed strategies. Higher means the bot
    /// randomizes more between close-EV actions.
    pub mix_temperature: f64,
    /// Prior probability an opponent folds to a bet, used before enough
    /// observations exist to estimate it.
    pub fold_equity_prior: f64,
}

impl BotProfile {
    /// Tight-aggressive: high call/value thresholds, moderate bluffing, large
    /// sizing.
    pub const fn tag() -> Self {
        Self {
            name: "TAG",
            call_threshold: 0.05,
            value_threshold: 0.65,
            bluff_freq: 0.12,
            aggression: 1.0,
            mix_temperature: 0.10,
            fold_equity_prior: 0.40,
        }
    }

    /// Loose-passive calling station: low call threshold, rarely raises,
    /// small/no bluffs.
    pub const fn calling_station() -> Self {
        Self {
            name: "Calling Station",
            call_threshold: 0.0,
            value_threshold: 0.55,
            bluff_freq: 0.02,
            aggression: 0.5,
            mix_temperature: 0.08,
            fold_equity_prior: 0.25,
        }
    }

    /// Balanced/tricky: mid thresholds, higher bluff frequency, wider mixing.
    pub const fn balanced() -> Self {
        Self {
            name: "Balanced",
            call_threshold: 0.03,
            value_threshold: 0.60,
            bluff_freq: 0.25,
            aggression: 0.9,
            mix_temperature: 0.20,
            fold_equity_prior: 0.45,
        }
    }

    /// Rock: very high thresholds, almost never bluffs.
    pub const fn rock() -> Self {
        Self {
            name: "Rock",
            call_threshold: 0.10,
            value_threshold: 0.75,
            bluff_freq: 0.01,
            aggression: 0.8,
            mix_temperature: 0.05,
            fold_equity_prior: 0.35,
        }
    }

    /// All bundled presets, for menus and tests.
    pub const ALL: [BotProfile; 4] = [
        Self::tag(),
        Self::calling_station(),
        Self::balanced(),
        Self::rock(),
    ];
}

/// How many pseudo-observations the personality's prior is worth when blending
/// it with a seat's measured fold rate. Larger means the prior dominates until
/// more real hands are seen.
const PRIOR_WEIGHT: f64 = 4.0;

/// Per-seat record of how often an opponent folds when facing a bet, plus the
/// `p_fold` estimate the decision engine reads. Lives here so the pure engine
/// can consume it; `pokertui` owns an instance and feeds it completed hands.
#[derive(Debug, Clone, PartialEq)]
pub struct OpponentModel {
    seats: Vec<FoldStats>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct FoldStats {
    /// Times this seat acted while there was an unmatched bet to call.
    faced: u32,
    /// Of those, times the seat folded.
    folded: u32,
}

impl OpponentModel {
    /// A blank model with no observations for `num_seats` seats.
    pub fn new(num_seats: usize) -> Self {
        Self {
            seats: vec![FoldStats::default(); num_seats],
        }
    }

    /// Measured fold-to-bet rate for a seat, or `None` if it has never faced a
    /// bet yet.
    pub fn observed_fold_rate(&self, seat: PlayerId) -> Option<f64> {
        let s = self.seats[seat.0];
        (s.faced > 0).then(|| s.folded as f64 / s.faced as f64)
    }

    /// Fold-equity estimate: how likely `seat` is to fold to a `bet` into `pot`.
    ///
    /// Blends the acting bot's `prior` with this seat's observed fold rate
    /// (weighting the prior more when few hands have been seen), then scales by
    /// the bet's size relative to the pot. The result is clamped to a
    /// probability.
    pub fn p_fold(&self, seat: PlayerId, bet: u64, pot: u64, prior: f64) -> f64 {
        let s = self.seats[seat.0];
        // Bayesian shrinkage toward the prior: the prior counts as PRIOR_WEIGHT
        // folds-out-of-PRIOR_WEIGHT, so with zero observations this is `prior`.
        let blended = (prior * PRIOR_WEIGHT + s.folded as f64) / (PRIOR_WEIGHT + s.faced as f64);
        (blended * size_scaling(bet, pot)).clamp(0.0, 1.0)
    }

    /// Fold a completed hand's action log into the per-seat fold stats. Replays
    /// the betting from the amounts the log records (blinds, bets, raises) to
    /// decide, for each action, whether the seat was facing an unmatched bet.
    ///
    /// `Call`/`AllIn` carry no amount in the log, so they are treated as
    /// matching the current bet — an accepted approximation for the rare
    /// all-in-over-bet case.
    pub fn record_hand(&mut self, log: &[LogEntry]) {
        let mut round_bet = vec![0u64; self.seats.len()];
        let mut current_bet = 0u64;

        for entry in log {
            match &entry.kind {
                LogKind::DealFlop(_) | LogKind::DealTurn(_) | LogKind::DealRiver(_) => {
                    round_bet.iter_mut().for_each(|r| *r = 0);
                    current_bet = 0;
                }
                LogKind::PostBlind { amount, .. } => {
                    if let Some(PlayerId(p)) = entry.actor {
                        round_bet[p] += amount;
                        current_bet = current_bet.max(round_bet[p]);
                    }
                }
                LogKind::Action(action) => {
                    let Some(PlayerId(p)) = entry.actor else {
                        continue;
                    };
                    let facing = current_bet > round_bet[p];
                    match action {
                        Action::Fold => {
                            if facing {
                                self.seats[p].faced += 1;
                                self.seats[p].folded += 1;
                            }
                        }
                        Action::Check => {}
                        Action::Call | Action::AllIn => {
                            if facing {
                                self.seats[p].faced += 1;
                            }
                            round_bet[p] = round_bet[p].max(current_bet);
                            current_bet = current_bet.max(round_bet[p]);
                        }
                        Action::Bet { to } | Action::Raise { to } => {
                            if facing {
                                self.seats[p].faced += 1;
                            }
                            round_bet[p] = *to;
                            current_bet = current_bet.max(*to);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Scales the fold estimate by how big a bet is relative to the pot: small bets
/// get called more (scaling below 1), pot-sized bets are the reference (1.0),
/// overbets fold opponents out more often (above 1, later clamped). Monotonic
/// non-decreasing in `bet / pot`.
fn size_scaling(bet: u64, pot: u64) -> f64 {
    let ratio = if pot == 0 {
        1.0
    } else {
        bet as f64 / pot as f64
    };
    0.5 + 0.5 * ratio
}

/// Monte Carlo trials per bot equity estimate. Lower than the UI's 10k: a bot
/// makes many decisions per hand and ±~2% is plenty to choose a line.
const BOT_EQUITY_ITERS: u32 = 2_000;

/// Base bet/raise size as a fraction of the pot, before the profile's
/// `aggression` multiplier.
const BASE_BET_FRACTION: f64 = 0.6;

/// When the best action beats the runner-up by more than this (as a fraction of
/// the pot), take it outright; otherwise the close actions are mixed.
const STRICT_MARGIN: f64 = 0.15;

/// One scored action under consideration.
struct Candidate {
    action: Action,
    ev: f64,
}

/// A constructed aggressive line (bet, raise, or shove) together with the chip
/// amounts the EV math needs.
struct Aggressive {
    action: Action,
    /// Chips this seat adds on this action (call + raise portion).
    invest: u64,
    /// Amount the bet/raise puts on top of the current bet — what pressures
    /// opponents, used for the fold-equity size scaling.
    increment: u64,
}

/// Decide one action for `seat` to take in `state`. The result is always a legal
/// action. Pure and deterministic in `rng_seed`: the same situation and seed
/// always yields the same action, so it is reproducible and property-testable.
///
/// Combines a Monte Carlo equity estimate, pot odds, and the `OpponentModel`'s
/// fold-equity estimate into an EV for each candidate line, then takes the best
/// — mixing between close-EV lines (softmax, `mix_temperature`) so the bot is
/// not trivially readable.
pub fn decide(
    state: &HandState,
    seat: PlayerId,
    profile: &BotProfile,
    model: &OpponentModel,
    rng_seed: u64,
) -> Action {
    let i = seat.0;
    let legal = legal_actions(state);
    debug_assert!(
        !legal.is_empty(),
        "decide called for a seat that cannot act"
    );
    if legal.is_empty() {
        return Action::Fold;
    }

    let n = state.config.num_players;
    let pot: u64 = state.contributed.iter().sum();
    let to_call = state.current_bet.saturating_sub(state.round_bet[i]);
    let stack = state.stacks[i];
    let pot_scale = pot.max(state.config.big_blind) as f64;

    // Opponents still in the hand contest the showdown (drive equity); those not
    // yet all-in can still fold (drive fold equity).
    let live_opponents = (0..n).filter(|&j| j != i && !state.folded[j]).count();
    let foldable: Vec<usize> = (0..n)
        .filter(|&j| j != i && !state.folded[j] && !state.all_in[j])
        .collect();

    let equity = hero_equity(
        state.hole[i],
        &state.board,
        live_opponents,
        BOT_EQUITY_ITERS,
        rng_seed,
    )
    .pct()
        / 100.0;

    // Choices are decorrelated from the equity simulation's seed.
    let mut rng = StdRng::seed_from_u64(rng_seed ^ 0x9E37_79B9_7F4A_7C15);

    // Passive candidates. When the pot is unbet, checking dominates folding (a
    // free look is never worse), so folding is dropped from the running.
    let mut candidates: Vec<Candidate> = Vec::new();
    if to_call == 0 {
        candidates.push(Candidate {
            action: Action::Check,
            ev: equity * pot as f64,
        });
    } else {
        candidates.push(Candidate {
            action: Action::Fold,
            ev: 0.0,
        });
        // EV(call) with a personality margin: `call_threshold` is the extra
        // equity over raw pot odds the bot demands before calling.
        let ev_call = equity * (pot + to_call) as f64
            - to_call as f64
            - profile.call_threshold * (pot + to_call) as f64;
        candidates.push(Candidate {
            action: Action::Call,
            ev: ev_call,
        });
    }
    let passive_best = candidates.iter().map(|c| c.ev).fold(f64::MIN, f64::max);

    // Aggressive candidate (bet / raise / shove), if one is legal here.
    if let Some(aggr) = aggressive_candidate(state, i, pot, to_call, stack, profile) {
        let p_fold: f64 = if foldable.is_empty() {
            0.0
        } else {
            foldable
                .iter()
                .map(|&j| model.p_fold(PlayerId(j), aggr.increment, pot, profile.fold_equity_prior))
                .product()
        };
        let invest = aggr.invest as f64;
        // EV(bet/raise): win the pot now when everyone folds, otherwise realize
        // equity in the inflated pot. The fold-equity term is what makes a
        // semi-bluff correct below raw pot odds.
        let ev_aggr =
            p_fold * pot as f64 + (1.0 - p_fold) * (equity * (pot as f64 + 2.0 * invest) - invest);

        // A value bet (equity past the threshold) is always on the table. A
        // bluff/semi-bluff is taken only when fold equity already makes it the
        // better line, or — for thinner, balancing bluffs — on a `bluff_freq`
        // roll.
        let is_value = equity >= profile.value_threshold;
        let justified = ev_aggr >= passive_best;
        let bluff_roll = rng.random::<f64>();
        if is_value || justified || bluff_roll < profile.bluff_freq {
            candidates.push(Candidate {
                action: aggr.action,
                ev: ev_aggr,
            });
        }
    }

    select(&candidates, pot_scale, profile.mix_temperature, &mut rng)
}

/// Build the aggressive line for the seat to act, sized as a pot fraction scaled
/// by `aggression` and clamped to the legal `[min_to, max_to]` range — mapped to
/// `AllIn` when it reaches the stack ceiling. Returns `None` when no bet/raise
/// is legal (e.g. already acted with no full re-raise available, or too short to
/// raise without it being a plain call).
fn aggressive_candidate(
    state: &HandState,
    i: usize,
    pot: u64,
    to_call: u64,
    stack: u64,
    profile: &BotProfile,
) -> Option<Aggressive> {
    if stack == 0 {
        return None;
    }
    let bb = state.config.big_blind;
    let sb = state.config.small_blind;
    let round_bet_i = state.round_bet[i];
    let max_to = round_bet_i + stack;
    let frac = (BASE_BET_FRACTION * profile.aggression).clamp(0.25, 1.5);

    // A shove that raises the action, used whenever the seat is too short (or has
    // already acted) to make a clean min-raise.
    let shove = |state: &HandState| Aggressive {
        action: Action::AllIn,
        invest: stack,
        increment: max_to.saturating_sub(state.current_bet),
    };

    if state.current_bet == 0 {
        // Betting into an unbet pot.
        let min_to = bb;
        if max_to < min_to {
            return Some(shove(state));
        }
        let desired = round_to(frac * pot as f64, sb);
        let to = desired.clamp(min_to, max_to);
        if to >= max_to {
            return Some(shove(state));
        }
        Some(Aggressive {
            action: Action::Bet { to },
            invest: to - round_bet_i,
            increment: to, // current_bet is 0
        })
    } else {
        // Raising over an existing bet.
        let can_raise = !state.acted_this_round[i];
        let min_to = state.current_bet + state.min_raise;
        if can_raise && max_to >= min_to {
            let desired = round_to(state.current_bet as f64 + frac * (pot + to_call) as f64, sb);
            let to = desired.clamp(min_to, max_to);
            if to >= max_to {
                return Some(shove(state));
            }
            Some(Aggressive {
                action: Action::Raise { to },
                invest: to - round_bet_i,
                increment: to - state.current_bet,
            })
        } else if max_to > state.current_bet {
            // Can't make a clean min-raise, but a shove still raises the bet.
            Some(shove(state))
        } else {
            // A shove wouldn't even reach the current bet — that's just a call.
            None
        }
    }
}

/// Round a chip amount to the nearest small blind for tidy sizing.
fn round_to(value: f64, step: u64) -> u64 {
    let v = value.max(0.0).round() as u64;
    if step == 0 {
        return v;
    }
    ((v + step / 2) / step) * step
}

/// Pick a final action: take the best EV outright when it clears the runner-up
/// by `STRICT_MARGIN`, otherwise sample the close field with a softmax over EV
/// (temperature `temp`) so close spots are mixed rather than predictable.
fn select(candidates: &[Candidate], pot_scale: f64, temp: f64, rng: &mut StdRng) -> Action {
    let best = candidates
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.ev.partial_cmp(&b.1.ev).unwrap())
        .map(|(idx, _)| idx)
        .expect("at least one candidate");
    if candidates.len() == 1 {
        return candidates[best].action;
    }

    let best_ev = candidates[best].ev;
    let second_ev = candidates
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != best)
        .map(|(_, c)| c.ev)
        .fold(f64::MIN, f64::max);
    if (best_ev - second_ev) / pot_scale > STRICT_MARGIN {
        return candidates[best].action;
    }

    // Softmax over pot-normalized EVs; subtract the max first for stability.
    let t = temp.max(1e-3);
    let max_norm = best_ev / pot_scale;
    let weights: Vec<f64> = candidates
        .iter()
        .map(|c| ((c.ev / pot_scale - max_norm) / t).exp())
        .collect();
    let sum: f64 = weights.iter().sum();
    let mut r = rng.random::<f64>() * sum;
    for (c, w) in candidates.iter().zip(&weights) {
        if r < *w {
            return c.action;
        }
        r -= *w;
    }
    candidates[best].action
}

#[cfg(test)]
mod opponent_model_tests {
    use super::*;
    use crate::Card;
    use crate::holdem::Action;

    fn blind(seat: usize, amount: u64, is_big: bool) -> LogEntry {
        LogEntry {
            actor: Some(PlayerId(seat)),
            kind: LogKind::PostBlind { amount, is_big },
        }
    }

    fn act(seat: usize, action: Action) -> LogEntry {
        LogEntry {
            actor: Some(PlayerId(seat)),
            kind: LogKind::Action(action),
        }
    }

    #[test]
    fn accumulates_fold_rate_from_a_synthetic_log() {
        // 4-handed preflop: SB 1 posts 50, BB 2 posts 100. UTG 3 folds to the
        // BB, button 0 calls, SB 1 folds (50 < 100, still facing), BB 2 checks
        // its option (nothing to call).
        let log = vec![
            blind(1, 50, false),
            blind(2, 100, true),
            act(3, Action::Fold),
            act(0, Action::Call),
            act(1, Action::Fold),
            act(2, Action::Check),
        ];
        let mut model = OpponentModel::new(4);
        model.record_hand(&log);

        assert_eq!(
            model.observed_fold_rate(PlayerId(3)),
            Some(1.0),
            "UTG folded to a bet"
        );
        assert_eq!(
            model.observed_fold_rate(PlayerId(0)),
            Some(0.0),
            "button faced and called"
        );
        assert_eq!(
            model.observed_fold_rate(PlayerId(1)),
            Some(1.0),
            "SB folded facing the BB"
        );
        assert_eq!(
            model.observed_fold_rate(PlayerId(2)),
            None,
            "BB never faced a bet"
        );
    }

    #[test]
    fn a_check_when_unbet_does_not_count_as_facing() {
        // Postflop everyone checks: no one faces a bet.
        let log = vec![
            blind(0, 50, false),
            blind(1, 100, true),
            act(0, Action::Call),
            act(1, Action::Check),
            LogEntry {
                actor: None,
                kind: LogKind::DealFlop([Card::parse("2c"), Card::parse("7d"), Card::parse("Jh")]),
            },
            act(0, Action::Check),
            act(1, Action::Check),
        ];
        let mut model = OpponentModel::new(2);
        model.record_hand(&log);
        // Seat 0 faced the BB preflop (50 < 100) but seat 1 never faced a bet.
        assert_eq!(model.observed_fold_rate(PlayerId(1)), None);
    }

    #[test]
    fn with_zero_observations_p_fold_is_the_prior_at_a_pot_sized_bet() {
        let model = OpponentModel::new(3);
        // bet == pot makes size_scaling exactly 1.0, isolating the prior.
        let p = model.p_fold(PlayerId(1), 100, 100, 0.4);
        assert!((p - 0.4).abs() < 1e-9, "expected prior 0.4, got {p}");
    }

    #[test]
    fn observations_pull_the_estimate_toward_the_measured_rate() {
        let mut model = OpponentModel::new(2);
        // Seat 1 folds to a bet eight hands running.
        for _ in 0..8 {
            model.record_hand(&[
                blind(0, 50, false),
                blind(1, 100, true),
                act(0, Action::Bet { to: 300 }),
                act(1, Action::Fold),
            ]);
        }
        // A nitty prior of 0.2 should still be dragged well up by the folds.
        let p = model.p_fold(PlayerId(1), 100, 100, 0.2);
        assert!(p > 0.6, "heavy folding should raise the estimate, got {p}");
    }

    #[test]
    fn size_scaling_is_monotonic_in_bet_over_pot() {
        let model = OpponentModel::new(1);
        let pot = 100;
        let mut last = 0.0;
        for bet in [10u64, 50, 100, 200] {
            let p = model.p_fold(PlayerId(0), bet, pot, 0.4);
            assert!(
                p >= last,
                "p_fold must not decrease as the bet grows: {p} < {last}"
            );
            last = p;
        }
    }
}

#[cfg(test)]
mod decide_tests {
    use super::*;
    use crate::Card;
    use crate::holdem::{HandConfig, Phase, apply};

    fn cards(s: &str) -> Vec<Card> {
        s.split_whitespace().map(Card::parse).collect()
    }

    fn hole(s: &str) -> [Card; 2] {
        let v = cards(s);
        [v[0], v[1]]
    }

    /// A profile that strips away the stochastic knobs (no bluffing, never a
    /// value bet on equity alone) so a test isolates one behavior. Fold-equity
    /// prior is the dial that turns fold equity on or off.
    fn plain_profile(fold_equity_prior: f64) -> BotProfile {
        BotProfile {
            name: "test",
            call_threshold: 0.0,
            value_threshold: 0.95,
            bluff_freq: 0.0,
            aggression: 1.0,
            mix_temperature: 0.05,
            fold_equity_prior,
        }
    }

    /// Build a controllable spot: `hero` heads-up against `opp_count` live
    /// opponents, with a chosen hole, board, current bet, and pot. Stacks are
    /// deep unless overridden by the caller afterwards.
    fn spot(
        hero: usize,
        opp_count: usize,
        hero_hole: &str,
        board: &str,
        current_bet: u64,
        hero_round_bet: u64,
        pot: u64,
    ) -> (HandState, PlayerId) {
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 1,
        };
        let mut s = HandState::new_hand(cfg, vec![10_000; 6]);
        let n = 6;

        s.hole[hero] = hole(hero_hole);
        s.board = cards(board);
        s.phase = match s.board.len() {
            0 => Phase::Preflop,
            3 => Phase::Flop,
            4 => Phase::Turn,
            _ => Phase::River,
        };

        // Live seats: the hero plus the first `opp_count` other seats.
        let live: Vec<usize> = std::iter::once(hero)
            .chain((0..n).filter(|&j| j != hero))
            .take(opp_count + 1)
            .collect();
        s.folded = (0..n).map(|j| !live.contains(&j)).collect();
        s.all_in = vec![false; n];
        s.acted_this_round = vec![false; n];
        s.stacks = vec![10_000; n];

        s.current_bet = current_bet;
        s.min_raise = 100;
        s.round_bet = vec![0; n];
        s.round_bet[hero] = hero_round_bet;
        for &j in &live {
            if j != hero {
                s.round_bet[j] = current_bet;
            }
        }

        // Put the pot on the live opponents (plus the hero's own committed chips).
        s.contributed = vec![0; n];
        s.contributed[hero] = hero_round_bet;
        let remaining = pot.saturating_sub(hero_round_bet);
        let opps: Vec<usize> = live.iter().copied().filter(|&j| j != hero).collect();
        if let Some(&first) = opps.first() {
            s.contributed[first] = remaining;
        }

        s.to_act = Some(PlayerId(hero));
        s.winners = Vec::new();
        s.pots = Vec::new();
        (s, PlayerId(hero))
    }

    fn is_aggressive(a: Action) -> bool {
        matches!(a, Action::Bet { .. } | Action::Raise { .. } | Action::AllIn)
    }

    #[test]
    fn nut_hand_on_a_made_board_always_bets() {
        // Royal flush on a full board: unbeatable, so it bets for value.
        let (s, hero) = spot(0, 1, "Ah Kh", "Qh Jh Th 2c 3d", 0, 0, 100);
        let model = OpponentModel::new(6);
        for seed in 0..20 {
            let a = decide(&s, hero, &BotProfile::tag(), &model, seed);
            assert!(
                is_aggressive(a),
                "nut hand should bet/raise, got {a:?} (seed {seed})"
            );
        }
    }

    #[test]
    fn trash_with_no_fold_equity_folds_to_a_bet() {
        // 7-high facing a pot-sized bet, and the opponent never folds.
        let (s, hero) = spot(0, 1, "2c 7d", "Ah Kd Qs 9c 4h", 100, 0, 100);
        let model = OpponentModel::new(6);
        let profile = plain_profile(0.0); // no fold equity
        for seed in 0..20 {
            let a = decide(&s, hero, &profile, &model, seed);
            assert_eq!(a, Action::Fold, "hopeless hand should fold (seed {seed})");
        }
    }

    #[test]
    fn strong_equity_in_an_unbet_pot_value_bets() {
        // Top set on the flop: way ahead, unbet pot → value bet.
        let (s, hero) = spot(0, 1, "Ah Ad", "Ac 7d 2s", 0, 0, 100);
        let model = OpponentModel::new(6);
        for seed in 0..20 {
            let a = decide(&s, hero, &BotProfile::tag(), &model, seed);
            assert!(
                is_aggressive(a),
                "top set should value-bet, got {a:?} (seed {seed})"
            );
        }
    }

    #[test]
    fn a_draw_semi_bluff_raises_when_fold_equity_is_high() {
        // Open-ended straight draw (an underdog vs random hands) facing a
        // half-pot bet. With a high fold-equity prior the semi-bluff raise is
        // the better line — fold equity, not raw equity, makes it correct.
        let (s, hero) = spot(0, 1, "8h 7h", "Kc 9d 2s", 50, 0, 100);
        let model = OpponentModel::new(6);
        let profile = plain_profile(0.9); // opponents fold often
        for seed in 0..20 {
            let a = decide(&s, hero, &profile, &model, seed);
            assert!(
                matches!(a, Action::Raise { .. } | Action::AllIn),
                "high fold equity should make the draw semi-bluff raise, got {a:?} (seed {seed})"
            );
        }
    }

    #[test]
    fn the_same_draw_only_calls_when_there_is_no_fold_equity() {
        // Identical draw, but opponents never fold: with no fold equity the
        // raise is a losing play for an underdog, so the bot never raises.
        let (s, hero) = spot(0, 1, "8h 7h", "Kc 9d 2s", 50, 0, 100);
        let model = OpponentModel::new(6);
        let profile = plain_profile(0.0); // no fold equity
        for seed in 0..20 {
            let a = decide(&s, hero, &profile, &model, seed);
            assert!(
                !is_aggressive(a),
                "without fold equity the draw should not raise, got {a:?} (seed {seed})"
            );
        }
    }

    #[test]
    fn the_returned_action_is_always_legal() {
        let model = OpponentModel::new(6);
        let spots = [
            spot(0, 1, "Ah Kh", "Qh Jh Th 2c 3d", 0, 0, 100),
            spot(0, 1, "2c 7d", "Ah Kd Qs 9c 4h", 100, 0, 100),
            spot(0, 2, "Ah Ad", "Ac 7d 2s", 0, 0, 300),
            spot(0, 1, "Ah 3h", "Kh 7h 2c", 50, 0, 100),
            spot(0, 3, "Jc Jd", "2s 8h Td", 200, 0, 600),
        ];
        for (s, hero) in &spots {
            for profile in BotProfile::ALL {
                for seed in 0..10 {
                    let a = decide(s, *hero, &profile, &model, seed);
                    assert!(
                        apply(s.clone(), a).is_ok(),
                        "decide returned an illegal action {a:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn same_seed_yields_the_same_action() {
        // A genuinely close spot (marginal call) so the softmax mix is in play;
        // the same seed must still be reproducible.
        let (s, hero) = spot(0, 1, "Ah 3h", "Kh 7h 2c", 50, 0, 100);
        let model = OpponentModel::new(6);
        let profile = BotProfile::balanced();
        for seed in 0..30 {
            let a = decide(&s, hero, &profile, &model, seed);
            let b = decide(&s, hero, &profile, &model, seed);
            assert_eq!(a, b, "decide must be deterministic for a fixed seed");
        }
    }

    #[test]
    fn a_short_stack_shoves_instead_of_an_illegal_raise() {
        // Hero is too short to make a clean min-raise, but wants to get aggressive
        // with the nuts: the engine must offer an all-in, never an illegal raise.
        let (mut s, hero) = spot(0, 1, "Ah Kh", "Qh Jh Th 2c 3d", 100, 0, 400);
        s.stacks[hero.0] = 150; // call is 100; a min-raise to 200 is out of reach
        let model = OpponentModel::new(6);
        let a = decide(&s, hero, &BotProfile::tag(), &model, 1);
        assert_eq!(a, Action::AllIn, "nut hand too short to raise should shove");
        assert!(apply(s.clone(), a).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every parameter must sit in a sane range so the decision engine never
    /// sees a nonsensical personality.
    #[test]
    fn all_presets_have_parameters_in_valid_ranges() {
        for p in BotProfile::ALL {
            assert!(
                (0.0..=1.0).contains(&p.call_threshold),
                "{}: call_threshold {} out of range",
                p.name,
                p.call_threshold
            );
            assert!(
                (0.0..=1.0).contains(&p.value_threshold),
                "{}: value_threshold {} out of range",
                p.name,
                p.value_threshold
            );
            assert!(
                (0.0..=1.0).contains(&p.bluff_freq),
                "{}: bluff_freq {} out of range",
                p.name,
                p.bluff_freq
            );
            assert!(
                p.aggression > 0.0,
                "{}: aggression must be positive",
                p.name
            );
            assert!(
                p.mix_temperature > 0.0,
                "{}: mix_temperature must be positive",
                p.name
            );
            assert!(
                (0.0..=1.0).contains(&p.fold_equity_prior),
                "{}: fold_equity_prior {} out of range",
                p.name,
                p.fold_equity_prior
            );
        }
    }

    /// The rock is the tightest personality: nobody calls or value-bets looser
    /// hands than it requires, and it bluffs least.
    #[test]
    fn rock_is_the_tightest_personality() {
        let rock = BotProfile::rock();
        for p in [
            BotProfile::tag(),
            BotProfile::calling_station(),
            BotProfile::balanced(),
        ] {
            assert!(
                rock.call_threshold >= p.call_threshold,
                "rock should not call looser than {}",
                p.name
            );
            assert!(
                rock.value_threshold >= p.value_threshold,
                "rock should not value-bet looser than {}",
                p.name
            );
            assert!(
                rock.bluff_freq <= p.bluff_freq,
                "rock should not bluff more than {}",
                p.name
            );
        }
    }

    /// The calling station is the loosest caller and a reluctant raiser.
    #[test]
    fn calling_station_calls_loosest_and_rarely_bluffs() {
        let station = BotProfile::calling_station();
        for p in [
            BotProfile::tag(),
            BotProfile::balanced(),
            BotProfile::rock(),
        ] {
            assert!(
                station.call_threshold <= p.call_threshold,
                "calling station should call at least as loose as {}",
                p.name
            );
        }
        // It bluffs less than both aggressive profiles.
        assert!(station.bluff_freq < BotProfile::tag().bluff_freq);
        assert!(station.bluff_freq < BotProfile::balanced().bluff_freq);
    }

    /// The balanced/tricky profile bluffs the most and mixes the widest.
    #[test]
    fn balanced_bluffs_most_and_mixes_widest() {
        let balanced = BotProfile::balanced();
        for p in [
            BotProfile::tag(),
            BotProfile::calling_station(),
            BotProfile::rock(),
        ] {
            assert!(
                balanced.bluff_freq >= p.bluff_freq,
                "balanced should bluff at least as much as {}",
                p.name
            );
            assert!(
                balanced.mix_temperature >= p.mix_temperature,
                "balanced should mix at least as wide as {}",
                p.name
            );
        }
    }

    /// TAG sizes its bets up: it is the most aggressive sizer among the
    /// value-oriented profiles.
    #[test]
    fn tag_sizes_largest() {
        let tag = BotProfile::tag();
        assert!(tag.aggression >= BotProfile::calling_station().aggression);
        assert!(tag.aggression >= BotProfile::rock().aggression);
    }

    #[test]
    fn presets_have_distinct_names() {
        let names: Vec<&str> = BotProfile::ALL.iter().map(|p| p.name).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "preset names must be distinct");
    }
}
