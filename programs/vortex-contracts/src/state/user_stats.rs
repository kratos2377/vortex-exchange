use anchor_lang::prelude::*;

use super::user_fees::UserFees;



#[account(zero_copy(unsafe))]
#[derive(Eq, PartialEq, Debug)]
#[repr(C)]
pub struct UserStats {
    pub authority: Pubkey,
    pub fees: UserFees,

    /// The timestamp of the next epoch
    /// Epoch is used to limit referrer rewards earned in single epoch
    pub next_epoch_ts: i64,

    /// Rolling 30day maker volume for user
    /// precision: QUOTE_PRECISION
    pub maker_volume_30d: u64,
    /// Rolling 30day taker volume for user
    /// precision: QUOTE_PRECISION
    pub taker_volume_30d: u64,
    /// Rolling 30day filler volume for user
    /// precision: QUOTE_PRECISION
    pub filler_volume_30d: u64,
    /// last time the maker volume was updated
    pub last_maker_volume_30d_ts: i64,
    /// last time the taker volume was updated
    pub last_taker_volume_30d_ts: i64,
    /// last time the filler volume was updated
    pub last_filler_volume_30d_ts: i64,

    /// The amount of tokens staked in the quote spot markets if
    pub if_staked_quote_asset_amount: u64,
    /// The current number of sub accounts
    pub number_of_sub_accounts: u16,
    /// The number of sub accounts created. Can be greater than the number of sub accounts if user
    /// has deleted sub accounts
    pub number_of_sub_accounts_created: u16,
    /// Whether the user is a referrer. Sub account 0 can not be deleted if user is a referrer
    pub is_referrer: bool,
    pub disable_update_perp_bid_ask_twap: bool,
    pub padding1: [u8; 2],
    /// accumulated fuel for token amounts of insurance
    pub fuel_insurance: u32,
    /// accumulated fuel for notional of deposits
    pub fuel_deposits: u32,
    /// accumulate fuel bonus for notional of borrows
    pub fuel_borrows: u32,
    /// accumulated fuel for perp open interest
    pub fuel_positions: u32,
    /// accumulate fuel bonus for taker volume
    pub fuel_taker: u32,
    /// accumulate fuel bonus for maker volume
    pub fuel_maker: u32,

    /// The amount of tokens staked in the governance spot markets if
    pub if_staked_gov_token_amount: u64,

    /// last unix ts user stats data was used to update if fuel (u32 to save space)
    pub last_fuel_if_bonus_update_ts: u32,

    pub padding: [u8; 12],
}

impl UserStats {
    pub const SIZE: usize = 240;
}

impl Default for UserStats {
    fn default() -> Self {
        UserStats {
            authority: Pubkey::default(),
            fees: UserFees::default(),
            next_epoch_ts: 0,
            maker_volume_30d: 0,
            taker_volume_30d: 0,
            filler_volume_30d: 0,
            last_maker_volume_30d_ts: 0,
            last_taker_volume_30d_ts: 0,
            last_filler_volume_30d_ts: 0,
            if_staked_quote_asset_amount: 0,
            number_of_sub_accounts: 0,
            number_of_sub_accounts_created: 0,
            is_referrer: false,
            disable_update_perp_bid_ask_twap: true,
            padding1: [0; 2],
            fuel_insurance: 0,
            fuel_deposits: 0,
            fuel_borrows: 0,
            fuel_taker: 0,
            fuel_maker: 0,
            fuel_positions: 0,
            if_staked_gov_token_amount: 0,
            last_fuel_if_bonus_update_ts: 0,
            padding: [0; 12],
        }
    }
}