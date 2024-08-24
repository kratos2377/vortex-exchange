use anchor_lang::prelude::Pubkey;
use solana_program::msg;

use crate::{errors::{DexError, VortexDexResult}, math_error, print_error, state::{fulfillment::SpotFulfillmentMethod, margin_calculation::{MarginCalculation, MarginContext}, oracle::StrictOraclePrice, oracle_map::OracleMap, position::PositionDirection, spot_market::SpotMarket, spot_market_map::SpotMarketMap, user::{MarketType, Order, OrderFillSimulation, OrderStatus, OrderTriggerCondition, OrderType, User}}, validate};

use super::{constants::{MARGIN_PRECISION_U128, OPEN_ORDER_MARGIN_REQUIREMENT, PERCENTAGE_PRECISION, PERCENTAGE_PRECISION_U64, PRICE_PRECISION_I128, QUOTE_PRECISION_I128, SPOT_WEIGHT_PRECISION, SPOT_WEIGHT_PRECISION_I128}, margin_utils::{calculate_margin_requirement_and_total_collateral_and_liability_info, MarginRequirementType}, matching_utils::do_orders_cross, spot_market_utils::{get_max_withdraw_for_market_with_token_amount, get_strict_token_value}};



#[inline(always)]
pub fn should_expire_order(user: &User, user_order_index: usize, now: i64) -> VortexDexResult<bool> {
    let order = &user.orders[user_order_index];
    if order.status != OrderStatus::Open || order.max_ts == 0 || order.must_be_triggered() {
        return Ok(false);
    }

    Ok(now > order.max_ts)
}

pub fn validate_order_for_force_reduce_only(order: &Order, existing_position: i64) -> VortexDexResult {
    validate!(
        order.reduce_only,
        DexError::InvalidOrderNotRiskReducing,
        "order must be reduce only",
    )?;

    validate!(
        existing_position != 0,
        DexError::InvalidOrderNotRiskReducing,
        "user must have position to submit order",
    )?;

    let existing_position_direction = if existing_position > 0 {
        PositionDirection::Long
    } else {
        PositionDirection::Short
    };

    validate!(
        order.direction != existing_position_direction,
        DexError::InvalidOrderNotRiskReducing,
        "order direction must be opposite of existing position in reduce only mode",
    )?;

    Ok(())
}


fn calculate_free_collateral_delta_for_spot(
    spot_market: &SpotMarket,
    worst_case_token_amount: u128,
    strict_oracle_price: &StrictOraclePrice,
    order_direction: PositionDirection,
    user_custom_liability_weight: u32,
    user_custom_asset_weight: u32,
) -> VortexDexResult<u32> {
    Ok(if order_direction == PositionDirection::Long {
        SPOT_WEIGHT_PRECISION.sub(
            spot_market
                .get_asset_weight(
                    worst_case_token_amount,
                    strict_oracle_price.current,
                    &MarginRequirementType::Initial,
                )?
                .min(user_custom_asset_weight),
        )
    } else {
        spot_market
            .get_liability_weight(worst_case_token_amount, &MarginRequirementType::Initial)?
            .max(user_custom_liability_weight)
            .sub(SPOT_WEIGHT_PRECISION)
    })
}

#[allow(clippy::unwrap_used)]
pub fn calculate_max_spot_order_size(
    user: &User,
    market_index: u16,
    direction: PositionDirection,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
) -> VortexDexResult<u64> {
    // calculate initial margin requirement
    let MarginCalculation {
        margin_requirement,
        total_collateral,
        ..
    } = calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
        spot_market_map,
        oracle_map,
        MarginContext::standard(MarginRequirementType::Initial).strict(true),
    )?;

    let user_custom_margin_ratio = user.max_margin_ratio;
    let user_custom_liability_weight = user.max_margin_ratio.saturating_add(SPOT_WEIGHT_PRECISION);
    let user_custom_asset_weight = SPOT_WEIGHT_PRECISION.saturating_sub(user_custom_margin_ratio);

    let mut order_size_to_flip = 0_u64;
    let free_collateral = total_collateral.safe_sub(margin_requirement.cast()?)?;

    let spot_market = spot_market_map.get_ref(&market_index)?;

    let oracle_price_data = oracle_map.get_price_data(&spot_market.oracle)?;
    let twap = spot_market
        .historical_oracle_data
        .last_oracle_price_twap_5min;
    let strict_oracle_price = StrictOraclePrice::new(oracle_price_data.price, twap, true);
    let max_oracle_price = strict_oracle_price.max();

    let spot_position = user.get_spot_position(market_index)?;
    let signed_token_amount = spot_position.get_signed_token_amount(&spot_market)?;

    let [bid_simulation, ask_simulation] = spot_position
        .simulate_fills_both_sides(
            &spot_market,
            &strict_oracle_price,
            Some(signed_token_amount),
            MarginRequirementType::Initial,
        )?
        .map(|simulation| {
            simulation
                .apply_user_custom_margin_ratio(
                    &spot_market,
                    strict_oracle_price.current,
                    user_custom_margin_ratio,
                )
                .unwrap()
        });

    let OrderFillSimulation {
        token_amount: mut worst_case_token_amount,
        ..
    } = OrderFillSimulation::riskier_side(ask_simulation, bid_simulation);

    // account for order flipping worst case
    if worst_case_token_amount < 0 && direction == PositionDirection::Long {
        // to determine order size to flip direction, need to know diff in free collateral
        let mut free_collateral_difference = bid_simulation
            .free_collateral_contribution
            .safe_sub(ask_simulation.free_collateral_contribution)?
            .max(0)
            .abs();

        let mut token_amount = bid_simulation.token_amount;

        // the free collateral delta is positive until the worst case hits 0
        if token_amount < 0 {
            let token_value =
                get_strict_token_value(token_amount, spot_market.decimals, &strict_oracle_price)?;

            let liability_weight = spot_market
                .get_liability_weight(token_amount.unsigned_abs(), &MarginRequirementType::Initial)?
                .max(user_custom_liability_weight);

            let free_collateral_regained = token_value
                .abs()
                .safe_mul(liability_weight.safe_sub(SPOT_WEIGHT_PRECISION)?.cast()?)?
                .safe_div(SPOT_WEIGHT_PRECISION_I128)?;

            free_collateral_difference =
                free_collateral_difference.safe_add(free_collateral_regained)?;

            order_size_to_flip = token_amount.abs().cast()?;
            token_amount = 0;
        }

        // free collateral delta is negative as the worst case goes above 0
        let weight = spot_market
            .get_asset_weight(
                token_amount.unsigned_abs(),
                strict_oracle_price.current,
                &MarginRequirementType::Initial,
            )?
            .min(user_custom_asset_weight);

        let free_collateral_delta_per_order = weight
            .cast::<i128>()?
            .safe_sub(SPOT_WEIGHT_PRECISION_I128)?
            .abs()
            .safe_mul(max_oracle_price.cast()?)?
            .safe_div(PRICE_PRECISION_I128)?
            .safe_mul(QUOTE_PRECISION_I128)?
            .safe_div(SPOT_WEIGHT_PRECISION_I128)?;

        order_size_to_flip = order_size_to_flip.safe_add(
            free_collateral_difference
                .safe_mul(spot_market.get_precision().cast()?)?
                .safe_div(free_collateral_delta_per_order)?
                .cast::<u64>()?,
        )?;

        worst_case_token_amount = token_amount.safe_sub(order_size_to_flip.cast()?)?;
    } else if worst_case_token_amount > 0 && direction == PositionDirection::Short {
        let mut free_collateral_difference = ask_simulation
            .free_collateral_contribution
            .safe_sub(bid_simulation.free_collateral_contribution)?
            .max(0)
            .abs();

        let mut token_amount = ask_simulation.token_amount;

        if token_amount > 0 {
            let token_value =
                get_strict_token_value(token_amount, spot_market.decimals, &strict_oracle_price)?;

            let asset_weight = spot_market
                .get_asset_weight(
                    token_amount.unsigned_abs(),
                    strict_oracle_price.current,
                    &MarginRequirementType::Initial,
                )?
                .min(user_custom_asset_weight);

            let free_collateral_regained = token_value
                .abs()
                .safe_mul(SPOT_WEIGHT_PRECISION.safe_sub(asset_weight)?.cast()?)?
                .safe_div(SPOT_WEIGHT_PRECISION_I128)?;

            free_collateral_difference =
                free_collateral_difference.safe_add(free_collateral_regained)?;

            order_size_to_flip = token_amount.abs().cast()?;
            token_amount = 0;
        }

        let weight = spot_market
            .get_liability_weight(token_amount.unsigned_abs(), &MarginRequirementType::Initial)?
            .max(user_custom_liability_weight);

        let free_collateral_delta_per_order = weight
            .cast::<i128>()?
            .safe_sub(SPOT_WEIGHT_PRECISION_I128)?
            .abs()
            .safe_mul(max_oracle_price.cast()?)?
            .safe_div(PRICE_PRECISION_I128)?
            .safe_mul(QUOTE_PRECISION_I128)?
            .safe_div(SPOT_WEIGHT_PRECISION_I128)?;

        order_size_to_flip = order_size_to_flip.safe_add(
            free_collateral_difference
                .safe_mul(spot_market.get_precision().cast()?)?
                .safe_div(free_collateral_delta_per_order)?
                .cast::<u64>()?,
        )?;

        worst_case_token_amount = token_amount.safe_sub(order_size_to_flip.cast()?)?;
    }

    if free_collateral <= 0 {
        return standardize_base_asset_amount(order_size_to_flip, spot_market.order_step_size);
    }

    let free_collateral_delta = calculate_free_collateral_delta_for_spot(
        &spot_market,
        worst_case_token_amount.unsigned_abs(),
        &strict_oracle_price,
        direction,
        user_custom_liability_weight,
        user_custom_asset_weight,
    )?;

    let precision_increase = 10i128.pow(spot_market.decimals - 6);

    let calculate_order_size_and_free_collateral_delta = |free_collateral_delta: u32| {
        let new_order_size = free_collateral
            .safe_sub(OPEN_ORDER_MARGIN_REQUIREMENT.cast()?)?
            .safe_mul(precision_increase)?
            .safe_mul(SPOT_WEIGHT_PRECISION.cast()?)?
            .safe_div(free_collateral_delta.cast()?)?
            .safe_mul(PRICE_PRECISION_I128)?
            .safe_div(max_oracle_price.cast()?)?
            .cast::<u64>()?;

        // increasing the worst case token amount with new order size may increase margin ratio,
        // so need to recalculate free collateral delta with updated margin ratio
        let new_free_collateral_delta = calculate_free_collateral_delta_for_spot(
            &spot_market,
            worst_case_token_amount
                .unsigned_abs()
                .safe_add(new_order_size.cast()?)?,
            &strict_oracle_price,
            direction,
            user_custom_liability_weight,
            user_custom_asset_weight,
        )?;

        Ok((new_order_size, new_free_collateral_delta))
    };

    let mut order_size = 0_u64;
    let mut updated_free_collateral_delta = free_collateral_delta;
    for _ in 0..6 {
        let (new_order_size, new_free_collateral_delta) =
            calculate_order_size_and_free_collateral_delta(updated_free_collateral_delta)?;
        order_size = new_order_size;
        updated_free_collateral_delta = new_free_collateral_delta;

        if updated_free_collateral_delta == free_collateral_delta {
            break;
        }
    }

    standardize_base_asset_amount(
        order_size.safe_add(order_size_to_flip)?,
        spot_market.order_step_size,
    )
}

pub fn standardize_base_asset_amount(base_asset_amount: u64, step_size: u64) -> VortexDexResult<u64> {
    let remainder = base_asset_amount
        .checked_rem_euclid(step_size)
        .ok_or_else(math_error!())?;

    base_asset_amount.safe_sub(remainder)
}


pub fn is_multiple_of_step_size(base_asset_amount: u64, step_size: u64) -> VortexDexResult<bool> {
    let remainder = base_asset_amount
        .checked_rem_euclid(step_size)
        .ok_or_else(math_error!())?;

    Ok(remainder == 0)
}

pub fn standardize_price(
    price: u64,
    tick_size: u64,
    direction: PositionDirection,
) -> VortexDexResult<u64> {
    if price == 0 {
        return Ok(0);
    }

    let remainder = price
        .checked_rem_euclid(tick_size)
        .ok_or_else(math_error!())?;

    if remainder == 0 {
        return Ok(price);
    }

    match direction {
        PositionDirection::Long => price.safe_sub(remainder),
        PositionDirection::Short => price.safe_add(tick_size)?.safe_sub(remainder),
    }
}

pub fn standardize_price_i64(
    price: i64,
    tick_size: i64,
    direction: PositionDirection,
) -> VortexDexResult<i64> {
    if price == 0 {
        return Ok(0);
    }

    let remainder = price
        .checked_rem_euclid(tick_size)
        .ok_or_else(math_error!())?;

    if remainder == 0 {
        return Ok(price);
    }

    match direction {
        PositionDirection::Long => price.safe_sub(remainder),
        PositionDirection::Short => price.safe_add(tick_size)?.safe_sub(remainder),
    }
}

pub fn is_new_order_risk_increasing(
    order: &Order,
    position_base_asset_amount: i64,
    position_bids: i64,
    position_asks: i64,
) -> VortexDexResult<bool> {
    if order.reduce_only {
        return Ok(false);
    }

    match order.direction {
        PositionDirection::Long => {
            if position_base_asset_amount >= 0 {
                return Ok(true);
            }

            Ok(position_bids.safe_add(order.base_asset_amount.cast()?)?
                > position_base_asset_amount.abs())
        }
        PositionDirection::Short => {
            if position_base_asset_amount <= 0 {
                return Ok(true);
            }

            Ok(position_asks
                .safe_sub(order.base_asset_amount.cast()?)?
                .abs()
                > position_base_asset_amount)
        }
    }
}

pub fn is_order_position_reducing(
    order_direction: &PositionDirection,
    order_base_asset_amount: u64,
    position_base_asset_amount: i64,
) -> VortexDexResult<bool> {
    Ok(match order_direction {
        // User is short and order is long
        PositionDirection::Long if position_base_asset_amount < 0 => {
            order_base_asset_amount <= position_base_asset_amount.unsigned_abs()
        }
        // User is long and order is short
        PositionDirection::Short if position_base_asset_amount > 0 => {
            order_base_asset_amount <= position_base_asset_amount.unsigned_abs()
        }
        _ => false,
    })
}

fn validate_base_asset_amount(
    order: &Order,
    step_size: u64,
    min_order_size: u64,
    reduce_only: bool,
) -> VortexDexResult {
    if order.base_asset_amount == 0 {
        msg!("Order base_asset_amount cant be 0");
        return Err(DexError::InvalidOrderSizeTooSmall);
    }

    validate!(
        is_multiple_of_step_size(order.base_asset_amount, step_size)?,
        DexError::InvalidOrderNotStepSizeMultiple,
        "Order base asset amount ({}) not a multiple of the step size ({})",
        order.base_asset_amount,
        step_size
    )?;

    validate!(
        reduce_only || order.base_asset_amount >= min_order_size,
        DexError::InvalidOrderMinOrderSize,
        "Order base_asset_amount ({}) < min_order_size ({})",
        order.base_asset_amount,
        min_order_size
    )?;

    Ok(())
}

pub fn validate_spot_order(order: &Order, step_size: u64, min_order_size: u64) -> VortexDexResult {
    match order.order_type {
        OrderType::Market => validate_market_order(order, step_size, min_order_size)?,
        OrderType::Limit => validate_spot_limit_order(order, step_size, min_order_size)?,
        OrderType::TriggerMarket => {
            validate_trigger_market_order(order, step_size, min_order_size)?
        }
        OrderType::TriggerLimit => validate_trigger_limit_order(order, step_size, min_order_size)?,
        OrderType::Oracle => validate_oracle_order(order, step_size, min_order_size)?,
    }

    Ok(())
}

fn validate_spot_limit_order(order: &Order, step_size: u64, min_order_size: u64) -> VortexDexResult {
    validate_base_asset_amount(order, step_size, min_order_size, order.reduce_only)?;

    if order.price == 0 && !order.has_oracle_price_offset() {
        msg!("Limit order price == 0");
        return Err(DexError::InvalidOrderLimitPrice);
    }

    if order.has_oracle_price_offset() && order.price != 0 {
        msg!("Limit order price must be 0 for taker oracle offset order");
        return Err(DexError::InvalidOrderOracleOffset);
    }

    if order.trigger_price > 0 {
        msg!("Limit order should not have trigger price");
        return Err(DexError::InvalidOrderTrigger);
    }

    if order.post_only {
        validate!(
            !order.has_auction(),
            DexError::InvalidOrder,
            "post only limit order cant have auction"
        )?;
    }

    validate_limit_order_auction_params(order)?;

    Ok(())
}

fn validate_market_order(order: &Order, step_size: u64, min_order_size: u64) -> VortexDexResult {
    validate_base_asset_amount(order, step_size, min_order_size, order.reduce_only)?;

    validate!(
        order.auction_start_price > 0 && order.auction_end_price > 0,
        DexError::InvalidOrderAuction,
        "Auction start and end price must be greater than 0"
    )?;

    validate_auction_params(order)?;

    if order.trigger_price > 0 {
        msg!("Market should not have trigger price");
        return Err(DexError::InvalidOrderTrigger);
    }

    if order.post_only {
        msg!("Market order can not be post only");
        return Err(DexError::InvalidOrderPostOnly);
    }

    if order.has_oracle_price_offset() {
        msg!("Market order can not have oracle offset");
        return Err(DexError::InvalidOrderOracleOffset);
    }

    if order.immediate_or_cancel {
        msg!("Market order can not be immediate or cancel");
        return Err(DexError::InvalidOrderIOC);
    }

    Ok(())
}

fn validate_oracle_order(order: &Order, step_size: u64, min_order_size: u64) -> VortexDexResult {
    validate_base_asset_amount(order, step_size, min_order_size, order.reduce_only)?;

    match order.direction {
        PositionDirection::Long => {
            if order.auction_start_price > order.auction_end_price {
                msg!(
                    "Auction start price offset ({}) was greater than auction end price offset ({})",
                    order.auction_start_price,
                    order.auction_end_price
                );
                return Err(DexError::InvalidOrderAuction);
            }

            if order.has_oracle_price_offset()
                && order.auction_end_price > order.oracle_price_offset.cast()?
            {
                msg!(
                    "Auction end price offset ({}) was greater than oracle price offset ({})",
                    order.auction_end_price,
                    order.oracle_price_offset
                );
                return Err(DexError::InvalidOrderAuction);
            }
        }
        PositionDirection::Short => {
            if order.auction_start_price < order.auction_end_price {
                msg!(
                    "Auction start price ({}) was less than auction end price ({})",
                    order.auction_start_price,
                    order.auction_end_price
                );
                return Err(DexError::InvalidOrderAuction);
            }

            if order.has_oracle_price_offset()
                && order.auction_end_price < order.oracle_price_offset.cast()?
            {
                msg!(
                    "Auction end price offset ({}) was less than oracle price offset ({})",
                    order.auction_end_price,
                    order.oracle_price_offset
                );
                return Err(DexError::InvalidOrderAuction);
            }
        }
    }

    if order.trigger_price > 0 {
        msg!("Oracle order should not have trigger price");
        return Err(DexError::InvalidOrderTrigger);
    }

    if order.post_only {
        msg!("Oracle order can not be post only");
        return Err(DexError::InvalidOrderPostOnly);
    }

    if order.price > 0 {
        msg!("Oracle order can not have a price");
        return Err(DexError::InvalidOrderLimitPrice);
    }

    if order.immediate_or_cancel {
        msg!("Oracle order can not be immediate or cancel");
        return Err(DexError::InvalidOrderIOC);
    }

    Ok(())
}


fn validate_limit_order_auction_params(order: &Order) -> VortexDexResult {
    if order.has_auction() {
        validate!(
            !order.has_oracle_price_offset(),
            DexError::InvalidOrder,
            "limit order with auction can not have an oracle price offset"
        )?;

        validate_auction_params(order)?;
    } else {
        validate!(
            order.auction_start_price == 0,
            DexError::InvalidOrder,
            "limit order without auction can not have an auction start price"
        )?;

        validate!(
            order.auction_end_price == 0,
            DexError::InvalidOrder,
            "limit order without auction can not have an auction end price"
        )?;
    }

    Ok(())
}


fn validate_trigger_limit_order(order: &Order, step_size: u64, min_order_size: u64) -> VortexDexResult {
    validate_base_asset_amount(order, step_size, min_order_size, order.reduce_only)?;

    if !matches!(
        order.trigger_condition,
        OrderTriggerCondition::Above | OrderTriggerCondition::Below
    ) {
        msg!("Invalid trigger condition, must be Above or Below");
        return Err(DexError::InvalidTriggerOrderCondition);
    }

    if order.price == 0 {
        msg!("Trigger limit order price == 0");
        return Err(DexError::InvalidOrderLimitPrice);
    }

    if order.trigger_price == 0 {
        msg!("Trigger price == 0");
        return Err(DexError::InvalidOrderTrigger);
    }

    if order.post_only {
        msg!("Trigger limit order can not be post only");
        return Err(DexError::InvalidOrderPostOnly);
    }

    if order.has_oracle_price_offset() {
        msg!("Trigger limit can not have oracle offset");
        return Err(DexError::InvalidOrderOracleOffset);
    }

    Ok(())
}

fn validate_trigger_market_order(
    order: &Order,
    step_size: u64,
    min_order_size: u64,
) -> VortexDexResult {
    validate_base_asset_amount(order, step_size, min_order_size, order.reduce_only)?;

    if !matches!(
        order.trigger_condition,
        OrderTriggerCondition::Above | OrderTriggerCondition::Below
    ) {
        msg!("Invalid trigger condition, must be Above or Below");
        return Err(DexError::InvalidTriggerOrderCondition);
    }

    if order.price > 0 {
        msg!("Trigger market order should not have price");
        return Err(DexError::InvalidOrderLimitPrice);
    }

    if order.trigger_price == 0 {
        msg!("Trigger market order trigger_price == 0");
        return Err(DexError::InvalidOrderTrigger);
    }

    if order.post_only {
        msg!("Trigger market order can not be post only");
        return Err(DexError::InvalidOrderPostOnly);
    }

    if order.has_oracle_price_offset() {
        msg!("Trigger market order can not have oracle offset");
        return Err(DexError::InvalidOrderOracleOffset);
    }

    Ok(())
}

fn validate_auction_params(order: &Order) -> VortexDexResult {
    validate!(
        order.auction_start_price != 0,
        DexError::InvalidOrderAuction,
        "Auction start price was 0"
    )?;

    validate!(
        order.auction_end_price != 0,
        DexError::InvalidOrderAuction,
        "Auction end price was 0"
    )?;

    match order.direction {
        PositionDirection::Long => {
            if order.auction_start_price > order.auction_end_price {
                msg!(
                    "Auction start price ({}) was greater than auction end price ({})",
                    order.auction_start_price,
                    order.auction_end_price
                );
                return Err(DexError::InvalidOrderAuction);
            }

            if order.price != 0 && order.price < order.auction_end_price.cast()? {
                msg!(
                    "Order price ({}) was less than auction end price ({})",
                    order.price,
                    order.auction_end_price
                );
                return Err(DexError::InvalidOrderAuction);
            }
        }
        PositionDirection::Short => {
            if order.auction_start_price < order.auction_end_price {
                msg!(
                    "Auction start price ({}) was less than auction end price ({})",
                    order.auction_start_price,
                    order.auction_end_price
                );
                return Err(DexError::InvalidOrderAuction);
            }

            if order.price != 0 && order.price > order.auction_end_price.cast()? {
                msg!(
                    "Order price ({}) was greater than auction end price ({})",
                    order.price,
                    order.auction_end_price
                );
                return Err(DexError::InvalidOrderAuction);
            }
        }
    }

    Ok(())
}


pub fn get_max_fill_amounts(
    user: &User,
    user_order_index: usize,
    base_market: &SpotMarket,
    quote_market: &SpotMarket,
    is_leaving_vortex: bool,
) -> VortexDexResult<(Option<u64>, Option<u64>)> {
    let direction: PositionDirection = user.orders[user_order_index].direction;
    match direction {
        PositionDirection::Long => {
            let max_quote = get_max_fill_amounts_for_market(user, quote_market, is_leaving_vortex)?
                .cast::<u64>()?;
            Ok((None, Some(max_quote)))
        }
        PositionDirection::Short => {
            let max_base = standardize_base_asset_amount(
                get_max_fill_amounts_for_market(user, base_market, is_leaving_vortex)?
                    .cast::<u64>()?,
                base_market.order_step_size,
            )?;
            Ok((Some(max_base), None))
        }
    }
}


fn get_max_fill_amounts_for_market(
    user: &User,
    market: &SpotMarket,
    is_leaving_vortex: bool,
) -> VortexDexResult<u128> {
    let position_index = user.get_spot_position_index(market.market_index)?;
    let token_amount = user.spot_positions[position_index].get_signed_token_amount(market)?;
    get_max_withdraw_for_market_with_token_amount(market, token_amount, is_leaving_vortex)
}

pub fn find_maker_orders(
    user: &User,
    direction: &PositionDirection,
    market_type: &MarketType,
    market_index: u16,
    valid_oracle_price: Option<i64>,
    slot: u64,
    tick_size: u64,
) -> VortexDexResult<Vec<(usize, u64)>> {
    let mut orders: Vec<(usize, u64)> = Vec::with_capacity(32);

    for (order_index, order) in user.orders.iter().enumerate() {
        if order.status != OrderStatus::Open {
            continue;
        }

        // if order direction is not same or market type is not same or market index is the same, skip
        if order.direction != *direction
            || order.market_type != *market_type
            || order.market_index != market_index
        {
            continue;
        }

        // if order is not limit order or must be triggered and not triggered, skip
        if !order.is_limit_order() || (order.must_be_triggered() && !order.triggered()) {
            continue;
        }

        let limit_price = order.force_get_limit_price(valid_oracle_price, None, slot, tick_size)?;

        orders.push((order_index, limit_price));
    }

    Ok(orders)
}

/// Cancel maker order if there limit price cross the oracle price sufficiently
/// E.g. if initial margin ratio is .05 and oracle price is 100, then maker limit price must be
/// less than 105 to be valid
pub fn limit_price_breaches_maker_oracle_price_bands(
    order_limit_price: u64,
    order_direction: PositionDirection,
    oracle_price: i64,
    margin_ratio_initial: u32,
) -> VortexDexResult<bool> {
    let oracle_price = oracle_price.unsigned_abs();

    let max_percent_diff = margin_ratio_initial;

    match order_direction {
        PositionDirection::Long => {
            if order_limit_price <= oracle_price {
                return Ok(false);
            }

            let percent_diff = order_limit_price
                .safe_sub(oracle_price)?
                .cast::<u128>()?
                .safe_mul(MARGIN_PRECISION_U128)?
                .safe_div(oracle_price.cast()?)?;

            if percent_diff >= max_percent_diff.cast()? {
                // order cant be buying if oracle price is more than 5% below limit price
                msg!(
                    "Limit Price Breaches Oracle for Long: {} >> {}",
                    order_limit_price,
                    oracle_price
                );
                return Ok(true);
            }

            Ok(false)
        }
        PositionDirection::Short => {
            if order_limit_price >= oracle_price {
                return Ok(false);
            }

            let percent_diff = oracle_price
                .safe_sub(order_limit_price)?
                .cast::<u128>()?
                .safe_mul(MARGIN_PRECISION_U128)?
                .safe_div(oracle_price.cast()?)?;

            if percent_diff >= max_percent_diff.cast()? {
                // order cant be selling if oracle price is more than 5% above limit price
                msg!(
                    "Limit Price Breaches Oracle for Short: {} << {}",
                    order_limit_price,
                    oracle_price
                );
                return Ok(true);
            }

            Ok(false)
        }
    }
}


pub fn calculate_quote_asset_amount_for_maker_order(
    base_asset_amount: u64,
    fill_price: u64,
    base_decimals: u32,
    position_direction: PositionDirection,
) -> VortexDexResult<u64> {
    let precision_decrease = 10_u128.pow(base_decimals);

    match position_direction {
        PositionDirection::Long => fill_price
            .cast::<u128>()?
            .safe_mul(base_asset_amount.cast()?)?
            .safe_div(precision_decrease)?
            .cast::<u64>(),
        PositionDirection::Short => fill_price
            .cast::<u128>()?
            .safe_mul(base_asset_amount.cast()?)?
            .safe_div_ceil(precision_decrease)?
            .cast::<u64>(),
    }
}

pub fn should_cancel_reduce_only_order(
    order: &Order,
    existing_base_asset_amount: i64,
    step_size: u64,
) -> VortexDexResult<bool> {
    let should_cancel = order.status == OrderStatus::Open
        && order.reduce_only
        && order.get_base_asset_amount_unfilled(Some(existing_base_asset_amount))? < step_size;

    Ok(should_cancel)
}

pub fn is_oracle_too_divergent_with_twap_5min(
    oracle_price: i64,
    oracle_twap_5min: i64,
    max_divergence: i64,
) -> VortexDexResult<bool> {
    let percent_diff = oracle_price
        .safe_sub(oracle_twap_5min)?
        .abs()
        .safe_mul(PERCENTAGE_PRECISION_U64.cast::<i64>()?)?
        .safe_div(oracle_twap_5min.abs())?;

    let too_divergent = percent_diff >= max_divergence;
    if too_divergent {
        msg!("max divergence {}", max_divergence);
        msg!(
            "Oracle Price Too Divergent from TWAP 5min. oracle: {} twap: {}",
            oracle_price,
            oracle_twap_5min
        );
    }

    Ok(too_divergent)
}

#[inline(always)]
pub fn should_expire_order_before_fill(
    user: &User,
    order_index: usize,
    now: i64,
) -> VortexDexResult<bool> {
    let should_order_be_expired = should_expire_order(user, order_index, now)?;
    if should_order_be_expired && user.orders[order_index].is_limit_order() {
        let now_sub_buffer = now.safe_sub(15)?;
        if !should_expire_order(user, order_index, now_sub_buffer)? {
            msg!("invalid fill. cant force expire limit order until 15s after max_ts. max ts {}, now {}, now plus buffer {}", user.orders[order_index].max_ts, now, now_sub_buffer);
            return Err(DexError::ImpossibleFill);
        }
    }

    Ok(should_order_be_expired)
}

pub fn determine_spot_fulfillment_methods(
    order: &Order,
    maker_orders_info: &[(Pubkey, usize, u64)],
    limit_price: Option<u64>,
    external_fulfillment_params_available: bool,
) -> VortexDexResult<Vec<SpotFulfillmentMethod>> {
    let mut fulfillment_methods = Vec::with_capacity(8);

    if !order.post_only && external_fulfillment_params_available {
        fulfillment_methods.push(SpotFulfillmentMethod::ExternalMarket);
        return Ok(fulfillment_methods);
    }

    let maker_direction = order.direction.opposite();

    for (maker_key, maker_order_index, maker_price) in maker_orders_info.iter() {
        let taker_crosses_maker = match limit_price {
            Some(taker_price) => do_orders_cross(maker_direction, *maker_price, taker_price),
            // todo come up with fallback price
            None => false,
        };

        if !taker_crosses_maker {
            break;
        }

        fulfillment_methods.push(SpotFulfillmentMethod::Match(
            *maker_key,
            *maker_order_index as u16,
        ));

        if fulfillment_methods.len() > 6 {
            break;
        }
    }

    Ok(fulfillment_methods)
}


pub fn validate_fill_price(
    quote_asset_amount: u64,
    base_asset_amount: u64,
    base_precision: u64,
    order_direction: PositionDirection,
    order_limit_price: u64,
    is_taker: bool,
) -> VortexDexResult {
    let rounded_quote_asset_amount = if is_taker {
        match order_direction {
            PositionDirection::Long => quote_asset_amount.saturating_sub(1),
            PositionDirection::Short => quote_asset_amount.saturating_add(1),
        }
    } else {
        quote_asset_amount
    };

    let fill_price = calculate_fill_price(
        rounded_quote_asset_amount,
        base_asset_amount,
        base_precision,
    )?;

    if order_direction == PositionDirection::Long && fill_price > order_limit_price {
        msg!(
            "long order fill price ({} = {}/{} * 1000) > limit price ({}) is_taker={}",
            fill_price,
            quote_asset_amount,
            base_asset_amount,
            order_limit_price,
            is_taker
        );
        return Err(DexError::InvalidOrderFillPrice);
    }

    if order_direction == PositionDirection::Short && fill_price < order_limit_price {
        msg!(
            "short order fill price ({} = {}/{} * 1000) < limit price ({}) is_taker={}",
            fill_price,
            quote_asset_amount,
            base_asset_amount,
            order_limit_price,
            is_taker
        );
        return Err(DexError::InvalidOrderFillPrice);
    }

    Ok(())
}


pub fn calculate_fill_price(
    quote_asset_amount: u64,
    base_asset_amount: u64,
    base_precision: u64,
) -> VortexDexResult<u64> {
    quote_asset_amount
        .cast::<u128>()?
        .safe_mul(base_precision as u128)?
        .safe_div(base_asset_amount.cast()?)?
        .cast::<u64>()
}

pub fn validate_fill_price_within_price_bands(
    fill_price: u64,
    direction: PositionDirection,
    oracle_price: i64,
    oracle_twap_5min: i64,
    margin_ratio_initial: u32,
    oracle_twap_5min_percent_divergence: u64,
) -> VortexDexResult {
    let oracle_price = oracle_price.unsigned_abs();
    let oracle_twap_5min = oracle_twap_5min.unsigned_abs();

    let max_oracle_diff = margin_ratio_initial.cast::<u128>()?;
    let max_oracle_twap_diff = oracle_twap_5min_percent_divergence.cast::<u128>()?; // 50%

    if direction == PositionDirection::Long {
        if fill_price < oracle_price && fill_price < oracle_twap_5min {
            return Ok(());
        }

        let percent_diff: u128 = fill_price
            .saturating_sub(oracle_price)
            .cast::<u128>()?
            .safe_mul(MARGIN_PRECISION_U128)?
            .safe_div(oracle_price.cast()?)?;

        validate!(
            percent_diff < max_oracle_diff,
            DexError::PriceBandsBreached,
            "Fill Price Breaches Oracle Price Bands: {} % <= {} % (fill: {} >= oracle: {})",
            max_oracle_diff,
            percent_diff,
            fill_price,
            oracle_price
        )?;

        let percent_diff = fill_price
            .saturating_sub(oracle_twap_5min)
            .cast::<u128>()?
            .safe_mul(PERCENTAGE_PRECISION)?
            .safe_div(oracle_twap_5min.cast()?)?;

        validate!(
            percent_diff < max_oracle_twap_diff,
            DexError::PriceBandsBreached,
            "Fill Price Breaches Oracle TWAP Price Bands:  {} % <= {} % (fill: {} >= twap: {})",
            max_oracle_twap_diff,
            percent_diff,
            fill_price,
            oracle_twap_5min
        )?;
    } else {
        if fill_price > oracle_price && fill_price > oracle_twap_5min {
            return Ok(());
        }

        let percent_diff: u128 = oracle_price
            .saturating_sub(fill_price)
            .cast::<u128>()?
            .safe_mul(MARGIN_PRECISION_U128)?
            .safe_div(oracle_price.cast()?)?;

        validate!(
            percent_diff < max_oracle_diff,
            DexError::PriceBandsBreached,
            "Fill Price Breaches Oracle Price Bands: {} % <= {} % (fill: {} <= oracle: {})",
            max_oracle_diff,
            percent_diff,
            fill_price,
            oracle_price
        )?;

        let percent_diff = oracle_twap_5min
            .saturating_sub(fill_price)
            .cast::<u128>()?
            .safe_mul(PERCENTAGE_PRECISION)?
            .safe_div(oracle_twap_5min.cast()?)?;

        validate!(
            percent_diff < max_oracle_twap_diff,
            DexError::PriceBandsBreached,
            "Fill Price Breaches Oracle TWAP Price Bands:  {} % <= {} % (fill: {} <= twap: {})",
            max_oracle_twap_diff,
            percent_diff,
            fill_price,
            oracle_twap_5min
        )?;
    }

    Ok(())
}

pub fn order_satisfies_trigger_condition(order: &Order, oracle_price: u64) -> VortexDexResult<bool> {
    match order.trigger_condition {
        OrderTriggerCondition::Above => Ok(oracle_price > order.trigger_price),
        OrderTriggerCondition::Below => Ok(oracle_price < order.trigger_price),
        _ => Err(print_error!(DexError::InvalidTriggerOrderCondition)()),
    }
}