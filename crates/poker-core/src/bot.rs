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
