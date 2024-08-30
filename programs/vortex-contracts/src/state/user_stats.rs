use std::cmp::max;

use anchor_lang::prelude::*;

use crate::{casting::Cast, errors::VortexDexResult, safe_methods::SafeMath, utils::{constants::{EPOCH_DURATION, FUEL_START_TS, QUOTE_PRECISION_U64, THIRTY_DAY}, stats_utils::calculate_rolling_sum}};

use super::{user::User, user_fees::UserFees};



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

    pub fn get_fuel_bonus_numerator(
        self,
        last_fuel_bonus_update_ts: i64,
        now: i64,
    ) -> VortexDexResult<i64> {
        if last_fuel_bonus_update_ts != 0 {
            let since_last = now.safe_sub(last_fuel_bonus_update_ts)?;
            return Ok(since_last);
        }

        Ok(0)
    }

    pub fn update_fuel_bonus_trade(&mut self, fuel_taker: u32, fuel_maker: u32) -> VortexDexResult {
        self.fuel_taker = self.fuel_taker.saturating_add(fuel_taker);
        self.fuel_maker = self.fuel_maker.saturating_add(fuel_maker);

        Ok(())
    }

    pub fn update_fuel_bonus(
        &mut self,
        user: &mut User,
        fuel_deposits: u32,
        fuel_borrows: u32,
        fuel_positions: u32,
        now: i64,
    ) -> VortexDexResult {
        if  now > FUEL_START_TS {
            self.fuel_deposits = self.fuel_deposits.saturating_add(fuel_deposits);
            self.fuel_borrows = self.fuel_borrows.saturating_add(fuel_borrows);
            self.fuel_positions = self.fuel_positions.saturating_add(fuel_positions);

     
        }

        Ok(())
    }

    pub fn update_fuel_maker_bonus(
        &mut self,
        fuel_boost: u8,
        quote_asset_amount: u64,
    ) -> VortexDexResult {
        if fuel_boost > 0 {
            self.fuel_maker = self.fuel_maker.saturating_add(
                fuel_boost
                    .cast::<u64>()?
                    .saturating_mul(quote_asset_amount / QUOTE_PRECISION_U64)
                    .cast::<u32>()
                    .unwrap_or(u32::MAX),
            ); // todo of ratio
        }
        Ok(())
    }

    pub fn update_fuel_taker_bonus(
        &mut self,
        fuel_boost: u8,
        quote_asset_amount: u64,
    ) -> VortexDexResult {
        if fuel_boost > 0 {
            self.fuel_taker = self.fuel_taker.saturating_add(
                fuel_boost
                    .cast::<u64>()?
                    .saturating_mul(quote_asset_amount / QUOTE_PRECISION_U64)
                    .cast::<u32>()
                    .unwrap_or(u32::MAX),
            ); // todo of ratio
        }
        Ok(())
    }

    pub fn update_maker_volume_30d(
        &mut self,
        fuel_boost: u8,
        quote_asset_amount: u64,
        now: i64,
    ) -> VortexDexResult {
        let since_last = max(1_i64, now.safe_sub(self.last_maker_volume_30d_ts)?);

        self.update_fuel_maker_bonus(fuel_boost, quote_asset_amount)?;

        self.maker_volume_30d = calculate_rolling_sum(
            self.maker_volume_30d,
            quote_asset_amount,
            since_last,
            THIRTY_DAY,
        )?;
        self.last_maker_volume_30d_ts = now;

        Ok(())
    }

    pub fn update_taker_volume_30d(
        &mut self,
        fuel_boost: u8,
        quote_asset_amount: u64,
        now: i64,
    ) -> VortexDexResult {
        let since_last = max(1_i64, now.safe_sub(self.last_taker_volume_30d_ts)?);

        self.update_fuel_taker_bonus(fuel_boost, quote_asset_amount)?;

        self.taker_volume_30d = calculate_rolling_sum(
            self.taker_volume_30d,
            quote_asset_amount,
            since_last,
            THIRTY_DAY,
        )?;
        self.last_taker_volume_30d_ts = now;

        Ok(())
    }

    pub fn update_filler_volume(&mut self, quote_asset_amount: u64, now: i64) -> VortexDexResult {
        let since_last = max(1_i64, now.safe_sub(self.last_filler_volume_30d_ts)?);

        self.filler_volume_30d = calculate_rolling_sum(
            self.filler_volume_30d,
            quote_asset_amount,
            since_last,
            THIRTY_DAY,
        )?;

        self.last_filler_volume_30d_ts = now;

        Ok(())
    }

    pub fn increment_total_fees(&mut self, fee: u64) -> VortexDexResult {
        self.fees.total_fee_paid = self.fees.total_fee_paid.safe_add(fee)?;

        Ok(())
    }

    pub fn increment_total_rebate(&mut self, fee: u64) -> VortexDexResult {
        self.fees.total_fee_rebate = self.fees.total_fee_rebate.safe_add(fee)?;

        Ok(())
    }

    pub fn increment_total_referrer_reward(&mut self, reward: u64, now: i64) -> VortexDexResult {
        self.fees.total_referrer_reward = self.fees.total_referrer_reward.safe_add(reward)?;

        self.fees.current_epoch_referrer_reward =
            self.fees.current_epoch_referrer_reward.safe_add(reward)?;

        if now > self.next_epoch_ts {
            let n_epoch_durations = now
                .safe_sub(self.next_epoch_ts)?
                .safe_div(EPOCH_DURATION)?
                .safe_add(1)?;

            self.next_epoch_ts = self
                .next_epoch_ts
                .safe_add(EPOCH_DURATION.safe_mul(n_epoch_durations)?)?;

            self.fees.current_epoch_referrer_reward = 0;
        }

        Ok(())
    }

    pub fn increment_total_referee_discount(&mut self, discount: u64) -> VortexDexResult {
        self.fees.total_referee_discount = self.fees.total_referee_discount.safe_add(discount)?;

        Ok(())
    }

    pub fn has_referrer(&self) -> bool {
        false
    }

    pub fn get_total_30d_volume(&self) -> VortexDexResult<u64> {
        self.taker_volume_30d.safe_add(self.maker_volume_30d)
    }

    pub fn get_age_ts(&self, now: i64) -> i64 {
        // upper bound of age of the user stats account
        let min_action_ts: i64 = self
            .last_filler_volume_30d_ts
            .min(self.last_maker_volume_30d_ts)
            .min(self.last_taker_volume_30d_ts);
        now.saturating_sub(min_action_ts).max(0)
    }

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