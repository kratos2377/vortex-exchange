use std::ops::Deref;

use anchor_lang::prelude::*;
use crate::{casting::Cast, controllers::{self, spot_balance::{update_revenue_pool_balances, update_spot_market_and_check_validity}, spot_position::update_spot_balances_and_cumulative_deposits}, dex_state::DexState, errors::{DexError, VortexDexResult}, events::{LiquidateSpotRecord, LiquidationRecord, LiquidationType, OrderActionExplanation, SpotBankruptcyRecord}, margin_calculation::{MarginCalculation, MarginContext, MarketIdentifier}, operations::SpotOperation, oracle_map::OracleMap, safe_methods::SafeMath, spot_market::SpotBalanceType, spot_market_map::SpotMarketMap, user::User, user_stats::UserStats, utils::{constants::{LIQUIDATION_FEE_PRECISION_U128, LIQUIDATION_PCT_PRECISION, QUOTE_PRECISION_I128}, liquidation_utils::{calculate_asset_transfer_for_liability_transfer, calculate_cumulative_deposit_interest_delta_to_resolve_bankruptcy, calculate_liability_transfer_implied_by_asset_amount, calculate_liability_transfer_to_cover_margin_shortage, calculate_liquidation_multiplier, calculate_margin_freed, calculate_max_pct_to_liquidate, calculate_spot_if_fee, validate_transfer_satisfies_limit_price, LiquidationMultiplierType}, margin_utils::{calculate_margin_requirement_and_total_collateral_and_liability_info, MarginRequirementType}, oracle_utils::VortexDexAction, order_utils::is_oracle_too_divergent_with_twap_5min, spot_market_utils::get_token_value, user_utils::is_user_bankrupt}, validate};


pub fn liquidate_spot(
    asset_market_index: u16,
    liability_market_index: u16,
    liquidator_max_liability_transfer: u128,
    limit_price: Option<u64>,
    user: &mut User,
    user_key: &Pubkey,
    user_stats: &mut UserStats,
    liquidator: &mut User,
    liquidator_key: &Pubkey,
    liquidator_stats: &mut UserStats,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    now: i64,
    slot: u64,
    state: &DexState,
) -> VortexDexResult {
    let liquidation_margin_buffer_ratio = state.liquidation_margin_buffer_ratio;
    let initial_pct_to_liquidate = state.initial_pct_to_liquidate as u128;
    let liquidation_duration = state.liquidation_duration as u128;

    validate!(
        !user.is_bankrupt(),
        DexError::UserBankrupt,
        "user bankrupt",
    )?;

    validate!(
        !liquidator.is_bankrupt(),
        DexError::UserBankrupt,
        "liquidator bankrupt",
    )?;

    let asset_spot_market = spot_market_map.get_ref(&asset_market_index)?;

    validate!(
        !asset_spot_market.is_operation_paused(SpotOperation::Liquidation),
        DexError::InvalidLiquidation,
        "Liquidation operation is paused for market {}",
        asset_market_index
    )?;

    drop(asset_spot_market);

    let liability_spot_market = spot_market_map.get_ref(&liability_market_index)?;

    validate!(
        !liability_spot_market.is_operation_paused(SpotOperation::Liquidation),
        DexError::InvalidLiquidation,
        "Liquidation operation is paused for market {}",
        liability_market_index
    )?;

    drop(liability_spot_market);

    // validate user and liquidator have spot balances
    user.get_spot_position(asset_market_index).map_err(|_| {
        msg!(
            "User does not have a spot balance for asset market {}",
            asset_market_index
        );
        DexError::CouldNotFindSpotPosition
    })?;

    user.get_spot_position(liability_market_index)
        .map_err(|_| {
            msg!(
                "User does not have a spot balance for liability market {}",
                liability_market_index
            );
            DexError::CouldNotFindSpotPosition
        })?;

    liquidator
        .force_get_spot_position_mut(asset_market_index)
        .map_err(|e| {
            msg!("Liquidator has no available spot balances to take on deposit");
            e
        })?;

    liquidator
        .force_get_spot_position_mut(liability_market_index)
        .map_err(|e| {
            msg!("Liquidator has no available spot balances to take on borrow");
            e
        })?;

    let (asset_amount, asset_price, asset_decimals, asset_weight, asset_liquidation_multiplier) = {
        let mut asset_market = spot_market_map.get_ref_mut(&asset_market_index)?;
        let (asset_price_data, validity_guard_rails) =
            oracle_map.get_price_data_and_guard_rails(&asset_market.oracle)?;

        update_spot_market_and_check_validity(
            &mut asset_market,
            asset_price_data,
            validity_guard_rails,
            now,
            Some(VortexDexAction::Liquidate),
        )?;

        let spot_deposit_position = user.get_spot_position(asset_market_index)?;

        validate!(
            spot_deposit_position.balance_type == SpotBalanceType::Deposit,
            DexError::WrongSpotBalanceType,
            "User did not have a deposit for the asset market index"
        )?;

        let token_amount = spot_deposit_position.get_token_amount(&asset_market)?;

        validate!(
            token_amount != 0,
            DexError::InvalidSpotPosition,
            "asset token amount zero for market index = {}",
            asset_market_index
        )?;

        let asset_price = asset_price_data.price;
        (
            token_amount,
            asset_price,
            asset_market.decimals,
            asset_market.maintenance_asset_weight,
            calculate_liquidation_multiplier(
                asset_market.liquidator_fee,
                LiquidationMultiplierType::Premium,
            )?,
        )
    };

    let (
        liability_amount,
        liability_price,
        liability_decimals,
        liability_weight,
        liability_liquidation_multiplier,
    ) = {
        let mut liability_market = spot_market_map.get_ref_mut(&liability_market_index)?;
        let (liability_price_data, validity_guard_rails) =
            oracle_map.get_price_data_and_guard_rails(&liability_market.oracle)?;

        update_spot_market_and_check_validity(
            &mut liability_market,
            liability_price_data,
            validity_guard_rails,
            now,
            Some(VortexDexAction::Liquidate),
        )?;

        let spot_position = user.get_spot_position(liability_market_index)?;

        validate!(
            spot_position.balance_type == SpotBalanceType::Borrow,
            DexError::WrongSpotBalanceType,
            "User did not have a borrow for the liability market index"
        )?;

        let token_amount = spot_position.get_token_amount(&liability_market)?;

        validate!(
            token_amount != 0,
            DexError::InvalidSpotPosition,
            "liability token amount zero for market index = {}",
            liability_market_index
        )?;

        let liability_price = liability_price_data.price;

        (
            token_amount,
            liability_price,
            liability_market.decimals,
            liability_market.maintenance_liability_weight,
            calculate_liquidation_multiplier(
                liability_market.liquidator_fee,
                LiquidationMultiplierType::Discount,
            )?,
        )
    };

    let margin_context = MarginContext::liquidation(liquidation_margin_buffer_ratio)
        .track_market_margin_requirement(MarketIdentifier::spot(liability_market_index))?
        .fuel_numerator(user, now);

    let margin_calculation = user.calculate_margin_and_increment_fuel_bonus(
        spot_market_map,
        oracle_map,
        margin_context,
        user_stats,
        now,
    )?;

    if !user.is_being_liquidated() && margin_calculation.meets_margin_requirement() {
        msg!("margin calculation: {:?}", margin_calculation);
        return Err(DexError::SufficientCollateral);
    } else if user.is_being_liquidated() && margin_calculation.can_exit_liquidation()? {
        user.exit_liquidation();
        return Ok(());
    }

    let liquidation_id = user.enter_liquidation(slot)?;
    let mut margin_freed = 0_u64;

    let canceled_order_ids = controllers::orders::cancel_orders(
        user,
        user_key,
        Some(liquidator_key),
        spot_market_map,
        oracle_map,
        now,
        slot,
        OrderActionExplanation::Liquidation,
        None,
        None,
        None,
    )?;

    // check if user exited liquidation territory
    let intermediate_margin_calculation = if !canceled_order_ids.is_empty() {
        let intermediate_margin_calculation =
            calculate_margin_requirement_and_total_collateral_and_liability_info(
                user,
                spot_market_map,
                oracle_map,
                MarginContext::liquidation(liquidation_margin_buffer_ratio)
                    .track_market_margin_requirement(MarketIdentifier::spot(
                        liability_market_index,
                    ))?
                    .fuel_numerator(user, now),
            )?;

        let initial_margin_shortage = margin_calculation.margin_shortage()?;
        let new_margin_shortage = intermediate_margin_calculation.margin_shortage()?;

        margin_freed = initial_margin_shortage
            .saturating_sub(new_margin_shortage)
            .cast::<u64>()?;
        user.increment_margin_freed(margin_freed)?;

        if intermediate_margin_calculation.can_exit_liquidation()? {
            emit!(LiquidationRecord {
                ts: now,
                liquidation_id,
                liquidation_type: LiquidationType::LiquidateSpot,
                user: *user_key,
                liquidator: *liquidator_key,
                margin_requirement: margin_calculation.margin_requirement,
                total_collateral: margin_calculation.total_collateral,
                bankrupt: user.is_bankrupt(),
                canceled_order_ids,
                margin_freed,
                liquidate_spot: LiquidateSpotRecord {
                    asset_market_index,
                    asset_price,
                    asset_transfer: 0,
                    liability_market_index,
                    liability_price,
                    liability_transfer: 0,
                    if_fee: 0,
                },
                ..LiquidationRecord::default()
            });

            user.exit_liquidation();
            return Ok(());
        }

        intermediate_margin_calculation
    } else {
        margin_calculation
    };

    let margin_shortage = intermediate_margin_calculation.margin_shortage()?;

    let liability_weight_with_buffer =
        liability_weight.safe_add(liquidation_margin_buffer_ratio)?;

    let liquidation_if_fee = calculate_spot_if_fee(
        intermediate_margin_calculation.tracked_market_margin_shortage(margin_shortage)?,
        liability_amount,
        asset_weight,
        asset_liquidation_multiplier,
        liability_weight_with_buffer,
        liability_liquidation_multiplier,
        liability_decimals,
        liability_price,
        spot_market_map
            .get_ref(&liability_market_index)?
            .if_liquidation_fee,
    )?;

    // Determine what amount of borrow to transfer to reduce margin shortage to 0
    let liability_transfer_to_cover_margin_shortage =
        calculate_liability_transfer_to_cover_margin_shortage(
            margin_shortage,
            asset_weight,
            asset_liquidation_multiplier,
            liability_weight_with_buffer,
            liability_liquidation_multiplier,
            liability_decimals,
            liability_price,
            liquidation_if_fee,
        )?;

    let max_pct_allowed = calculate_max_pct_to_liquidate(
        user,
        margin_shortage,
        slot,
        initial_pct_to_liquidate,
        liquidation_duration,
    )?;
    let max_liability_allowed_to_be_transferred = liability_transfer_to_cover_margin_shortage
        .saturating_mul(max_pct_allowed)
        .safe_div(LIQUIDATION_PCT_PRECISION)?;

    if max_liability_allowed_to_be_transferred == 0 {
        msg!("max_liability_allowed_to_be_transferred == 0");
        return Ok(());
    }

    // Given the user's deposit amount, how much borrow can be transferred?
    let liability_transfer_implied_by_asset_amount =
        calculate_liability_transfer_implied_by_asset_amount(
            asset_amount,
            asset_liquidation_multiplier,
            asset_decimals,
            asset_price,
            liability_liquidation_multiplier,
            liability_decimals,
            liability_price,
        )?;

    let liability_value = get_token_value(
        liability_amount.cast()?,
        liability_decimals,
        liability_price,
    )?;

    let minimum_liability_transfer = if liability_value > 10 * QUOTE_PRECISION_I128 {
        0_u128
    } else {
        liability_amount
    };

    let liability_transfer = liquidator_max_liability_transfer
        .min(liability_amount)
        // want to make sure the liability_transfer_to_cover_margin_shortage doesn't lead to dust positions
        .min(max_liability_allowed_to_be_transferred.max(minimum_liability_transfer))
        .min(liability_transfer_implied_by_asset_amount);

    // Given the borrow amount to transfer, determine how much deposit amount to transfer
    let asset_transfer = calculate_asset_transfer_for_liability_transfer(
        asset_amount,
        asset_liquidation_multiplier,
        asset_decimals,
        asset_price,
        liability_transfer,
        liability_liquidation_multiplier,
        liability_decimals,
        liability_price,
    )?;

    if asset_transfer == 0 || liability_transfer == 0 {
        msg!(
            "asset_market_index {} liability_market_index {}",
            asset_market_index,
            liability_market_index
        );
        msg!("liquidator_max_liability_transfer {} liability_amount {} liability_transfer_to_cover_margin_shortage {}", liquidator_max_liability_transfer, liability_amount, liability_transfer_to_cover_margin_shortage);
        msg!(
            "liability_transfer_implied_by_asset_amount {} liability_transfer {} asset_transfer {}",
            liability_transfer_implied_by_asset_amount,
            liability_transfer,
            asset_transfer
        );
        return Err(DexError::InvalidLiquidation);
    }

    let liability_oracle_too_divergent = is_oracle_too_divergent_with_twap_5min(
        liability_price.cast()?,
        spot_market_map
            .get_ref(&liability_market_index)?
            .historical_oracle_data
            .last_oracle_price_twap_5min,
        state
            .oracle_guard_rails
            .max_oracle_twap_5min_percent_divergence()
            .cast()?,
    )?;

    validate!(
        !liability_oracle_too_divergent,
        DexError::PriceBandsBreached,
        "liability oracle too divergent"
    )?;

    let asset_oracle_too_divergent = is_oracle_too_divergent_with_twap_5min(
        asset_price.cast()?,
        spot_market_map
            .get_ref(&asset_market_index)?
            .historical_oracle_data
            .last_oracle_price_twap_5min,
        state
            .oracle_guard_rails
            .max_oracle_twap_5min_percent_divergence()
            .cast()?,
    )?;

    validate!(
        !asset_oracle_too_divergent,
        DexError::PriceBandsBreached,
        "asset oracle too divergent"
    )?;

    validate_transfer_satisfies_limit_price(
        asset_transfer,
        liability_transfer,
        asset_decimals,
        liability_decimals,
        limit_price,
    )?;

    let if_fee = liability_transfer
        .safe_mul(liquidation_if_fee.cast()?)?
        .safe_div(LIQUIDATION_FEE_PRECISION_U128)?;
    {
        let mut liability_market = spot_market_map.get_ref_mut(&liability_market_index)?;

        update_spot_balances_and_cumulative_deposits(
            liability_transfer.safe_sub(if_fee)?,
            &SpotBalanceType::Deposit,
            &mut liability_market,
            user.get_spot_position_mut(liability_market_index)?,
            false,
            Some(liability_transfer.safe_sub(if_fee)?),
        )?;

        update_revenue_pool_balances(if_fee, &SpotBalanceType::Deposit, &mut liability_market)?;

        update_spot_balances_and_cumulative_deposits(
            liability_transfer,
            &SpotBalanceType::Borrow,
            &mut liability_market,
            liquidator.get_spot_position_mut(liability_market_index)?,
            false,
            Some(liability_transfer),
        )?;
    }

    {
        let mut asset_market = spot_market_map.get_ref_mut(&asset_market_index)?;

        update_spot_balances_and_cumulative_deposits(
            asset_transfer,
            &SpotBalanceType::Deposit,
            &mut asset_market,
            liquidator.force_get_spot_position_mut(asset_market_index)?,
            false,
            Some(asset_transfer),
        )?;

        update_spot_balances_and_cumulative_deposits(
            asset_transfer,
            &SpotBalanceType::Borrow,
            &mut asset_market,
            user.force_get_spot_position_mut(asset_market_index)?,
            false,
            Some(asset_transfer),
        )?;
    }

    let (margin_freed_from_liability, _) = calculate_margin_freed(
        user,
        spot_market_map,
        oracle_map,
        liquidation_margin_buffer_ratio,
        margin_shortage,
    )?;
    margin_freed = margin_freed.safe_add(margin_freed_from_liability)?;
    user.increment_margin_freed(margin_freed_from_liability)?;

    if liability_transfer >= liability_transfer_to_cover_margin_shortage {
        user.exit_liquidation();
    } else if is_user_bankrupt(user) {
        user.enter_bankruptcy();
    }

    let liq_margin_context = MarginContext::standard(MarginRequirementType::Initial)
        .fuel_spot_deltas([
            (asset_market_index, -(asset_transfer as i128)),
            (liability_market_index, liability_transfer as i128),
        ])
        .fuel_numerator(liquidator, now);

    let liquidator_meets_initial_margin_requirement = liquidator
        .calculate_margin_and_increment_fuel_bonus(
            spot_market_map,
            oracle_map,
            liq_margin_context,
            liquidator_stats,
            now,
        )
        .map(|calc| calc.meets_margin_requirement())?;

    validate!(
        liquidator_meets_initial_margin_requirement,
        DexError::InsufficientCollateral,
        "Liquidator doesnt have enough collateral to take over borrow"
    )?;

    emit!(LiquidationRecord {
        ts: now,
        liquidation_id,
        liquidation_type: LiquidationType::LiquidateSpot,
        user: *user_key,
        liquidator: *liquidator_key,
        margin_requirement: margin_calculation.margin_requirement,
        total_collateral: margin_calculation.total_collateral,
        bankrupt: user.is_bankrupt(),
        margin_freed,
        liquidate_spot: LiquidateSpotRecord {
            asset_market_index,
            asset_price,
            asset_transfer,
            liability_market_index,
            liability_price,
            liability_transfer,
            if_fee: if_fee.cast()?,
        },
        ..LiquidationRecord::default()
    });

    Ok(())
}

pub fn resolve_spot_bankruptcy(
    market_index: u16,
    user: &mut User,
    user_key: &Pubkey,
    liquidator: &mut User,
    liquidator_key: &Pubkey,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    now: i64,
    insurance_fund_vault_balance: u64,
) -> VortexDexResult<u64> {
    if !user.is_bankrupt() && is_user_bankrupt(user) {
        user.enter_bankruptcy();
    }

    validate!(
        user.is_bankrupt(),
        DexError::UserNotBankrupt,
        "user not bankrupt",
    )?;

    validate!(
        !liquidator.is_being_liquidated(),
        DexError::UserIsBeingLiquidated,
        "liquidator being liquidated",
    )?;

    validate!(
        !liquidator.is_bankrupt(),
        DexError::UserBankrupt,
        "liquidator bankrupt",
    )?;

    let market = spot_market_map.get_ref(&market_index)?;

    validate!(
        !market.is_operation_paused(SpotOperation::Liquidation),
        DexError::InvalidLiquidation,
        "Liquidation operation is paused for market {}",
        market_index
    )?;

    drop(market);

    // validate user and liquidator have spot position balances
    user.get_spot_position(market_index).map_err(|_| {
        msg!(
            "User does not have a spot balance for market {}",
            market_index
        );
        DexError::CouldNotFindSpotPosition
    })?;

    let MarginCalculation {
        margin_requirement,
        total_collateral,
        ..
    } = calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
        spot_market_map,
        oracle_map,
        MarginContext::standard(MarginRequirementType::Maintenance),
    )?;

    let borrow_amount = {
        let spot_position = user.get_spot_position(market_index)?;
        validate!(
            spot_position.balance_type == SpotBalanceType::Borrow,
            DexError::UserHasInvalidBorrow
        )?;

        validate!(
            spot_position.scaled_balance > 0,
            DexError::UserHasInvalidBorrow
        )?;

        spot_position.get_token_amount(spot_market_map.get_ref(&market_index)?.deref())?
    };

    // todo: add market's insurance fund draw attempt here (before social loss)
    // subtract 1 so insurance_fund_vault_balance always stays >= 1
    let if_payment = borrow_amount.min(insurance_fund_vault_balance.saturating_sub(1).cast()?);

    let loss_to_socialize = borrow_amount.safe_sub(if_payment)?;

    let cumulative_deposit_interest_delta =
        calculate_cumulative_deposit_interest_delta_to_resolve_bankruptcy(
            loss_to_socialize,
            spot_market_map.get_ref(&market_index)?.deref(),
        )?;

    {
        let mut spot_market = spot_market_map.get_ref_mut(&market_index)?;
        let oracle_price_data = &oracle_map.get_price_data(&spot_market.oracle)?;
        let quote_social_loss = get_token_value(
            -borrow_amount.cast()?,
            spot_market.decimals,
            oracle_price_data.price,
        )?;
        user.increment_total_socialized_loss(quote_social_loss.unsigned_abs().cast()?)?;

        let spot_position = user.get_spot_position_mut(market_index)?;
        update_spot_balances_and_cumulative_deposits(
            borrow_amount,
            &SpotBalanceType::Deposit,
            &mut spot_market,
            spot_position,
            false,
            None,
        )?;

        spot_market.cumulative_deposit_interest = spot_market
            .cumulative_deposit_interest
            .safe_sub(cumulative_deposit_interest_delta)?;

        spot_market.total_social_loss = spot_market
            .total_social_loss
            .safe_add(borrow_amount.cast()?)?;

        spot_market.total_quote_social_loss = spot_market
            .total_quote_social_loss
            .safe_add(quote_social_loss.unsigned_abs().cast()?)?;
    }

    // exit bankruptcy
    if !is_user_bankrupt(user) {
        user.exit_bankruptcy();
    }

    let liquidation_id = user.next_liquidation_id.safe_sub(1)?;

    emit!(LiquidationRecord {
        ts: now,
        liquidation_id,
        liquidation_type: LiquidationType::SpotBankruptcy,
        user: *user_key,
        liquidator: *liquidator_key,
        margin_requirement,
        total_collateral,
        bankrupt: true,
        spot_bankruptcy: SpotBankruptcyRecord {
            market_index,
            borrow_amount,
            if_payment,
            cumulative_deposit_interest_delta,
        },
        ..LiquidationRecord::default()
    });

    if_payment.cast()
}
