use solana_program::msg;

use crate::{controllers::spot_balance::update_revenue_pool_balances, errors::{DexError, VortexDexResult}, safe_decrement, safe_increment, state::{operations::SpotOperation, position::PositionDirection, spot_market::{AssetTier, MarketStatus, SpotBalanceType, SpotMarket}, user::{SpotPosition, User}, user_stats::UserStats}, utils::{constants::QUOTE_PRECISION, spot_market_utils::check_withdraw_limits}, validate};

use super::spot_balance::update_spot_balances;



pub fn increase_spot_open_bids_and_asks(
    spot_position: &mut SpotPosition,
    direction: &PositionDirection,
    base_asset_amount_unfilled: u64,
) -> VortexDexResult {
    match direction {
        PositionDirection::Long => {
            spot_position.open_bids = spot_position
                .open_bids
                .safe_add(base_asset_amount_unfilled.cast()?)?;
        }
        PositionDirection::Short => {
            spot_position.open_asks = spot_position
                .open_asks
                .safe_sub(base_asset_amount_unfilled.cast()?)?;
        }
    }

    Ok(())
}

pub fn decrease_spot_open_bids_and_asks(
    spot_position: &mut SpotPosition,
    direction: &PositionDirection,
    base_asset_amount_unfilled: u64,
) -> VortexDexResult {
    match direction {
        PositionDirection::Long => {
            spot_position.open_bids = spot_position
                .open_bids
                .safe_sub(base_asset_amount_unfilled.cast()?)?;
        }
        PositionDirection::Short => {
            spot_position.open_asks = spot_position
                .open_asks
                .safe_add(base_asset_amount_unfilled.cast()?)?;
        }
    }

    Ok(())
}

pub fn update_spot_balances_and_cumulative_deposits(
    token_amount: u128,
    update_direction: &SpotBalanceType,
    spot_market: &mut SpotMarket,
    spot_position: &mut SpotPosition,
    is_leaving_drift: bool,
    cumulative_deposit_delta: Option<u128>,
) -> VortexDexResult {
    update_spot_balances(
        token_amount,
        update_direction,
        spot_market,
        spot_position,
        is_leaving_drift,
    )?;

    let cumulative_deposit_delta = cumulative_deposit_delta.unwrap_or(token_amount);
    match update_direction {
        SpotBalanceType::Deposit => {
            safe_increment!(
                spot_position.cumulative_deposits,
                cumulative_deposit_delta.cast()?
            )
        }
        SpotBalanceType::Borrow => {
            safe_decrement!(
                spot_position.cumulative_deposits,
                cumulative_deposit_delta.cast()?
            )
        }
    }

    Ok(())
}

pub fn update_spot_balances_and_cumulative_deposits_with_limits(
    token_amount: u128,
    update_direction: &SpotBalanceType,
    spot_market: &mut SpotMarket,
    user: &mut User,
) -> VortexDexResult {
    let spot_position_index = user.get_spot_position_index(spot_market.market_index)?;

    update_spot_balances_and_cumulative_deposits(
        token_amount,
        update_direction,
        spot_market,
        &mut user.spot_positions[spot_position_index],
        true,
        None,
    )?;

    let valid_withdraw = check_withdraw_limits(spot_market, Some(user), Some(token_amount))?;

    validate!(
        valid_withdraw,
        DexError::DailyWithdrawLimit,
        "Spot Market {} has hit daily withdraw limit. Attempted withdraw amount of {} by {}",
        spot_market.market_index,
        token_amount,
        user.authority
    )?;

    validate!(
        matches!(
            spot_market.status,
            MarketStatus::Active | MarketStatus::ReduceOnly | MarketStatus::Settlement
        ),
        DexError::MarketWithdrawPaused,
        "Spot Market {} withdraws are currently paused, market not active or in settlement",
        spot_market.market_index
    )?;

    validate!(
        !spot_market.is_operation_paused(SpotOperation::Withdraw),
        DexError::MarketWithdrawPaused,
        "Spot Market {} withdraws are currently paused",
        spot_market.market_index
    )?;

    validate!(
        !(spot_market.asset_tier == AssetTier::Protected
            && user.spot_positions[spot_position_index].balance_type() == &SpotBalanceType::Borrow),
        DexError::ProtectedAssetTierViolation,
        "Spot Market {} has Protected status and cannot be borrowed",
        spot_market.market_index
    )?;

    Ok(())
}

#[cfg(test)]
pub fn transfer_spot_position_deposit(
    token_amount: i128,
    spot_market: &mut SpotMarket,
    from_spot_position: &mut SpotPosition,
    to_spot_position: &mut SpotPosition,
) -> VortexDexResult {
    use crate::{errors::DexError, validate};

    validate!(
        from_spot_position.market_index == to_spot_position.market_index,
        DexError::UnequalMarketIndexForSpotTransfer,
        "transfer market indexes arent equal",
    )?;

    if token_amount < 0 {
        update_spot_balances_and_cumulative_deposits(
            token_amount.unsigned_abs(),
            &SpotBalanceType::Deposit,
            spot_market,
            from_spot_position,
            false,
            None,
        )?;

        update_spot_balances_and_cumulative_deposits(
            token_amount.unsigned_abs(),
            &SpotBalanceType::Borrow,
            spot_market,
            to_spot_position,
            false,
            None,
        )?;
    } else {
        update_spot_balances_and_cumulative_deposits(
            token_amount.unsigned_abs(),
            &SpotBalanceType::Deposit,
            spot_market,
            to_spot_position,
            false,
            None,
        )?;

        update_spot_balances_and_cumulative_deposits(
            token_amount.unsigned_abs(),
            &SpotBalanceType::Borrow,
            spot_market,
            from_spot_position,
            false,
            None,
        )?;
    }

    Ok(())
}

pub fn charge_withdraw_fee(
    spot_market: &mut SpotMarket,
    oracle_price: i64,
    user: &mut User,
    user_stats: &mut UserStats,
) -> VortexDexResult<u128> {
    let fee_quote = QUOTE_PRECISION / 2000;
    let fee = fee_quote
        .safe_mul(spot_market.get_precision().cast()?)?
        .safe_div(oracle_price.unsigned_abs().cast()?)?;

    user.update_cumulative_spot_fees(-fee.cast()?)?;
    user_stats.increment_total_fees(fee.cast()?)?;

    msg!("Charging withdraw fee of {}", fee);

    update_revenue_pool_balances(fee, &SpotBalanceType::Deposit, spot_market)?;

    let position_index = user.force_get_spot_position_index(spot_market.market_index)?;
    update_spot_balances_and_cumulative_deposits(
        fee,
        &SpotBalanceType::Borrow,
        spot_market,
        &mut user.spot_positions[position_index],
        false,
        Some(0), // to make fee show in cumulative deposits
    )?;

    Ok(fee)
}
