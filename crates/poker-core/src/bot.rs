use crate::holdem::{Action, LogEntry, LogKind, PlayerId};

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
