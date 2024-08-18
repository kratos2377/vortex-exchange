use crate::{dex_state::DexState, errors::{DexError, VortexDexResult}, state::user::SpotPosition, user::{OrderStatus, User}, user_stats::UserStats, validate};

use super::constants::{MAX_OPEN_ORDERS, SPOT_IMF_PRECISION, SPOT_UTILIZATION_PRECISION_U32, SPOT_WEIGHT_PRECISION, THIRTEEN_DAY};





pub fn validate_spot_position(position: &SpotPosition) -> VortexDexResult {
    validate!(
        position.open_orders <= MAX_OPEN_ORDERS,
        DexError::InvalidSpotPositionDetected,
        "user spot={} position.open_orders={} is greater than MAX_OPEN_ORDERS={}",
        position.market_index,
        position.open_orders,
        MAX_OPEN_ORDERS,
    )?;

    validate!(
        position.open_bids >= 0,
        DexError::InvalidSpotPositionDetected,
        "user spot={} position.open_bids={} is less than 0",
        position.market_index,
        position.open_bids,
    )?;

    validate!(
        position.open_asks <= 0,
        DexError::InvalidSpotPositionDetected,
        "user spot={} position.open_asks={} is greater than 0",
        position.market_index,
        position.open_asks,
    )?;

    Ok(())
}

pub fn validate_user_deletion(
    user: &User,
    user_stats: &UserStats,
    state: &DexState,
    now: i64,
) -> VortexDexResult {
    validate!(
        !user_stats.is_referrer || user.sub_account_id != 0,
        DexError::UserCantBeDeleted,
        "user id 0 cant be deleted if user is a referrer"
    )?;

    validate!(
        !user.is_bankrupt(),
        DexError::UserCantBeDeleted,
        "user bankrupt"
    )?;

    validate!(
        !user.is_being_liquidated(),
        DexError::UserCantBeDeleted,
        "user being liquidated"
    )?;

    for perp_position in &user.perp_positions {
        validate!(
            perp_position.is_available(),
            DexError::UserCantBeDeleted,
            "user has perp position for market {}",
            perp_position.market_index
        )?;
    }

    for spot_position in &user.spot_positions {
        validate!(
            spot_position.is_available(),
            DexError::UserCantBeDeleted,
            "user has spot position for market {}",
            spot_position.market_index
        )?;
    }

    for order in &user.orders {
        validate!(
            order.status == OrderStatus::Init,
            DexError::UserCantBeDeleted,
            "user has an open order"
        )?;
    }

    if state.max_initialize_user_fee > 0 {
        let estimated_user_stats_age = user_stats.get_age_ts(now);
        if estimated_user_stats_age < THIRTEEN_DAY {
            validate!(
                user.idle,
                DexError::UserCantBeDeleted,
                "user is not idle with fresh user stats account creation ({} < {})",
                estimated_user_stats_age,
                THIRTEEN_DAY
            )?;
        }
    }

    Ok(())
}


pub fn validate_borrow_rate(
    optimal_utilization: u32,
    optimal_borrow_rate: u32,
    max_borrow_rate: u32,
    min_borrow_rate: u32,
) -> VortexDexResult {
    validate!(
        optimal_utilization <= SPOT_UTILIZATION_PRECISION_U32,
        DexError::InvalidSpotMarketInitialization,
        "For spot market, optimal_utilization must be < {}",
        SPOT_UTILIZATION_PRECISION_U32
    )?;

    validate!(
        optimal_borrow_rate <= max_borrow_rate,
        DexError::InvalidSpotMarketInitialization,
        "For spot market, optimal borrow rate ({}) must be <=  max borrow rate ({})",
        optimal_borrow_rate,
        max_borrow_rate
    )?;

    validate!(
        optimal_borrow_rate >= min_borrow_rate,
        DexError::InvalidSpotMarketInitialization,
        "For spot market, optimal borrow rate ({}) must be >= min borrow rate ({})",
        optimal_borrow_rate,
        min_borrow_rate
    )?;

    Ok(())
}


pub fn validate_margin_weights(
    spot_market_index: u16,
    initial_asset_weight: u32,
    maintenance_asset_weight: u32,
    initial_liability_weight: u32,
    maintenance_liability_weight: u32,
    imf_factor: u32,
) -> VortexDexResult {
    if spot_market_index == 0 {
        validate!(
            initial_asset_weight == SPOT_WEIGHT_PRECISION,
            DexError::InvalidSpotMarketInitialization,
            "For quote asset spot market, initial asset weight must be {}",
            SPOT_WEIGHT_PRECISION
        )?;

        validate!(
            maintenance_asset_weight == SPOT_WEIGHT_PRECISION,
            DexError::InvalidSpotMarketInitialization,
            "For quote asset spot market, maintenance asset weight must be {}",
            SPOT_WEIGHT_PRECISION
        )?;

        validate!(
            initial_liability_weight == SPOT_WEIGHT_PRECISION,
            DexError::InvalidSpotMarketInitialization,
            "For quote asset spot market, initial liability weight must be {}",
            SPOT_WEIGHT_PRECISION
        )?;

        validate!(
            maintenance_liability_weight == SPOT_WEIGHT_PRECISION,
            DexError::InvalidSpotMarketInitialization,
            "For quote asset spot market, maintenance liability weight must be {}",
            SPOT_WEIGHT_PRECISION
        )?;
    } else {
        validate!(
            initial_asset_weight < SPOT_WEIGHT_PRECISION,
            DexError::InvalidSpotMarketInitialization,
            "Initial asset weight must be less than {}",
            SPOT_WEIGHT_PRECISION
        )?;

        validate!(
            initial_asset_weight <= maintenance_asset_weight
                && maintenance_asset_weight > 0
                && maintenance_asset_weight < SPOT_WEIGHT_PRECISION,
            DexError::InvalidSpotMarketInitialization,
            "Maintenance asset weight must be between 0 {}",
            SPOT_WEIGHT_PRECISION
        )?;

        validate!(
            initial_liability_weight > SPOT_WEIGHT_PRECISION,
            DexError::InvalidSpotMarketInitialization,
            "Initial liability weight must be greater than {}",
            SPOT_WEIGHT_PRECISION
        )?;

        validate!(
            initial_liability_weight >= maintenance_liability_weight
                && maintenance_liability_weight > SPOT_WEIGHT_PRECISION,
            DexError::InvalidSpotMarketInitialization,
            "Maintenance liability weight must be greater than {}",
            SPOT_WEIGHT_PRECISION
        )?;
    }

    validate!(
        imf_factor < SPOT_IMF_PRECISION,
        DexError::InvalidSpotMarketInitialization,
        "imf_factor={} must be less than SPOT_IMF_PRECISION={}",
        imf_factor,
        SPOT_IMF_PRECISION,
    )?;

    Ok(())
}
