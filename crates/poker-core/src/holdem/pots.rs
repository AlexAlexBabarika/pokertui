use super::types::PlayerId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pot {
    pub amount: u64,
    pub eligible: Vec<PlayerId>,
}
