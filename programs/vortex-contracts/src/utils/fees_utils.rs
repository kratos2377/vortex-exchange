use std::cmp::{max, min};

use crate::msg;
use num_integer::Roots;

use crate::{casting::Cast, errors::{DexError, VortexDexResult}, safe_methods::SafeMath, state::{dex_state::{FeeStructure, FeeTier, OrderFillerRewardStructure}, user::MarketType, user_stats::UserStats}, validate};

use super::{constants::{FEE_ADJUSTMENT_MAX, FEE_DENOMINATOR, FEE_PERCENTAGE_DENOMINATOR, OPEN_ORDER_MARGIN_REQUIREMENT, TEN_BPS}, fuel_utils::ExternalFillFees, helper_utils::get_proportion_u128};


pub struct FillFees {
    pub user_fee: u64,
    pub maker_rebate: u64,
    pub fee_to_market: i64,
    pub fee_to_market_for_lp: i64,
    pub filler_reward: u64,
    pub referrer_reward: u64,
    pub referee_discount: u64,
}


pub fn calculate_fee_for_fulfillment_with_match(
    taker_stats: &UserStats,
    maker_stats: &Option<&mut UserStats>,
    quote_asset_amount: u64,
    fee_structure: &FeeStructure,
    order_slot: u64,
    clock_slot: u64,
    filler_multiplier: u64,
    reward_referrer: bool,
    referrer_stats: &Option<&mut UserStats>,
    market_type: &MarketType,
    fee_adjustment: i16,
) -> VortexDexResult<FillFees> {
    let taker_fee_tier = determine_user_fee_tier(taker_stats, fee_structure, market_type)?;
    let maker_fee_tier = if let Some(maker_stats) = maker_stats {
        determine_user_fee_tier(maker_stats, fee_structure, market_type)?
    } else {
        determine_user_fee_tier(taker_stats, fee_structure, market_type)?
    };

    let taker_fee = calculate_taker_fee(quote_asset_amount, taker_fee_tier, fee_adjustment)?;

    let (taker_fee, referee_discount, referrer_reward) = if reward_referrer {
        calculate_referee_fee_and_referrer_reward(
            taker_fee,
            taker_fee_tier,
            fee_structure.referrer_reward_epoch_upper_bound,
            referrer_stats,
        )?
    } else {
        (taker_fee, 0, 0)
    };

    let maker_rebate = calculate_maker_rebate(quote_asset_amount, maker_fee_tier, fee_adjustment)?;

    let filler_reward = if filler_multiplier == 0 {
        0_u64
    } else {
        calculate_filler_reward(
            taker_fee,
            order_slot,
            clock_slot,
            filler_multiplier,
            &fee_structure.filler_reward_structure,
        )?
    };

    // must be non-negative
    let fee_to_market = taker_fee
        .safe_sub(filler_reward)?
        .safe_sub(referrer_reward)?
        .safe_sub(maker_rebate)?
        .cast::<i64>()?;

    Ok(FillFees {
        user_fee: taker_fee,
        maker_rebate,
        fee_to_market,
        filler_reward,
        referrer_reward,
        fee_to_market_for_lp: 0,
        referee_discount,
    })
}

pub fn determine_user_fee_tier<'a>(
    user_stats: &UserStats,
    fee_structure: &'a FeeStructure,
    market_type: &MarketType,
) -> VortexDexResult<&'a FeeTier> {
    match market_type {
        MarketType::Spot => determine_spot_fee_tier(user_stats, fee_structure),
    }
}

fn calculate_referee_fee_and_referrer_reward(
    fee: u64,
    fee_tier: &FeeTier,
    referrer_reward_epoch_upper_bound: u64,
    referrer_stats: &Option<&mut UserStats>,
) -> VortexDexResult<(u64, u64, u64)> {
    let referee_discount = get_proportion_u128(
        fee as u128,
        fee_tier.referee_fee_numerator as u128,
        fee_tier.referee_fee_denominator as u128,
    )?
    .cast::<u64>()?;

    let max_referrer_reward_from_fee = get_proportion_u128(
        fee as u128,
        fee_tier.referrer_reward_numerator as u128,
        fee_tier.referrer_reward_denominator as u128,
    )?
    .cast::<u64>()?;

    let referee_fee = fee.safe_sub(referee_discount)?;

    let referrer_reward = match referrer_stats {
        Some(referrer_stats) => {
            let max_referrer_reward_in_epoch = referrer_reward_epoch_upper_bound
                .saturating_sub(referrer_stats.fees.current_epoch_referrer_reward);
            max_referrer_reward_from_fee.min(max_referrer_reward_in_epoch)
        }
        None => max_referrer_reward_from_fee,
    };
    Ok((referee_fee, referee_discount, referrer_reward))
}

fn determine_spot_fee_tier<'a>(
    _user_stats: &UserStats,
    fee_structure: &'a FeeStructure,
) -> VortexDexResult<&'a FeeTier> {
    Ok(&fee_structure.fee_tiers[0])
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

fn calculate_maker_rebate(
    quote_asset_amount: u64,
    fee_tier: &FeeTier,
    fee_adjustment: i16,
) -> VortexDexResult<u64> {
    let mut maker_fee = quote_asset_amount
        .cast::<u128>()?
        .safe_mul(fee_tier.maker_rebate_numerator as u128)?
        .safe_div(fee_tier.maker_rebate_denominator as u128)?
        .cast::<u64>()?;

    if fee_adjustment < 0 {
        maker_fee = maker_fee.saturating_sub(
            maker_fee
                .safe_mul(fee_adjustment.unsigned_abs().cast()?)?
                .safe_div_ceil(FEE_ADJUSTMENT_MAX)?,
        );
    } else if fee_adjustment > 0 {
        maker_fee = maker_fee.saturating_add(
            maker_fee
                .safe_mul(fee_adjustment.cast()?)?
                .safe_div(FEE_ADJUSTMENT_MAX)?,
        );
    }

    Ok(maker_fee)
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


pub fn validate_fee_structure(fee_structure: &FeeStructure) -> VortexDexResult {
    for (i, fee_tier) in fee_structure.fee_tiers.iter().enumerate() {
        validate_fee_tier(
            i,
            fee_tier,
            fee_structure.filler_reward_structure.reward_numerator,
        )?;
    }

    let is_filler_reward_valid = fee_structure.filler_reward_structure.reward_numerator <= 20
        && fee_structure.filler_reward_structure.reward_denominator == FEE_PERCENTAGE_DENOMINATOR; // <= 20%

    validate!(
        is_filler_reward_valid,
        DexError::InvalidFeeStructure,
        "invalid filler reward numerator ({}) or denominator  ({})",
        fee_structure.filler_reward_structure.reward_numerator,
        fee_structure.filler_reward_structure.reward_denominator
    )?;

    validate!(
        fee_structure.flat_filler_fee < OPEN_ORDER_MARGIN_REQUIREMENT as u64 / 2,
        DexError::InvalidFeeStructure,
        "invalid flat filler fee {}",
        fee_structure.flat_filler_fee
    )?;

    Ok(())
}


pub fn validate_fee_tier(
    fee_tier_index: usize,
    fee_tier: &FeeTier,
    filler_reward_numerator: u32,
) -> VortexDexResult {
    let fee_valid = fee_tier.fee_numerator <= 100 && fee_tier.fee_denominator == FEE_DENOMINATOR; // <= 10bps

    validate!(
        fee_valid,
        DexError::InvalidFeeStructure,
        "invalid fee numerator ({}) or denominator  ({})",
        fee_tier.fee_numerator,
        fee_tier.fee_denominator
    )?;

    let maker_rebate_valid = fee_tier.maker_rebate_numerator <= 30
        && fee_tier.maker_rebate_denominator == FEE_DENOMINATOR; // <= 3bps

    validate!(
        maker_rebate_valid,
        DexError::InvalidFeeStructure,
        "invalid maker rebate numerator ({}) or denominator  ({})",
        fee_tier.maker_rebate_numerator,
        fee_tier.maker_rebate_denominator
    )?;

    let referee_discount_valid = fee_tier.referee_fee_numerator <= 20
        && fee_tier.referee_fee_denominator == FEE_PERCENTAGE_DENOMINATOR; // <= 20%

    validate!(
        referee_discount_valid,
        DexError::InvalidFeeStructure,
        "invalid referee discount numerator ({}) or denominator  ({})",
        fee_tier.referee_fee_numerator,
        fee_tier.referee_fee_denominator
    )?;

    let referrer_reward_valid = fee_tier.referrer_reward_numerator <= 20
        && fee_tier.referrer_reward_denominator == FEE_PERCENTAGE_DENOMINATOR; // <= 20%

    validate!(
        referrer_reward_valid,
        DexError::InvalidFeeStructure,
        "invalid referrer reward numerator ({}) or denominator  ({})",
        fee_tier.referrer_reward_numerator,
        fee_tier.referrer_reward_denominator
    )?;

    let taker_fee = fee_tier.fee_numerator * (100 - fee_tier.referee_fee_numerator) / 100;
    let fee_to_market = taker_fee
        - fee_tier.maker_rebate_numerator
        - taker_fee * (fee_tier.referrer_reward_numerator + filler_reward_numerator) / 100;

    validate!(
        fee_to_market <= fee_tier.fee_numerator,
        DexError::InvalidFeeStructure,
        "invalid fee to market ({}) for index ({})",
        fee_tier.referrer_reward_numerator,
        fee_tier_index,
    )?;

    Ok(())
}
