use borsh::{BorshDeserialize, BorshSerialize};

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq, Default)]
pub enum PositionDirection {
    #[default]
    Long,
    Short,
}

impl PositionDirection {
    pub fn opposite(&self) -> Self {
        match self {
            PositionDirection::Long => PositionDirection::Short,
            PositionDirection::Short => PositionDirection::Long,
        }
    }
}