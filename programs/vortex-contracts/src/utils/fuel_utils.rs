use std::cmp::{max, min};

use num_integer::Roots;

use crate::{casting::Cast, errors::VortexDexResult, safe_methods::SafeMath, state::{dex_state::{FeeStructure, FeeTier, OrderFillerRewardStructure}, spot_market::SpotMarket, user::MarketType, user_stats::UserStats}, utils::constants::{FUEL_WINDOW_U128, QUOTE_PRECISION, QUOTE_PRECISION_U64}};

use super::{constants::{FEE_ADJUSTMENT_MAX, TEN_BPS}, fees_utils::determine_user_fee_tier};

pub fn calculate_spot_fuel_bonus(
    spot_market: &SpotMarket,
    signed_token_value: i128,
    fuel_bonus_numerator: i64,
) -> VortexDexResult<u64> {
    let result: u64 = if signed_token_value.unsigned_abs() < QUOTE_PRECISION {
        0_u64
    } else if signed_token_value > 0 {
        signed_token_value
            .unsigned_abs()
            .safe_mul(fuel_bonus_numerator.cast()?)?
            .safe_mul(spot_market.fuel_boost_deposits.cast()?)?
            .safe_div(FUEL_WINDOW_U128)?
            .cast::<u64>()?
            / (QUOTE_PRECISION_U64 / 10)
    } else {
        signed_token_value
            .unsigned_abs()
            .safe_mul(fuel_bonus_numerator.cast()?)?
            .safe_mul(spot_market.fuel_boost_borrows.cast()?)?
            .safe_div(FUEL_WINDOW_U128)?
            .cast::<u64>()?
            / (QUOTE_PRECISION_U64 / 10)
    };

    Ok(result)
}

pub fn calculate_insurance_fuel_bonus(
    spot_market: &SpotMarket,
    stake_amount: u64,
    stake_amount_delta: i64,
    fuel_bonus_numerator: u32,
) -> VortexDexResult<u64> {
    Ok(stake_amount
        .saturating_sub(stake_amount_delta.unsigned_abs())
        .cast::<u128>()?
        .safe_mul(fuel_bonus_numerator.cast()?)?
        .safe_mul(spot_market.fuel_boost_insurance.cast()?)?
        .safe_div(FUEL_WINDOW_U128)?
        .cast::<u64>()?
        / (QUOTE_PRECISION_U64 / 10))
}


fn calculate_taker_fee(
    quote_asset_amount: u64,
    fee_tier: &FeeTier,
    fee_adjustment: i16,
) -> VortexDexResult<u64> {
    let mut taker_fee = quote_asset_amount
        .cast::<u128>()?
        .safe_mul(fee_tier.fee_numerator.cast::<u128>()?)?
        .safe_div_ceil(fee_tier.fee_denominator.cast::<u128>()?)?
        .cast::<u64>()?;

    if fee_adjustment < 0 {
        taker_fee = taker_fee.saturating_sub(
            taker_fee
                .safe_mul(fee_adjustment.unsigned_abs().cast()?)?
                .safe_div(FEE_ADJUSTMENT_MAX)?,
        );
    } else if fee_adjustment > 0 {
        taker_fee = taker_fee.saturating_add(
            taker_fee
                .safe_mul(fee_adjustment.cast()?)?
                .safe_div_ceil(FEE_ADJUSTMENT_MAX)?,
        );
    }

    Ok(taker_fee)
}


fn calculate_filler_reward(
    fee: u64,
    order_slot: u64,
    clock_slot: u64,
    multiplier: u64,
    filler_reward_structure: &OrderFillerRewardStructure,
) -> VortexDexResult<u64> {
    // incentivize keepers to prioritize filling older orders (rather than just largest orders)
    // for sufficiently small-sized order, reward based on fraction of fee paid

    let size_filler_reward = fee
        .safe_mul(filler_reward_structure.reward_numerator as u64)?
        .safe_div(filler_reward_structure.reward_denominator as u64)?;

    let multiplier_precision = TEN_BPS.cast::<u128>()?;

    let min_time_filler_reward = filler_reward_structure
        .time_based_reward_lower_bound
        .safe_mul(
            multiplier
                .cast::<u128>()?
                .max(multiplier_precision)
                .min(multiplier_precision * 100),
        )?
        .safe_div(multiplier_precision)?;

    let slots_since_order = max(1, clock_slot.safe_sub(order_slot)?.cast::<u128>()?);
    let time_filler_reward = slots_since_order
        .safe_mul(100_000_000)? // 1e8
        .nth_root(4)
        .safe_mul(min_time_filler_reward)?
        .safe_div(100)? // 1e2 = sqrt(sqrt(1e8))
        .cast::<u64>()?;

    // lesser of size-based and time-based reward
    let fee = min(size_filler_reward, time_filler_reward);

    Ok(fee)
}

pub struct ExternalFillFees {
    pub user_fee: u64,
    pub fee_to_market: u64,
    pub fee_pool_delta: i64,
    pub filler_reward: u64,
}

pub fn calculate_fee_for_fulfillment_with_external_market(
    user_stats: &UserStats,
    quote_asset_amount: u64,
    fee_structure: &FeeStructure,
    order_slot: u64,
    clock_slot: u64,
    reward_filler: bool,
    external_market_fee: u64,
    unsettled_referrer_rebate: u64,
    fee_pool_amount: u64,
    fee_adjustment: i16,
) -> VortexDexResult<ExternalFillFees> {
    let taker_fee_tier = determine_user_fee_tier(user_stats, fee_structure, &MarketType::Spot)?;

    let fee = calculate_taker_fee(quote_asset_amount, taker_fee_tier, fee_adjustment)?;

    let fee_plus_referrer_rebate = external_market_fee.safe_add(unsettled_referrer_rebate)?;

    let user_fee = fee.max(fee_plus_referrer_rebate);

    let filler_reward = if reward_filler {
        let immediately_available_fee = user_fee.safe_sub(fee_plus_referrer_rebate)?;

        let eventual_available_fee = user_fee.safe_sub(external_market_fee)?;

        // can only pay the filler immediately if
        // 1. there are fees already in the fee pool
        // 2. the user_fee is greater than the serum_fee_plus_referrer_rebate
        let available_fee =
            eventual_available_fee.min(fee_pool_amount.max(immediately_available_fee));

        calculate_filler_reward(
            quote_asset_amount,
            order_slot,
            clock_slot,
            0,
            &fee_structure.filler_reward_structure,
        )?
        .min(available_fee)
    } else {
        0
    };

    let fee_to_market = user_fee
        .safe_sub(external_market_fee)?
        .safe_sub(filler_reward)?;

    let fee_pool_delta = fee_to_market
        .cast::<i64>()?
        .safe_sub(unsettled_referrer_rebate.cast()?)?;

    Ok(ExternalFillFees {
        user_fee,
        fee_to_market,
        filler_reward,
        fee_pool_delta,
    })
}
