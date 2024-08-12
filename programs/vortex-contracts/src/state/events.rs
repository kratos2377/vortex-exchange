use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};

#[event]
pub struct NewUserAccountRecord {
    pub ts: i64,
    pub user_authority: Pubkey,
    pub user: Pubkey,
    pub name: [u8; 32],
}


#[event]
pub struct SpotInterestRecord {
    pub ts: i64,
    pub market_index: u16,
    /// precision: SPOT_BALANCE_PRECISION
    pub deposit_balance: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub cumulative_deposit_interest: u128,
    /// precision: SPOT_BALANCE_PRECISION
    pub borrow_balance: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub cumulative_borrow_interest: u128,
    /// precision: PERCENTAGE_PRECISION
    pub optimal_utilization: u32,
    /// precision: PERCENTAGE_PRECISION
    pub optimal_borrow_rate: u32,
    /// precision: PERCENTAGE_PRECISION
    pub max_borrow_rate: u32,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
pub enum DepositDirection {
    #[default]
    Deposit,
    Withdraw,
}

#[event]
pub struct DepositRecord {
    /// unix_timestamp of action
    pub ts: i64,
    pub user_authority: Pubkey,
    /// user account public key
    pub user: Pubkey,
    pub direction: DepositDirection,
    pub deposit_record_id: u64,
    /// precision: token mint precision
    pub amount: u64,
    /// spot market index
    pub market_index: u16,
    /// precision: PRICE_PRECISION
    pub oracle_price: i64,
    /// precision: SPOT_BALANCE_PRECISION
    pub market_deposit_balance: u128,
    /// precision: SPOT_BALANCE_PRECISION
    pub market_withdraw_balance: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub market_cumulative_deposit_interest: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub market_cumulative_borrow_interest: u128,
    /// precision: QUOTE_PRECISION
    pub total_deposits_after: u64,
    /// precision: QUOTE_PRECISION
    pub total_withdraws_after: u64,
    pub explanation: DepositExplanation,
    pub transfer_user: Option<Pubkey>,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
pub enum DepositExplanation {
    #[default]
    None,
    Transfer,
    Borrow,
    RepayBorrow,
}
