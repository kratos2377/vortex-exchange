use borsh::{BorshDeserialize, BorshSerialize};



#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Debug, Default)]
pub enum SpotBalanceType {
    #[default]
    Deposit,
    Borrow,
}