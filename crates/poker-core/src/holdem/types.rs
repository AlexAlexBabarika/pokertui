#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct PlayerId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    Preflop,
    Flop,
    Turn,
    River,
    Showdown,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Action {
    Fold,
    Check,
    Call,
    Bet { to: u64 },
    Raise { to: u64 },
    AllIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandConfig {
    pub num_players: usize,
    pub small_blind: u64,
    pub big_blind: u64,
    pub dealer: PlayerId,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    NotYourTurn,
    IllegalAction(&'static str),
    HandComplete,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_have_distinct_identity() {
        assert_ne!(Phase::Preflop, Phase::Flop);
        assert_ne!(Phase::Showdown, Phase::Complete);
    }

    #[test]
    fn action_equality_uses_amount() {
        assert_eq!(Action::Bet { to: 100 }, Action::Bet { to: 100 });
        assert_ne!(Action::Bet { to: 100 }, Action::Bet { to: 200 });
    }

    #[test]
    fn action_serde_round_trips_as_json() {
        let raise = Action::Raise { to: 300 };
        let json = serde_json::to_string(&raise).expect("serialize");
        let back: Action = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, raise);
    }
}
