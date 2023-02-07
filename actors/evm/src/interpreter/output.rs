use std::fmt::Debug;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Outcome {
    #[default]
    Return,
    Revert,
}

/// Output of EVM execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// Indicates the "outcome" of the execution.
    pub outcome: Outcome,
    /// The return data.
    pub return_data: Vec<u8>,
}
