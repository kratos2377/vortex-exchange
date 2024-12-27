use std::{cell::RefMut, collections::BTreeMap, ops::DerefMut};

use anchor_lang::prelude::*;

use crate::{casting::Cast, errors::{DexError, VortexDexResult}, get_struct_values, get_then_update_id, load_mut, print_error, safe_methods::{SafeMath, SafeUnwrap}, state::{dex_state::{DexState, ExchangeStatus, FeeStructure},
  events::{emit_stack, get_order_action_record, OrderAction, 
    OrderActionExplanation, OrderActionRecord, OrderRecord}, 
    fulfillment::SpotFulfillmentMethod, margin_calculation::{MarginCalculation, MarginContext}, operations::SpotOperation, oracle::{OraclePriceData, StrictOraclePrice}, oracle_map::OracleMap, order_params::{ModifyOrderParams, ModifyOrderPolicy, OrderParams, PlaceOrderOptions, PostOnlyParam}, position::PositionDirection, spot_fulfillment_params::{ExternalSpotFill, SpotFulfillmentParams}, spot_market::{MarketStatus, SpotBalanceType, SpotMarket}, spot_market_map::SpotMarketMap, user::{AssetType, MarketType, Order, OrderStatus, OrderTriggerCondition, OrderType, User}, user_map::{UserMap, UserStatsMap}, user_stats::UserStats}, utils::{auction_utils::{calculate_auction_params_for_trigger_order, calculate_auction_prices}, constants::QUOTE_SPOT_MARKET_INDEX, fees_utils::{self, FillFees}, fuel_utils::ExternalFillFees, liquidation_utils::validate_user_not_being_liquidated, margin_utils::{calculate_margin_requirement_and_total_collateral_and_liability_info, meets_initial_margin_requirement, meets_place_order_margin_requirement, validate_spot_margin_trading, MarginRequirementType}, matching_utils::{are_orders_same_market_but_different_sides, calculate_fill_for_matched_orders, calculate_filler_multiplier_for_matched_orders, do_orders_cross, is_maker_for_taker}, oracle_utils::{is_oracle_valid_for_action, VortexDexAction}, order_utils::{calculate_fill_price, calculate_max_spot_order_size, determine_spot_fulfillment_methods, find_maker_orders, get_max_fill_amounts, is_multiple_of_step_size, is_new_order_risk_increasing, is_oracle_too_divergent_with_twap_5min, is_order_position_reducing, limit_price_breaches_maker_oracle_price_bands, order_satisfies_trigger_condition, should_cancel_reduce_only_order, should_expire_order, should_expire_order_before_fill, standardize_base_asset_amount, standardize_price, standardize_price_i64, validate_fill_price, validate_fill_price_within_price_bands, validate_order_for_force_reduce_only, validate_spot_order}, spot_market_utils::{get_signed_token_amount, get_token_amount, select_margin_type_for_swap}}, validate};

use super::{spot_balance::{update_spot_balances, update_spot_market_cumulative_interest}, spot_position::{decrease_spot_open_bids_and_asks, increase_spot_open_bids_and_asks, update_spot_balances_and_cumulative_deposits}};


pub fn cancel_order(
    order_index: usize,
    user: &mut User,
    user_key: &Pubkey,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    now: i64,
    _slot: u64,
    explanation: OrderActionExplanation,
    filler_key: Option<&Pubkey>,
    filler_reward: u64,
    skip_log: bool,
) -> VortexDexResult {
    let (order_status, order_market_index, order_direction, order_market_type) = get_struct_values!(
        user.orders[order_index],
        status,
        market_index,
        direction,
        market_type
    );


    validate!(order_status == OrderStatus::Open, DexError::OrderNotOpen)?;

    let oracle =  spot_market_map.get_ref(&order_market_index)?.oracle;
    

    if !skip_log {
        let (taker, taker_order, maker, maker_order) =
            get_taker_and_maker_for_order_record(user_key, &user.orders[order_index]);

        let order_action_record = get_order_action_record(
            now,
            OrderAction::Cancel,
            explanation,
            order_market_index,
            filler_key.copied(),
            None,
            Some(filler_reward),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            taker,
            taker_order,
            maker,
            maker_order,
            oracle_map.get_price_data(&oracle)?.price,
        )?;
        emit_stack::<_, { OrderActionRecord::SIZE }>(order_action_record)?;
    }

    user.decrement_open_orders(user.orders[order_index].has_auction());

    let spot_position_index = user.get_spot_position_index(order_market_index)?;

    // only decrease open/bids ask if it's not a trigger order or if it's been triggered
    if !user.orders[order_index].must_be_triggered() || user.orders[order_index].triggered() {
        let base_asset_amount_unfilled =
            user.orders[order_index].get_base_asset_amount_unfilled(None)?;
        decrease_spot_open_bids_and_asks(
            &mut user.spot_positions[spot_position_index],
            &order_direction,
            base_asset_amount_unfilled,
        )?;
    }
    user.spot_positions[spot_position_index].open_orders -= 1;
    user.orders[order_index] = Order::default();
    

    Ok(())
}


pub fn cancel_orders(
    user: &mut User,
    user_key: &Pubkey,
    filler_key: Option<&Pubkey>,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    now: i64,
    slot: u64,
    explanation: OrderActionExplanation,
    market_type: Option<MarketType>,
    market_index: Option<u16>,
    direction: Option<PositionDirection>,
) -> VortexDexResult<Vec<u32>> {
    let mut canceled_order_ids: Vec<u32> = vec![];
    for order_index in 0..user.orders.len() {
        if user.orders[order_index].status != OrderStatus::Open {
            continue;
        }

        if let (Some(market_type), Some(market_index)) = (market_type, market_index) {
            if user.orders[order_index].market_type != market_type {
                continue;
            }

            if user.orders[order_index].market_index != market_index {
                continue;
            }
        }

        if let Some(direction) = direction {
            if user.orders[order_index].direction != direction {
                continue;
            }
        }

        canceled_order_ids.push(user.orders[order_index].order_id);
        cancel_order(
            order_index,
            user,
            user_key,
            spot_market_map,
            oracle_map,
            now,
            slot,
            explanation,
            filler_key,
            0,
            false,
        )?;
    }

    user.update_last_active_slot(slot);

    Ok(canceled_order_ids)
}

pub fn cancel_order_by_order_id(
    order_id: u32,
    user: &AccountLoader<User>,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    clock: &Clock,
) -> VortexDexResult {
    let user_key = user.key();
    let user = &mut load_mut!(user)?;
    let order_index = match user.get_order_index(order_id) {
        Ok(order_index) => order_index,
        Err(_) => {
            msg!("could not find order id {}", order_id);
            return Ok(());
        }
    };

    cancel_order(
        order_index,
        user,
        &user_key,
        spot_market_map,
        oracle_map,
        clock.unix_timestamp,
        clock.slot,
        OrderActionExplanation::None,
        None,
        0,
        false,
    )?;

    user.update_last_active_slot(clock.slot);

    Ok(())
}


pub fn cancel_order_by_user_order_id(
    user_order_id: u8,
    user: &AccountLoader<User>,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    clock: &Clock,
) -> VortexDexResult {
    let user_key = user.key();
    let user = &mut load_mut!(user)?;
    let order_index = match user
        .orders
        .iter()
        .position(|order| order.user_order_id == user_order_id)
    {
        Some(order_index) => order_index,
        None => {
            msg!("could not find user order id {}", user_order_id);
            return Ok(());
        }
    };

    cancel_order(
        order_index,
        user,
        &user_key,
        spot_market_map,
        oracle_map,
        clock.unix_timestamp,
        clock.slot,
        OrderActionExplanation::None,
        None,
        0,
        false,
    )?;

    user.update_last_active_slot(clock.slot);

    Ok(())
}

#[allow(clippy::type_complexity)]
fn get_taker_and_maker_for_order_record(
    user_key: &Pubkey,
    user_order: &Order,
) -> (Option<Pubkey>, Option<Order>, Option<Pubkey>, Option<Order>) {
    if user_order.post_only {
        (None, None, Some(*user_key), Some(*user_order))
    } else {
        (Some(*user_key), Some(*user_order), None, None)
    }
}


pub enum ModifyOrderId {
    UserOrderId(u8),
    OrderId(u32),
}

pub fn modify_order(
    order_id: ModifyOrderId,
    modify_order_params: ModifyOrderParams,
    user_loader: &AccountLoader<User>,
    state: &DexState,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    clock: &Clock,
) -> VortexDexResult {
    let user_key = user_loader.key();
    let mut user = load_mut!(user_loader)?;

    let order_index = match order_id {
        ModifyOrderId::UserOrderId(user_order_id) => {
            match user.get_order_index_by_user_order_id(user_order_id) {
                Ok(order_index) => order_index,
                Err(e) => {
                    msg!("User order id {} not found", user_order_id);
                    if modify_order_params.policy == Some(ModifyOrderPolicy::MustModify) {
                        return Err(e);
                    } else {
                        return Ok(());
                    }
                }
            }
        }
        ModifyOrderId::OrderId(order_id) => match user.get_order_index(order_id) {
            Ok(order_index) => order_index,
            Err(e) => {
                msg!("Order id {} not found", order_id);
                if modify_order_params.policy == Some(ModifyOrderPolicy::MustModify) {
                    return Err(e);
                } else {
                    return Ok(());
                }
            }
        },
    };

    let existing_order = user.orders[order_index];

    cancel_order(
        order_index,
        &mut user,
        &user_key,
        spot_market_map,
        oracle_map,
        clock.unix_timestamp,
        clock.slot,
        OrderActionExplanation::None,
        None,
        0,
        false,
    )?;

    user.update_last_active_slot(clock.slot);

    let order_params =
        merge_modify_order_params_with_existing_order(&existing_order, &modify_order_params)?;


        place_spot_order(
            state,
            &mut user,
            user_key,
            spot_market_map,
            oracle_map,
            clock,
            order_params,
            PlaceOrderOptions::default(),
        )?;
    

    Ok(())
}


fn merge_modify_order_params_with_existing_order(
    existing_order: &Order,
    modify_order_params: &ModifyOrderParams,
) -> VortexDexResult<OrderParams> {
    let order_type = existing_order.order_type;
    let market_type = existing_order.market_type;
    let direction = modify_order_params
        .direction
        .unwrap_or(existing_order.direction);
    let user_order_id = existing_order.user_order_id;
    let base_asset_amount = modify_order_params
        .base_asset_amount
        .unwrap_or(existing_order.get_base_asset_amount_unfilled(None)?);
    let price = modify_order_params.price.unwrap_or(existing_order.price);
    let market_index = existing_order.market_index;
    let reduce_only = modify_order_params
        .reduce_only
        .unwrap_or(existing_order.reduce_only);
    let post_only = modify_order_params
        .post_only
        .unwrap_or(if existing_order.post_only {
            PostOnlyParam::MustPostOnly
        } else {
            PostOnlyParam::None
        });
    let immediate_or_cancel = false;
    let max_ts = modify_order_params.max_ts.or(Some(existing_order.max_ts));
    let trigger_price = modify_order_params
        .trigger_price
        .or(Some(existing_order.trigger_price));
    let trigger_condition =
        modify_order_params
            .trigger_condition
            .unwrap_or(match existing_order.trigger_condition {
                OrderTriggerCondition::TriggeredAbove | OrderTriggerCondition::Above => {
                    OrderTriggerCondition::Above
                }
                OrderTriggerCondition::TriggeredBelow | OrderTriggerCondition::Below => {
                    OrderTriggerCondition::Below
                }
            });
    let oracle_price_offset = modify_order_params
        .oracle_price_offset
        .or(Some(existing_order.oracle_price_offset));
    let (auction_duration, auction_start_price, auction_end_price) =
        if modify_order_params.auction_duration.is_some()
            && modify_order_params.auction_start_price.is_some()
            && modify_order_params.auction_end_price.is_some()
        {
            (
                modify_order_params.auction_duration,
                modify_order_params.auction_start_price,
                modify_order_params.auction_end_price,
            )
        } else {
            (None, None, None)
        };

    Ok(OrderParams {
        order_type,
        market_type,
        direction,
        user_order_id,
        base_asset_amount,
        price,
        market_index,
        reduce_only,
        post_only,
        immediate_or_cancel,
        max_ts,
        trigger_price,
        trigger_condition,
        oracle_price_offset,
        auction_duration,
        auction_start_price,
        auction_end_price,
    })
}


pub fn place_spot_order(
    state: &DexState,
    user: &mut User,
    user_key: Pubkey,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    clock: &Clock,
    params: OrderParams,
    mut options: PlaceOrderOptions,
) -> VortexDexResult {
    let now = clock.unix_timestamp;
    let slot = clock.slot;

    validate_user_not_being_liquidated(
        user,
        spot_market_map,
        oracle_map,
        state.liquidation_margin_buffer_ratio,
    )?;

    validate!(!user.is_bankrupt(), DexError::UserBankrupt)?;

    if options.try_expire_orders {
        expire_orders(
            user,
            &user_key,
            spot_market_map,
            oracle_map,
            now,
            slot,
        )?;
    }

    if user.is_reduce_only() {
        validate!(
            params.reduce_only,
            DexError::UserReduceOnly,
            "order must be reduce only"
        )?;
    }

    let max_ts = match params.max_ts {
        Some(max_ts) => max_ts,
        None => match params.order_type {
            OrderType::Market | OrderType::Oracle => now.safe_add(30)?,
            _ => 0_i64,
        },
    };

    if max_ts != 0 && max_ts < now {
        msg!("max_ts ({}) < now ({}), skipping order", max_ts, now);
        return Ok(());
    }

    let new_order_index = user
        .orders
        .iter()
        .position(|order| order.status.eq(&OrderStatus::Init))
        .ok_or(DexError::MaxNumberOfOrders)?;

    if params.user_order_id > 0 {
        let user_order_id_already_used = user
            .orders
            .iter()
            .position(|order| order.user_order_id == params.user_order_id);

        if user_order_id_already_used.is_some() {
            msg!("user_order_id is already in use {}", params.user_order_id);
            return Err(DexError::UserOrderIdAlreadyInUse);
        }
    }

    let market_index = params.market_index;
    let spot_market = &spot_market_map.get_ref(&market_index)?;
    let force_reduce_only = spot_market.is_reduce_only();
    let step_size = spot_market.order_step_size;

    validate!(
        !matches!(spot_market.status, MarketStatus::Initialized),
        DexError::MarketBeingInitialized,
        "Market is being initialized"
    )?;

    let spot_position_index = user
        .get_spot_position_index(market_index)
        .or_else(|_| user.add_spot_position(market_index, SpotBalanceType::Deposit))?;

    let balance_type = user.spot_positions[spot_position_index].balance_type;
    let token_amount = user.spot_positions[spot_position_index].get_token_amount(spot_market)?;
    let signed_token_amount = get_signed_token_amount(token_amount, &balance_type)?;

    let oracle_price_data = *oracle_map.get_price_data(&spot_market.oracle)?;

    // Increment open orders for existing position
    let (existing_position_direction, order_base_asset_amount) = {
        validate!(
            params.base_asset_amount >= step_size,
            DexError::InvalidOrderSizeTooSmall,
            "params.base_asset_amount={} cannot be below spot_market.order_step_size={}",
            params.base_asset_amount,
            step_size
        )?;

        let base_asset_amount = if params.base_asset_amount == u64::MAX {
            calculate_max_spot_order_size(
                user,
                params.market_index,
                params.direction,
                spot_market_map,
                oracle_map,
            )?
        } else {
            standardize_base_asset_amount(params.base_asset_amount, step_size)?
        };

        validate!(
            is_multiple_of_step_size(base_asset_amount, step_size)?,
            DexError::InvalidOrderNotStepSizeMultiple,
            "Order base asset amount ({}), is not a multiple of step size ({})",
            base_asset_amount,
            step_size
        )?;

        let existing_position_direction = if signed_token_amount >= 0 {
            PositionDirection::Long
        } else {
            PositionDirection::Short
        };
        (
            existing_position_direction,
            base_asset_amount.cast::<u64>()?,
        )
    };

    let (auction_start_price, auction_end_price, auction_duration) = get_auction_params(
        &params,
        &oracle_price_data,
        spot_market.order_tick_size,
        state.default_spot_auction_duration,
    )?;

    validate!(spot_market.orders_enabled, DexError::SpotOrdersDisabled)?;

    validate!(
        params.market_index != QUOTE_SPOT_MARKET_INDEX,
        DexError::InvalidOrderBaseQuoteAsset,
        "can not place order for quote asset"
    )?;

    validate!(
        params.market_type == MarketType::Spot,
        DexError::InvalidOrderMarketType,
        "must be spot order"
    )?;

    let new_order = Order {
        status: OrderStatus::Open,
        order_type: params.order_type,
        market_type: params.market_type,
        slot,
        order_id: get_then_update_id!(user, next_order_id),
        user_order_id: params.user_order_id,
        market_index: params.market_index,
        price: standardize_price(params.price, spot_market.order_tick_size, params.direction)?,
        existing_position_direction,
        base_asset_amount: order_base_asset_amount,
        base_asset_amount_filled: 0,
        quote_asset_amount_filled: 0,
        direction: params.direction,
        reduce_only: params.reduce_only || force_reduce_only,
        trigger_price: standardize_price(
            params.trigger_price.unwrap_or(0),
            spot_market.order_tick_size,
            params.direction,
        )?,
        trigger_condition: params.trigger_condition,
        post_only: params.post_only != PostOnlyParam::None,
        oracle_price_offset: params.oracle_price_offset.unwrap_or(0),
        immediate_or_cancel: params.immediate_or_cancel,
        auction_start_price,
        auction_end_price,
        auction_duration,
        max_ts,
        padding: [0; 3],
    };

    validate_spot_order(
        &new_order,
        spot_market.order_step_size,
        spot_market.min_order_size,
    )?;

    let risk_increasing = is_new_order_risk_increasing(
        &new_order,
        signed_token_amount.cast()?,
        user.spot_positions[spot_position_index].open_bids,
        user.spot_positions[spot_position_index].open_asks,
    )?;

    user.increment_open_orders(new_order.has_auction());
    user.orders[new_order_index] = new_order;
    user.spot_positions[spot_position_index].open_orders += 1;
    if !new_order.must_be_triggered() {
        increase_spot_open_bids_and_asks(
            &mut user.spot_positions[spot_position_index],
            &params.direction,
            order_base_asset_amount,
        )?;
    }

    options.update_risk_increasing(risk_increasing);

    if options.enforce_margin_check {
        meets_place_order_margin_requirement(
            user,
            spot_market_map,
            oracle_map,
            options.risk_increasing,
        )?;
    }

    validate_spot_margin_trading(user, spot_market_map, oracle_map)?;

    if force_reduce_only {
        validate_order_for_force_reduce_only(
            &user.orders[new_order_index],
            signed_token_amount.cast()?,
        )?;
    }

    let (taker, taker_order, maker, maker_order) =
        get_taker_and_maker_for_order_record(&user_key, &new_order);

    let order_action_record = get_order_action_record(
        now,
        OrderAction::Place,
        OrderActionExplanation::None,
        params.market_index,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        taker,
        taker_order,
        maker,
        maker_order,
        oracle_price_data.price,
    )?;
    emit_stack::<_, { OrderActionRecord::SIZE }>(order_action_record)?;

    let order_record = OrderRecord {
        ts: now,
        user: user_key,
        order: user.orders[new_order_index],
    };
    emit_stack::<_, { OrderRecord::SIZE }>(order_record)?;

    user.update_last_active_slot(slot);

    Ok(())
}

pub fn expire_orders(
    user: &mut User,
    user_key: &Pubkey,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    now: i64,
    slot: u64,
) -> VortexDexResult {
    for order_index in 0..user.orders.len() {
        if !should_expire_order(user, order_index, now)? {
            continue;
        }

        cancel_order(
            order_index,
            user,
            user_key,
            spot_market_map,
            oracle_map,
            now,
            slot,
            OrderActionExplanation::OrderExpired,
            None,
            0,
            false,
        )?;
    }

    Ok(())
}


fn get_auction_params(
    params: &OrderParams,
    oracle_price_data: &OraclePriceData,
    tick_size: u64,
    min_auction_duration: u8,
) -> VortexDexResult<(i64, i64, u8)> {
    if !matches!(
        params.order_type,
        OrderType::Market | OrderType::Oracle | OrderType::Limit
    ) {
        return Ok((0_i64, 0_i64, 0_u8));
    }

    if params.order_type == OrderType::Limit {
        return match (
            params.auction_start_price,
            params.auction_end_price,
            params.auction_duration,
        ) {
            (Some(auction_start_price), Some(auction_end_price), Some(auction_duration)) => {
                let auction_duration = if auction_duration == 0 {
                    auction_duration
                } else {
                    // if auction is non-zero, force it to be at least min_auction_duration
                    auction_duration.max(min_auction_duration)
                };

                Ok((
                    standardize_price_i64(
                        auction_start_price,
                        tick_size.cast()?,
                        params.direction,
                    )?,
                    standardize_price_i64(auction_end_price, tick_size.cast()?, params.direction)?,
                    auction_duration,
                ))
            }
            _ => Ok((0_i64, 0_i64, 0_u8)),
        };
    }

    let auction_duration = params
        .auction_duration
        .unwrap_or(0)
        .max(min_auction_duration);

    let (auction_start_price, auction_end_price) =
        match (params.auction_start_price, params.auction_end_price) {
            (Some(auction_start_price), Some(auction_end_price)) => {
                (auction_start_price, auction_end_price)
            }
            _ if params.order_type == OrderType::Oracle => {
                msg!("Oracle order must specify auction start and end price offsets");
                return Err(DexError::InvalidOrderAuction);
            }
            _ => calculate_auction_prices(oracle_price_data, params.direction, params.price)?,
        };

    Ok((
        standardize_price_i64(auction_start_price, tick_size.cast()?, params.direction)?,
        standardize_price_i64(auction_end_price, tick_size.cast()?, params.direction)?,
        auction_duration,
    ))
}


pub fn fill_spot_order(
    order_id: u32,
    state: &DexState,
    user: &AccountLoader<User>,
    user_stats: &AccountLoader<UserStats>,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    filler: &AccountLoader<User>,
    filler_stats: &AccountLoader<UserStats>,
    makers_and_referrer: &UserMap,
    makers_and_referrer_stats: &UserStatsMap,
    jit_maker_order_id: Option<u32>,
    clock: &Clock,
    fulfillment_params: &mut dyn SpotFulfillmentParams,
) -> VortexDexResult<u64> {
    let now = clock.unix_timestamp;
    let slot = clock.slot;

    let filler_key = filler.key();
    let user_key = user.key();
    let user = &mut load_mut!(user)?;
    let user_stats = &mut load_mut!(user_stats)?;

    let order_index = user
        .orders
        .iter()
        .position(|order| order.order_id == order_id)
        .ok_or_else(print_error!(DexError::OrderDoesNotExist))?;

    let (order_status, order_market_index, order_market_type, order_direction) = get_struct_values!(
        user.orders[order_index],
        status,
        market_index,
        market_type,
        direction
    );

    {
        let spot_market = spot_market_map.get_ref(&order_market_index)?;
        validate!(
            spot_market.fills_enabled(),
            DexError::MarketFillOrderPaused,
            "Market unavailable for fills"
        )?;
    }

    validate!(
        order_market_type == MarketType::Spot,
        DexError::InvalidOrderMarketType,
        "must be spot order"
    )?;

    validate!(
        order_status == OrderStatus::Open,
        DexError::OrderNotOpen,
        "Order not open"
    )?;

    validate!(
        !user.orders[order_index].must_be_triggered() || user.orders[order_index].triggered(),
        DexError::OrderMustBeTriggeredFirst,
        "Order must be triggered first"
    )?;

    if user.is_bankrupt() {
        msg!("User is bankrupt");
        return Ok(0);
    }

    match validate_user_not_being_liquidated(
        user,
        spot_market_map,
        oracle_map,
        state.liquidation_margin_buffer_ratio,
    ) {
        Ok(_) => {}
        Err(_) => {
            msg!("User is being liquidated");
            return Ok(0);
        }
    }

    let is_filler_taker = user_key == filler_key;
    let is_filler_maker = makers_and_referrer.0.contains_key(&filler_key);
    let (mut filler, mut filler_stats) = if !is_filler_maker && !is_filler_taker {
        let filler = load_mut!(filler)?;
        if filler.authority != user.authority {
            (Some(filler), Some(load_mut!(filler_stats)?))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let oracle_price = oracle_map
        .get_price_data(&spot_market_map.get_ref_mut(&order_market_index)?.oracle)?
        .price;
    let maker_order_info = get_spot_maker_orders_info(
        spot_market_map,
        oracle_map,
        makers_and_referrer,
        &user_key,
        &user.orders[order_index],
        &mut filler.as_deref_mut(),
        &filler_key,
        state.spot_fee_structure.flat_filler_fee,
        oracle_price,
        jit_maker_order_id,
        now,
        slot,
    )?;

    {
        let mut quote_market = spot_market_map.get_quote_spot_market_mut()?;
        let oracle_price_data = oracle_map.get_price_data(&quote_market.oracle)?;
        update_spot_market_cumulative_interest(&mut quote_market, Some(oracle_price_data), now)?;

        let mut base_market = spot_market_map.get_ref_mut(&order_market_index)?;
        let oracle_price_data = oracle_map.get_price_data(&base_market.oracle)?;
        update_spot_market_cumulative_interest(&mut base_market, Some(oracle_price_data), now)?;

        let oracle_too_divergent_with_twap_5min = is_oracle_too_divergent_with_twap_5min(
            oracle_price_data.price,
            base_market
                .historical_oracle_data
                .last_oracle_price_twap_5min,
            state
                .oracle_guard_rails
                .max_oracle_twap_5min_percent_divergence()
                .cast()?,
        )?;

        if oracle_too_divergent_with_twap_5min {
            // update filler last active so tx doesn't revert
            if let Some(filler) = filler.as_mut() {
                filler.update_last_active_slot(slot);
            }

            return Ok(0);
        }
    }

    let should_expire_order = should_expire_order_before_fill(user, order_index, now)?;

    let should_cancel_reduce_only = if user.orders[order_index].reduce_only {
        let market_index = user.orders[order_index].market_index;
        let position_index = user.get_spot_position_index(market_index)?;
        let spot_market = spot_market_map.get_ref(&market_index)?;
        let signed_token_amount =
            user.spot_positions[position_index].get_signed_token_amount(&spot_market)?;
        should_cancel_reduce_only_order(
            &user.orders[order_index],
            signed_token_amount.cast()?,
            spot_market.order_step_size,
        )?
    } else {
        false
    };

    if should_expire_order || should_cancel_reduce_only {
        let filler_reward = {
            let mut quote_market = spot_market_map.get_quote_spot_market_mut()?;
            pay_keeper_flat_reward_for_spot(
                user,
                filler.as_deref_mut(),
                &mut quote_market,
                state.spot_fee_structure.flat_filler_fee,
                slot,
            )?
        };

        let explanation = if should_expire_order {
            OrderActionExplanation::OrderExpired
        } else {
            OrderActionExplanation::ReduceOnlyOrderIncreasedPosition
        };

        cancel_order(
            order_index,
            user,
            &user_key,
            spot_market_map,
            oracle_map,
            now,
            slot,
            explanation,
            Some(&filler_key),
            filler_reward,
            false,
        )?;
        return Ok(0);
    }

    if fulfillment_params.is_external() {
        let exchange_status = state.get_exchange_status()?;

        validate!(
            !exchange_status
                .contains(ExchangeStatus::DepositPaused | ExchangeStatus::WithdrawPaused),
            DexError::ExchangePaused
        )?;
    }

    let (base_asset_amount, quote_asset_amount) = fulfill_spot_order(
        user,
        order_index,
        &user_key,
        user_stats,
        makers_and_referrer,
        makers_and_referrer_stats,
        &maker_order_info,
        &mut filler.as_deref_mut(),
        &filler_key,
        &mut filler_stats.as_deref_mut(),
        spot_market_map,
        oracle_map,
        now,
        slot,
        &state.spot_fee_structure,
        fulfillment_params,
    )?;

    if base_asset_amount != 0 {
        let spot_market = spot_market_map.get_ref(&order_market_index)?;
        let fill_price = calculate_fill_price(
            quote_asset_amount,
            base_asset_amount,
            spot_market.get_precision(),
        )?;

        let oracle_price = oracle_map.get_price_data(&spot_market.oracle)?.price;
        let oracle_twap_5min = spot_market
            .historical_oracle_data
            .last_oracle_price_twap_5min;
        validate_fill_price_within_price_bands(
            fill_price,
            order_direction,
            oracle_price,
            oracle_twap_5min,
            spot_market.get_margin_ratio(&MarginRequirementType::Initial)?,
            state
                .oracle_guard_rails
                .max_oracle_twap_5min_percent_divergence(),
        )?;
    }

    let is_open = user.orders[order_index].status == OrderStatus::Open;
    let is_reduce_only = user.orders[order_index].reduce_only;
    let should_cancel_reduce_only = if is_open && is_reduce_only {
        let market_index = user.orders[order_index].market_index;
        let position_index = user.get_spot_position_index(market_index)?;
        let spot_market = spot_market_map.get_ref(&market_index)?;
        let signed_token_amount =
            user.spot_positions[position_index].get_signed_token_amount(&spot_market)?;
        should_cancel_reduce_only_order(
            &user.orders[order_index],
            signed_token_amount.cast()?,
            spot_market.order_step_size,
        )?
    } else {
        false
    };

    let should_cancel_for_no_borrow_liquidity = if is_open {
        let market_index = user.orders[order_index].market_index;
        let base_market = spot_market_map.get_ref(&market_index)?;
        let quote_market = spot_market_map.get_quote_spot_market()?;
        let (max_base_asset_amount, max_quote_asset_amount) =
            get_max_fill_amounts(user, order_index, &base_market, &quote_market, false)?;
        max_base_asset_amount == Some(0) || max_quote_asset_amount == Some(0)
    } else {
        false
    };

    if should_cancel_reduce_only || should_cancel_for_no_borrow_liquidity {
        let filler_reward = {
            let mut quote_market = spot_market_map.get_quote_spot_market_mut()?;
            pay_keeper_flat_reward_for_spot(
                user,
                filler.as_deref_mut(),
                &mut quote_market,
                state.spot_fee_structure.flat_filler_fee,
                slot,
            )?
        };

        let explanation = if should_cancel_reduce_only {
            OrderActionExplanation::ReduceOnlyOrderIncreasedPosition
        } else {
            OrderActionExplanation::NoBorrowLiquidity
        };

        cancel_order(
            order_index,
            user,
            &user_key,
            spot_market_map,
            oracle_map,
            now,
            slot,
            explanation,
            Some(&filler_key),
            filler_reward,
            false,
        )?
    }

    spot_market_map
        .get_ref(&order_market_index)?
        .validate_max_token_deposits_and_borrows(false)?;

    user.update_last_active_slot(slot);

    Ok(base_asset_amount)
}


pub fn pay_keeper_flat_reward_for_spot(
    user: &mut User,
    filler: Option<&mut User>,
    quote_market: &mut SpotMarket,
    filler_reward: u64,
    slot: u64,
) -> VortexDexResult<u64> {
    let filler_reward = if let Some(filler) = filler {
        update_spot_balances(
            filler_reward as u128,
            &SpotBalanceType::Deposit,
            quote_market,
            filler.get_quote_spot_position_mut(),
            false,
        )?;

        filler.update_last_active_slot(slot);

        filler.update_cumulative_spot_fees(filler_reward.cast()?)?;

        update_spot_balances(
            filler_reward as u128,
            &SpotBalanceType::Borrow,
            quote_market,
            user.get_quote_spot_position_mut(),
            false,
        )?;

        user.update_cumulative_spot_fees(-filler_reward.cast()?)?;

        filler_reward
    } else {
        0
    };

    Ok(filler_reward)
}


#[allow(clippy::type_complexity)]
fn get_spot_maker_orders_info(
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    makers_and_referrer: &UserMap,
    taker_key: &Pubkey,
    taker_order: &Order,
    filler: &mut Option<&mut User>,
    filler_key: &Pubkey,
    filler_reward: u64,
    oracle_price: i64,
    jit_maker_order_id: Option<u32>,
    now: i64,
    slot: u64,
) -> VortexDexResult<Vec<(Pubkey, usize, u64)>> {
    let maker_direction = taker_order.direction.opposite();

    let mut maker_orders_info = Vec::with_capacity(16);

    for (maker_key, user_account_loader) in makers_and_referrer.0.iter() {
        if maker_key == taker_key {
            continue;
        }

        let mut maker = load_mut!(user_account_loader)?;

        if maker.is_being_liquidated() || maker.is_bankrupt() {
            continue;
        }

        let market = spot_market_map.get_ref_mut(&taker_order.market_index)?;
        let maker_order_price_and_indexes = find_maker_orders(
            &maker,
            &maker_direction,
            &MarketType::Spot,
            taker_order.market_index,
            Some(oracle_price),
            slot,
            market.order_tick_size,
        )?;

        if maker_order_price_and_indexes.is_empty() {
            continue;
        }

        maker.update_last_active_slot(slot);

        let initial_margin_ratio = market.get_margin_ratio(&MarginRequirementType::Initial)?;
        let step_size = market.order_step_size;

        let existing_base_asset_amount = maker
            .get_spot_position(taker_order.market_index)?
            .get_signed_token_amount(&market)?;

        drop(market);

        for (maker_order_index, maker_order_price) in maker_order_price_and_indexes.iter() {
            let maker_order_index = *maker_order_index;
            let maker_order_price = *maker_order_price;

            let maker_order = &maker.orders[maker_order_index];
            if !is_maker_for_taker(maker_order, taker_order, slot)? {
                continue;
            }

            if !are_orders_same_market_but_different_sides(maker_order, taker_order) {
                continue;
            }

            if let Some(jit_maker_order_id) = jit_maker_order_id {
                // if jit maker order id exists, must only use that order
                if maker_order.order_id != jit_maker_order_id {
                    continue;
                }
            }

            let breaches_oracle_price_limits = {
                limit_price_breaches_maker_oracle_price_bands(
                    maker_order_price,
                    maker_order.direction,
                    oracle_price,
                    initial_margin_ratio,
                )?
            };

            let should_expire_order = should_expire_order(&maker, maker_order_index, now)?;

            let should_cancel_reduce_only_order = should_cancel_reduce_only_order(
                &maker.orders[maker_order_index],
                existing_base_asset_amount.cast()?,
                step_size,
            )?;

            if breaches_oracle_price_limits
                || should_expire_order
                || should_cancel_reduce_only_order
            {
                let filler_reward = {
                    pay_keeper_flat_reward_for_spot(
                        &mut maker,
                        filler.as_deref_mut(),
                        spot_market_map.get_quote_spot_market_mut()?.deref_mut(),
                        filler_reward,
                        slot,
                    )?
                };

                let explanation = if breaches_oracle_price_limits {
                    OrderActionExplanation::OraclePriceBreachedLimitPrice
                } else if should_expire_order {
                    OrderActionExplanation::OrderExpired
                } else {
                    OrderActionExplanation::ReduceOnlyOrderIncreasedPosition
                };

                cancel_order(
                    maker_order_index,
                    maker.deref_mut(),
                    maker_key,
                    spot_market_map,
                    oracle_map,
                    now,
                    slot,
                    explanation,
                    Some(filler_key),
                    filler_reward,
                    false,
                )?;

                continue;
            }

            insert_maker_order_info(
                &mut maker_orders_info,
                (*maker_key, maker_order_index, maker_order_price),
                maker_direction,
            );
        }
    }

    Ok(maker_orders_info)
}


#[inline(always)]
fn insert_maker_order_info(
    maker_orders_info: &mut Vec<(Pubkey, usize, u64)>,
    maker_order_info: (Pubkey, usize, u64),
    direction: PositionDirection,
) {
    let price = maker_order_info.2;
    let index = match maker_orders_info.binary_search_by(|item| match direction {
        PositionDirection::Short => item.2.cmp(&price),
        PositionDirection::Long => price.cmp(&item.2),
    }) {
        Ok(index) => index,
        Err(index) => index,
    };

    if index < maker_orders_info.capacity() {
        maker_orders_info.insert(index, maker_order_info);
    }
}


fn fulfill_spot_order(
    user: &mut User,
    user_order_index: usize,
    user_key: &Pubkey,
    user_stats: &mut UserStats,
    makers_and_referrer: &UserMap,
    makers_and_referrer_stats: &UserStatsMap,
    maker_orders_info: &[(Pubkey, usize, u64)],
    filler: &mut Option<&mut User>,
    filler_key: &Pubkey,
    filler_stats: &mut Option<&mut UserStats>,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    now: i64,
    slot: u64,
    fee_structure: &FeeStructure,
    fulfillment_params: &mut dyn SpotFulfillmentParams,
) -> VortexDexResult<(u64, u64)> {
    let base_market_index = user.orders[user_order_index].market_index;
    let order_direction = user.orders[user_order_index].direction;

    let mut quote_market = spot_market_map.get_quote_spot_market_mut()?;
    let mut base_market = spot_market_map.get_ref_mut(&base_market_index)?;

    if fulfillment_params.is_external() {
        if order_direction == PositionDirection::Long {
            validate!(
                !quote_market.is_operation_paused(SpotOperation::Withdraw),
                DexError::MarketFillOrderPaused,
                "withdraw from quote market paused"
            )?;

            validate!(
                !base_market.is_operation_paused(SpotOperation::Deposit),
                DexError::MarketFillOrderPaused,
                "deposit to base market paused"
            )?;
        } else {
            validate!(
                !quote_market.is_operation_paused(SpotOperation::Deposit),
                DexError::MarketFillOrderPaused,
                "deposit to quote market paused"
            )?;

            validate!(
                !base_market.is_operation_paused(SpotOperation::Withdraw),
                DexError::MarketFillOrderPaused,
                "withdraw from base market paused"
            )?;
        }
    }

    let quote_token_amount_before = user
        .get_quote_spot_position()
        .get_signed_token_amount(&quote_market)?;
    let base_token_amount_before = user
        .force_get_spot_position_mut(base_market_index)?
        .get_signed_token_amount(&base_market)?;

    let mut maker_token_amounts_before: BTreeMap<Pubkey, (i128, i128)> = BTreeMap::new();
    for (maker_key, _, _) in maker_orders_info {
        let maker = makers_and_referrer.get_ref(maker_key)?;
        let maker_quote_token_amount_before = maker
            .get_quote_spot_position()
            .get_signed_token_amount(&quote_market)?;
        let maker_base_token_amount_before = maker
            .get_spot_position(base_market_index)?
            .get_signed_token_amount(&base_market)?;
        maker_token_amounts_before.insert(
            *maker_key,
            (
                maker_base_token_amount_before,
                maker_quote_token_amount_before,
            ),
        );
    }

    // todo come up with fallback price
    let oracle_price = oracle_map.get_price_data(&base_market.oracle)?.price;
    let limit_price = user.orders[user_order_index].get_limit_price(
        Some(oracle_price),
        None,
        slot,
        base_market.order_tick_size,
    )?;

    let fulfillment_methods = determine_spot_fulfillment_methods(
        &user.orders[user_order_index],
        maker_orders_info,
        limit_price,
        fulfillment_params.is_external(),
    )?;

    let mut base_asset_amount = 0_u64;
    let mut quote_asset_amount = 0_u64;
    let mut maker_fills: BTreeMap<Pubkey, i64> = BTreeMap::new();
    let maker_direction = user.orders[user_order_index].direction.opposite();
    for fulfillment_method in fulfillment_methods.iter() {
        if user.orders[user_order_index].status != OrderStatus::Open {
            break;
        }

        let (base_filled, quote_filled) = match fulfillment_method {
            SpotFulfillmentMethod::Match(maker_key, maker_order_index) => {
                let mut maker = makers_and_referrer.get_ref_mut(maker_key)?;
                let mut maker_stats = if maker.authority == user.authority {
                    None
                } else {
                    Some(makers_and_referrer_stats.get_ref_mut(&maker.authority)?)
                };

                let (base_filled, quote_filled) = fulfill_spot_order_with_match(
                    &mut base_market,
                    &mut quote_market,
                    user,
                    user_stats,
                    user_order_index,
                    user_key,
                    &mut maker,
                    &mut maker_stats.as_deref_mut(),
                    *maker_order_index as usize,
                    maker_key,
                    filler.as_deref_mut(),
                    filler_stats.as_deref_mut(),
                    filler_key,
                    now,
                    slot,
                    oracle_map,
                    fee_structure,
                )?;

                if base_filled != 0 {
                    update_maker_fills_map(
                        &mut maker_fills,
                        maker_key,
                        maker_direction,
                        base_filled,
                    )?;
                }

                (base_filled, quote_filled)
            }
            SpotFulfillmentMethod::ExternalMarket => fulfill_spot_order_with_external_market(
                &mut base_market,
                &mut quote_market,
                user,
                user_stats,
                user_order_index,
                user_key,
                filler.as_deref_mut(),
                filler_stats.as_deref_mut(),
                filler_key,
                now,
                slot,
                oracle_map,
                fee_structure,
                fulfillment_params,
            )?,
        };

        base_asset_amount = base_asset_amount.safe_add(base_filled)?;
        quote_asset_amount = quote_asset_amount.safe_add(quote_filled)?;
    }

    validate!(
        (base_asset_amount > 0) == (quote_asset_amount > 0),
        DexError::DefaultError,
        "invalid fill base = {} quote = {}",
        base_asset_amount,
        quote_asset_amount
    )?;

    let quote_token_amount_after = user
        .get_quote_spot_position()
        .get_signed_token_amount(&quote_market)?;
    let base_token_amount_after = user
        .force_get_spot_position_mut(base_market_index)?
        .get_signed_token_amount(&base_market)?;

    let quote_price = oracle_map.get_price_data(&quote_market.oracle)?.price;
    let base_price = oracle_map.get_price_data(&base_market.oracle)?.price;

    let strict_quote_price = StrictOraclePrice::new(
        quote_price,
        quote_market
            .historical_oracle_data
            .last_oracle_price_twap_5min,
        true,
    );
    let strict_base_price = StrictOraclePrice::new(
        base_price,
        base_market
            .historical_oracle_data
            .last_oracle_price_twap_5min,
        true,
    );

    let margin_type = if order_direction == PositionDirection::Long {
        // sell quote, buy base
        select_margin_type_for_swap(
            &quote_market,
            &base_market,
            &strict_quote_price,
            &strict_base_price,
            quote_token_amount_before,
            base_token_amount_before,
            quote_token_amount_after,
            base_token_amount_after,
            MarginRequirementType::Fill,
        )?
    } else {
        // sell base, buy quote
        select_margin_type_for_swap(
            &base_market,
            &quote_market,
            &strict_base_price,
            &strict_quote_price,
            base_token_amount_before,
            quote_token_amount_before,
            base_token_amount_after,
            quote_token_amount_after,
            MarginRequirementType::Fill,
        )?
    };

    drop(base_market);
    drop(quote_market);

    let taker_margin_calculation =
        calculate_margin_requirement_and_total_collateral_and_liability_info(
            user,
            spot_market_map,
            oracle_map,
            MarginContext::standard(margin_type)
                .fuel_spot_deltas([
                    (
                        base_market_index,
                        base_token_amount_before.safe_sub(base_token_amount_after)?,
                    ),
                    (
                        QUOTE_SPOT_MARKET_INDEX,
                        quote_token_amount_before.safe_sub(quote_token_amount_after)?,
                    ),
                ])
                .fuel_numerator(user, now),
        )?;

    // user hasnt recieved initial fuel or below global start time
    user_stats.update_fuel_bonus(
        user,
        taker_margin_calculation.fuel_deposits,
        taker_margin_calculation.fuel_borrows,
        taker_margin_calculation.fuel_positions,
        now,
    )?;

    if !taker_margin_calculation.meets_margin_requirement() {
        msg!(
            "taker breached maintenance requirements (margin requirement {}) (total_collateral {})",
            taker_margin_calculation.margin_requirement,
            taker_margin_calculation.total_collateral
        );
        return Err(DexError::InsufficientCollateral);
    }

    for (maker_key, _) in maker_fills {
        let mut maker: RefMut<User> = makers_and_referrer.get_ref_mut(&maker_key)?;
        let maker_stats = if maker.authority == user.authority {
            None
        } else {
            Some(makers_and_referrer_stats.get_ref_mut(&maker.authority)?)
        };

        let quote_market = spot_market_map.get_quote_spot_market()?;
        let base_market = spot_market_map.get_ref(&base_market_index)?;

        let (maker_base_token_amount_before, maker_quote_token_amount_before) =
            maker_token_amounts_before.get(&maker_key).safe_unwrap()?;

        let maker_quote_token_amount_after = maker
            .get_quote_spot_position()
            .get_signed_token_amount(&quote_market)?;
        let maker_base_token_amount_after = maker
            .get_spot_position(base_market_index)?
            .get_signed_token_amount(&base_market)?;

        let margin_type = if maker_direction == PositionDirection::Long {
            // sell quote, buy base
            select_margin_type_for_swap(
                &quote_market,
                &base_market,
                &strict_quote_price,
                &strict_base_price,
                *maker_quote_token_amount_before,
                *maker_base_token_amount_before,
                maker_quote_token_amount_after,
                maker_base_token_amount_after,
                MarginRequirementType::Fill,
            )?
        } else {
            // sell base, buy quote
            select_margin_type_for_swap(
                &base_market,
                &quote_market,
                &strict_base_price,
                &strict_quote_price,
                *maker_base_token_amount_before,
                *maker_quote_token_amount_before,
                maker_base_token_amount_after,
                maker_quote_token_amount_after,
                MarginRequirementType::Fill,
            )?
        };

        drop(base_market);
        drop(quote_market);

        let maker_margin_calculation: MarginCalculation =
            calculate_margin_requirement_and_total_collateral_and_liability_info(
                &maker,
                spot_market_map,
                oracle_map,
                MarginContext::standard(margin_type)
                    .fuel_spot_deltas([
                        (
                            base_market_index,
                            maker_base_token_amount_before
                                .safe_sub(maker_base_token_amount_after)?,
                        ),
                        (
                            QUOTE_SPOT_MARKET_INDEX,
                            maker_quote_token_amount_before
                                .safe_sub(maker_quote_token_amount_after)?,
                        ),
                    ])
                    .fuel_numerator(&maker, now),
            )?;

        if let Some(mut maker_stats) = maker_stats {
            maker_stats.update_fuel_bonus(
                &mut maker,
                maker_margin_calculation.fuel_deposits,
                maker_margin_calculation.fuel_borrows,
                maker_margin_calculation.fuel_positions,
                now,
            )?;
        }

        if !maker_margin_calculation.meets_margin_requirement() {
            msg!(
                    "maker ({}) breached maintenance requirements (margin requirement {}) (total_collateral {})",
                    maker_key,
                    maker_margin_calculation.margin_requirement,
                    maker_margin_calculation.total_collateral
                );
            return Err(DexError::InsufficientCollateral);
        }
    }

    Ok((base_asset_amount, quote_asset_amount))
}


pub fn fulfill_spot_order_with_match(
    base_market: &mut SpotMarket,
    quote_market: &mut SpotMarket,
    taker: &mut User,
    taker_stats: &mut UserStats,
    taker_order_index: usize,
    taker_key: &Pubkey,
    maker: &mut User,
    maker_stats: &mut Option<&mut UserStats>,
    maker_order_index: usize,
    maker_key: &Pubkey,
    filler: Option<&mut User>,
    filler_stats: Option<&mut UserStats>,
    filler_key: &Pubkey,
    now: i64,
    slot: u64,
    oracle_map: &mut OracleMap,
    fee_structure: &FeeStructure,
) -> VortexDexResult<(u64, u64)> {
    if !are_orders_same_market_but_different_sides(
        &maker.orders[maker_order_index],
        &taker.orders[taker_order_index],
    ) {
        return Ok((0_u64, 0_u64));
    }

    let market_index = taker.orders[taker_order_index].market_index;
    let oracle_price = oracle_map.get_price_data(&base_market.oracle)?.price;
    let taker_price = match taker.orders[taker_order_index].get_limit_price(
        Some(oracle_price),
        None,
        slot,
        base_market.order_tick_size,
    )? {
        Some(price) => price,
        None => {
            return Ok((0_u64, 0_u64));
        }
    };

    let taker_spot_position_index = taker.get_spot_position_index(market_index)?;
    let taker_token_amount =
        taker.spot_positions[taker_spot_position_index].get_signed_token_amount(base_market)?;
    let taker_base_asset_amount = taker.orders[taker_order_index]
        .get_standardized_base_asset_amount_unfilled(
            Some(taker_token_amount.cast()?),
            base_market.order_step_size,
        )?;
    let taker_order_slot = taker.orders[taker_order_index].slot;
    let taker_direction = taker.orders[taker_order_index].direction;

    let maker_price = maker.orders[maker_order_index].force_get_limit_price(
        Some(oracle_price),
        None,
        slot,
        base_market.order_tick_size,
    )?;
    let maker_direction = maker.orders[maker_order_index].direction;
    let maker_spot_position_index = maker.get_spot_position_index(market_index)?;
    let maker_token_amount =
        maker.spot_positions[maker_spot_position_index].get_signed_token_amount(base_market)?;
    let maker_base_asset_amount = maker.orders[maker_order_index]
        .get_standardized_base_asset_amount_unfilled(
            Some(maker_token_amount.cast()?),
            base_market.order_step_size,
        )?;

    let orders_cross = do_orders_cross(maker_direction, maker_price, taker_price);

    if !orders_cross {
        msg!(
            "orders dont cross. maker price {} taker price {}",
            maker_price,
            taker_price
        );
        return Ok((0_u64, 0_u64));
    }

    let (taker_max_base_asset_amount, taker_max_quote_asset_amount) =
        get_max_fill_amounts(taker, taker_order_index, base_market, quote_market, false)?;

    let taker_base_asset_amount =
        if let Some(taker_max_quote_asset_amount) = taker_max_quote_asset_amount {
            let taker_implied_max_base_asset_amount = standardize_base_asset_amount(
                taker_max_quote_asset_amount
                    .cast::<u128>()?
                    .safe_mul(base_market.get_precision().cast()?)?
                    .safe_div(maker_price.cast()?)?
                    .cast::<u64>()?,
                base_market.order_step_size,
            )?;
            taker_base_asset_amount.min(taker_implied_max_base_asset_amount)
        } else if let Some(taker_max_base_asset_amount) = taker_max_base_asset_amount {
            taker_base_asset_amount.min(taker_max_base_asset_amount)
        } else {
            taker_base_asset_amount
        };

    let (maker_max_base_asset_amount, maker_max_quote_asset_amount) =
        get_max_fill_amounts(maker, maker_order_index, base_market, quote_market, false)?;

    let maker_base_asset_amount =
        if let Some(maker_max_quote_asset_amount) = maker_max_quote_asset_amount {
            let maker_implied_max_base_asset_amount = standardize_base_asset_amount(
                maker_max_quote_asset_amount
                    .cast::<u128>()?
                    .safe_mul(base_market.get_precision().cast()?)?
                    .safe_div(maker_price.cast()?)?
                    .cast::<u64>()?,
                base_market.order_step_size,
            )?;
            maker_base_asset_amount.min(maker_implied_max_base_asset_amount)
        } else if let Some(maker_max_base_asset_amount) = maker_max_base_asset_amount {
            maker_base_asset_amount.min(maker_max_base_asset_amount)
        } else {
            maker_base_asset_amount
        };

    let (base_asset_amount, quote_asset_amount) = calculate_fill_for_matched_orders(
        maker_base_asset_amount,
        maker_price,
        taker_base_asset_amount,
        base_market.decimals,
        maker_direction,
    )?;

    if base_asset_amount == 0 {
        return Ok((0_u64, 0_u64));
    }

    let base_precision = base_market.get_precision();
    validate_fill_price(
        quote_asset_amount,
        base_asset_amount,
        base_precision,
        taker_direction,
        taker_price,
        true,
    )?;
    validate_fill_price(
        quote_asset_amount,
        base_asset_amount,
        base_precision,
        maker_direction,
        maker_price,
        false,
    )?;

    let filler_multiplier = if filler.is_some() {
        calculate_filler_multiplier_for_matched_orders(maker_price, maker_direction, oracle_price)?
    } else {
        0
    };

    let FillFees {
        user_fee: taker_fee,
        maker_rebate,
        filler_reward,
        fee_to_market,
        ..
    } = fees_utils::calculate_fee_for_fulfillment_with_match(
        taker_stats,
        maker_stats,
        quote_asset_amount,
        fee_structure,
        taker_order_slot,
        slot,
        filler_multiplier,
        false,
        &None,
        &MarketType::Spot,
        base_market.fee_adjustment,
    )?;

    // Update taker state
    update_spot_balances_and_cumulative_deposits(
        base_asset_amount.cast()?,
        &taker.orders[taker_order_index].get_spot_position_update_direction(AssetType::Base),
        base_market,
        &mut taker.spot_positions[taker_spot_position_index],
        false,
        None,
    )?;

    let taker_quote_asset_amount_delta = match &taker.orders[taker_order_index].direction {
        PositionDirection::Long => quote_asset_amount.safe_add(taker_fee)?,
        PositionDirection::Short => quote_asset_amount.safe_sub(taker_fee)?,
    };

    update_spot_balances_and_cumulative_deposits(
        taker_quote_asset_amount_delta.cast()?,
        &taker.orders[taker_order_index].get_spot_position_update_direction(AssetType::Quote),
        quote_market,
        taker.get_quote_spot_position_mut(),
        false,
        Some(quote_asset_amount.cast()?),
    )?;

    taker.update_cumulative_spot_fees(-taker_fee.cast()?)?;

    update_order_after_fill(
        &mut taker.orders[taker_order_index],
        base_asset_amount,
        quote_asset_amount,
    )?;

    let taker_order_direction = taker.orders[taker_order_index].direction;
    decrease_spot_open_bids_and_asks(
        &mut taker.spot_positions[taker_spot_position_index],
        &taker_order_direction,
        base_asset_amount,
    )?;

    taker_stats.update_taker_volume_30d(base_market.fuel_boost_taker, quote_asset_amount, now)?;

    taker_stats.increment_total_fees(taker_fee)?;

    // Update maker state
    update_spot_balances_and_cumulative_deposits(
        base_asset_amount.cast()?,
        &maker.orders[maker_order_index].get_spot_position_update_direction(AssetType::Base),
        base_market,
        &mut maker.spot_positions[maker_spot_position_index],
        false,
        None,
    )?;

    let maker_quote_asset_amount_delta = match &maker.orders[maker_order_index].direction {
        PositionDirection::Long => quote_asset_amount.safe_sub(maker_rebate)?,
        PositionDirection::Short => quote_asset_amount.safe_add(maker_rebate)?,
    };

    update_spot_balances_and_cumulative_deposits(
        maker_quote_asset_amount_delta.cast()?,
        &maker.orders[maker_order_index].get_spot_position_update_direction(AssetType::Quote),
        quote_market,
        maker.get_quote_spot_position_mut(),
        false,
        Some(quote_asset_amount.cast()?),
    )?;

    maker.update_cumulative_spot_fees(maker_rebate.cast()?)?;

    update_order_after_fill(
        &mut maker.orders[maker_order_index],
        base_asset_amount,
        quote_asset_amount,
    )?;

    let maker_order_direction = maker.orders[maker_order_index].direction;
    decrease_spot_open_bids_and_asks(
        &mut maker.spot_positions[maker_spot_position_index],
        &maker_order_direction,
        base_asset_amount,
    )?;

    if let Some(maker_stats) = maker_stats {
        maker_stats.update_maker_volume_30d(
            base_market.fuel_boost_maker,
            quote_asset_amount,
            now,
        )?;
        maker_stats.increment_total_rebate(maker_rebate)?;
    } else {
        taker_stats.update_maker_volume_30d(
            base_market.fuel_boost_maker,
            quote_asset_amount,
            now,
        )?;
        taker_stats.increment_total_rebate(maker_rebate)?;
    }

    // Update filler state
    if let (Some(filler), Some(filler_stats)) = (filler, filler_stats) {
        if filler_reward > 0 {
            update_spot_balances(
                filler_reward.cast()?,
                &SpotBalanceType::Deposit,
                quote_market,
                filler.get_quote_spot_position_mut(),
                false,
            )?;

            filler.update_cumulative_spot_fees(filler_reward.cast()?)?;
        }

        filler.update_last_active_slot(slot);
        filler_stats.update_filler_volume(quote_asset_amount, now)?;
    }

    // Update base market
    base_market.total_spot_fee = base_market.total_spot_fee.safe_add(fee_to_market.cast()?)?;

    update_spot_balances(
        fee_to_market.cast()?,
        &SpotBalanceType::Deposit,
        quote_market,
        &mut base_market.spot_fee_pool,
        false,
    )?;

    let fill_record_id = get_then_update_id!(base_market, next_fill_record_id);
    let order_action_explanation = if maker.orders[maker_order_index].is_jit_maker() {
        OrderActionExplanation::OrderFilledWithMatchJit
    } else {
        OrderActionExplanation::OrderFilledWithMatch
    };
    let order_action_record = get_order_action_record(
        now,
        OrderAction::Fill,
        order_action_explanation,
        maker.orders[maker_order_index].market_index,
        Some(*filler_key),
        Some(fill_record_id),
        Some(filler_reward),
        Some(base_asset_amount),
        Some(quote_asset_amount.cast()?),
        Some(taker_fee),
        Some(maker_rebate),
        None,
        Some(0),
        Some(0),
        Some(*taker_key),
        Some(taker.orders[taker_order_index]),
        Some(*maker_key),
        Some(maker.orders[maker_order_index]),
        oracle_map.get_price_data(&base_market.oracle)?.price,
    )?;
    emit_stack::<_, { OrderActionRecord::SIZE }>(order_action_record)?;

    // Clear taker/maker order if completely filled
    if taker.orders[taker_order_index].get_base_asset_amount_unfilled(None)? == 0 {
        taker.decrement_open_orders(taker.orders[taker_order_index].has_auction());
        taker.orders[taker_order_index] = Order::default();
        taker.spot_positions[taker_spot_position_index].open_orders -= 1;
    }

    if maker.orders[maker_order_index].get_base_asset_amount_unfilled(None)? == 0 {
        maker.decrement_open_orders(maker.orders[maker_order_index].has_auction());
        maker.orders[maker_order_index] = Order::default();
        maker.spot_positions[maker_spot_position_index].open_orders -= 1;
    }

    Ok((base_asset_amount, quote_asset_amount))
}


pub fn update_order_after_fill(
    order: &mut Order,
    base_asset_amount: u64,
    quote_asset_amount: u64,
) -> VortexDexResult {
    order.base_asset_amount_filled = order.base_asset_amount_filled.safe_add(base_asset_amount)?;

    order.quote_asset_amount_filled = order
        .quote_asset_amount_filled
        .safe_add(quote_asset_amount)?;

    if order.get_base_asset_amount_unfilled(None)? == 0 {
        order.status = OrderStatus::Filled;
    }

    Ok(())
}

pub fn fulfill_spot_order_with_external_market(
    base_market: &mut SpotMarket,
    quote_market: &mut SpotMarket,
    taker: &mut User,
    taker_stats: &mut UserStats,
    taker_order_index: usize,
    taker_key: &Pubkey,
    filler: Option<&mut User>,
    filler_stats: Option<&mut UserStats>,
    filler_key: &Pubkey,
    now: i64,
    slot: u64,
    oracle_map: &mut OracleMap,
    fee_structure: &FeeStructure,
    fulfillment_params: &mut dyn SpotFulfillmentParams,
) -> VortexDexResult<(u64, u64)> {
    let oracle_price = oracle_map.get_price_data(&base_market.oracle)?.price;
    let taker_price = taker.orders[taker_order_index].get_limit_price(
        Some(oracle_price),
        None,
        slot,
        base_market.order_tick_size,
    )?;
    let taker_token_amount = taker
        .force_get_spot_position_mut(base_market.market_index)?
        .get_signed_token_amount(base_market)?;
    let taker_base_asset_amount = taker.orders[taker_order_index]
        .get_standardized_base_asset_amount_unfilled(
            Some(taker_token_amount.cast()?),
            base_market.order_step_size,
        )?;
    let order_direction = taker.orders[taker_order_index].direction;
    let taker_order_slot = taker.orders[taker_order_index].slot;

    let (max_base_asset_amount, max_quote_asset_amount) =
        get_max_fill_amounts(taker, taker_order_index, base_market, quote_market, true)?;

    let taker_base_asset_amount =
        taker_base_asset_amount.min(max_base_asset_amount.unwrap_or(u64::MAX));

    let (best_bid, best_ask) = fulfillment_params.get_best_bid_and_ask()?;
    base_market.update_historical_index_price(best_bid, best_ask, now)?;

    let taker_price = if let Some(price) = taker_price {
        price
    } else {
        match order_direction {
            PositionDirection::Long => {
                if let Some(ask) = best_ask {
                    ask.safe_add(ask / 100)?
                } else {
                    msg!("External market has no ask");
                    return Ok((0, 0));
                }
            }
            PositionDirection::Short => {
                if let Some(bid) = best_bid {
                    bid.safe_sub(bid / 100)?
                } else {
                    msg!("External market has no bid");
                    return Ok((0, 0));
                }
            }
        }
    };

    let ExternalSpotFill {
        base_asset_amount_filled,
        base_update_direction,
        quote_asset_amount_filled,
        quote_update_direction,
        fee: external_market_fee,
        settled_referrer_rebate,
        unsettled_referrer_rebate,
    } = fulfillment_params.fulfill_order(
        order_direction,
        taker_price,
        taker_base_asset_amount,
        max_quote_asset_amount.unwrap_or(u64::MAX),
    )?;

    if base_asset_amount_filled == 0 {
        return Ok((0, 0));
    }

    update_spot_balances(
        settled_referrer_rebate as u128,
        &SpotBalanceType::Deposit,
        quote_market,
        &mut base_market.spot_fee_pool,
        false,
    )?;

    validate_fill_price(
        quote_asset_amount_filled,
        base_asset_amount_filled,
        base_market.get_precision(),
        order_direction,
        taker_price,
        true,
    )?;

    let fee_pool_amount = get_token_amount(
        base_market.spot_fee_pool.scaled_balance,
        quote_market,
        &SpotBalanceType::Deposit,
    )?;

    let ExternalFillFees {
        user_fee: taker_fee,
        fee_to_market,
        fee_pool_delta,
        filler_reward,
    } = fees_utils::calculate_fee_for_fulfillment_with_external_market(
        taker_stats,
        quote_asset_amount_filled,
        fee_structure,
        taker_order_slot,
        slot,
        filler.is_some(),
        external_market_fee,
        unsettled_referrer_rebate,
        fee_pool_amount.cast()?,
        base_market.fee_adjustment,
    )?;

    let quote_spot_position_delta = match quote_update_direction {
        SpotBalanceType::Deposit => quote_asset_amount_filled.safe_sub(taker_fee)?,
        SpotBalanceType::Borrow => quote_asset_amount_filled.safe_add(taker_fee)?,
    };

    validate!(
        base_update_direction
            == taker.orders[taker_order_index].get_spot_position_update_direction(AssetType::Base),
        DexError::FailedToFillOnExternalMarket,
        "Fill on external spot market lead to unexpected to update direction"
    )?;

    let base_update_direction =
        taker.orders[taker_order_index].get_spot_position_update_direction(AssetType::Base);
    update_spot_balances_and_cumulative_deposits(
        base_asset_amount_filled.cast()?,
        &base_update_direction,
        base_market,
        taker.force_get_spot_position_mut(base_market.market_index)?,
        base_update_direction == SpotBalanceType::Borrow,
        None,
    )?;

    validate!(
        quote_update_direction
            == taker.orders[taker_order_index].get_spot_position_update_direction(AssetType::Quote),
        DexError::FailedToFillOnExternalMarket,
        "Fill on external market lead to unexpected to update direction"
    )?;

    let quote_update_direction =
        taker.orders[taker_order_index].get_spot_position_update_direction(AssetType::Quote);
    update_spot_balances_and_cumulative_deposits(
        quote_spot_position_delta.cast()?,
        &quote_update_direction,
        quote_market,
        taker.get_quote_spot_position_mut(),
        quote_update_direction == SpotBalanceType::Borrow,
        Some(quote_asset_amount_filled.cast()?),
    )?;

    taker.update_cumulative_spot_fees(-taker_fee.cast()?)?;

    taker_stats.update_taker_volume_30d(
        base_market.fuel_boost_taker,
        quote_asset_amount_filled.cast()?,
        now,
    )?;

    taker_stats.increment_total_fees(taker_fee.cast()?)?;

    update_order_after_fill(
        &mut taker.orders[taker_order_index],
        base_asset_amount_filled,
        quote_asset_amount_filled,
    )?;

    let taker_order_direction = taker.orders[taker_order_index].direction;
    decrease_spot_open_bids_and_asks(
        taker.force_get_spot_position_mut(base_market.market_index)?,
        &taker_order_direction,
        base_asset_amount_filled,
    )?;

    if let (Some(filler), Some(filler_stats)) = (filler, filler_stats) {
        if filler_reward > 0 {
            update_spot_balances(
                filler_reward.cast()?,
                &SpotBalanceType::Deposit,
                quote_market,
                filler.get_quote_spot_position_mut(),
                false,
            )?;

            filler.update_cumulative_spot_fees(filler_reward.cast()?)?;
        }

        filler.update_last_active_slot(slot);
        filler_stats.update_filler_volume(quote_asset_amount_filled.cast()?, now)?;
    }

    if fee_pool_delta != 0 {
        update_spot_balances(
            fee_pool_delta.unsigned_abs().cast()?,
            if fee_pool_delta > 0 {
                &SpotBalanceType::Deposit
            } else {
                &SpotBalanceType::Borrow
            },
            quote_market,
            &mut base_market.spot_fee_pool,
            false,
        )?;
    }

    base_market.total_spot_fee = base_market.total_spot_fee.safe_add(fee_to_market.cast()?)?;

    let fill_record_id = get_then_update_id!(base_market, next_fill_record_id);
    let order_action_record = get_order_action_record(
        now,
        OrderAction::Fill,
        fulfillment_params.get_order_action_explanation()?,
        taker.orders[taker_order_index].market_index,
        Some(*filler_key),
        Some(fill_record_id),
        Some(filler_reward),
        Some(base_asset_amount_filled),
        Some(quote_asset_amount_filled.cast()?),
        Some(taker_fee),
        Some(0),
        None,
        Some(0),
        Some(external_market_fee),
        Some(*taker_key),
        Some(taker.orders[taker_order_index]),
        None,
        None,
        oracle_price,
    )?;
    emit_stack::<_, { OrderActionRecord::SIZE }>(order_action_record)?;

    if taker.orders[taker_order_index].get_base_asset_amount_unfilled(None)? == 0 {
        taker.decrement_open_orders(taker.orders[taker_order_index].has_auction());
        taker.orders[taker_order_index] = Order::default();
        taker
            .force_get_spot_position_mut(base_market.market_index)?
            .open_orders -= 1;
    }

    Ok((base_asset_amount_filled, quote_asset_amount_filled))
}


#[inline(always)]
fn update_maker_fills_map(
    map: &mut BTreeMap<Pubkey, i64>,
    maker_key: &Pubkey,
    maker_direction: PositionDirection,
    fill: u64,
) -> VortexDexResult {
    let signed_fill = match maker_direction {
        PositionDirection::Long => fill.cast::<i64>()?,
        PositionDirection::Short => -fill.cast::<i64>()?,
    };

    if let Some(maker_filled) = map.get_mut(maker_key) {
        *maker_filled = maker_filled.safe_add(signed_fill)?;
    } else {
        map.insert(*maker_key, signed_fill);
    }

    Ok(())
}


pub fn trigger_spot_order(
    order_id: u32,
    state: &DexState,
    user: &AccountLoader<User>,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    filler: &AccountLoader<User>,
    clock: &Clock,
) -> VortexDexResult {
    let now = clock.unix_timestamp;
    let slot = clock.slot;

    let filler_key = filler.key();
    let user_key = user.key();
    let user = &mut load_mut!(user)?;

    let order_index = user
        .orders
        .iter()
        .position(|order| order.order_id == order_id)
        .ok_or_else(print_error!(DexError::OrderDoesNotExist))?;

    let (order_status, market_index, market_type) =
        get_struct_values!(user.orders[order_index], status, market_index, market_type);

    validate!(
        order_status == OrderStatus::Open,
        DexError::OrderNotOpen,
        "Order not open"
    )?;

    validate!(
        user.orders[order_index].must_be_triggered(),
        DexError::OrderNotTriggerable,
        "Order is not triggerable"
    )?;

    validate!(
        !user.orders[order_index].triggered(),
        DexError::OrderNotTriggerable,
        "Order is already triggered"
    )?;

    validate!(
        market_type == MarketType::Spot,
        DexError::InvalidOrderMarketType,
        "Order must be a spot order"
    )?;

    validate_user_not_being_liquidated(
        user,
        spot_market_map,
        oracle_map,
        state.liquidation_margin_buffer_ratio,
    )?;

    validate!(!user.is_bankrupt(), DexError::UserBankrupt)?;

    let spot_market = spot_market_map.get_ref(&market_index)?;
    let (oracle_price_data, oracle_validity) = oracle_map.get_price_data_and_validity(
        MarketType::Spot,
        spot_market.market_index,
        &spot_market.oracle,
        spot_market.historical_oracle_data.last_oracle_price_twap,
        spot_market.get_max_confidence_interval_multiplier()?,
    )?;
    let strict_oracle_price = StrictOraclePrice {
        current: oracle_price_data.price,
        twap_5min: Some(
            spot_market
                .historical_oracle_data
                .last_oracle_price_twap_5min,
        ),
    };

    validate!(
        is_oracle_valid_for_action(oracle_validity, Some(VortexDexAction::TriggerOrder))?,
        DexError::InvalidOracle,
        "OracleValidity for spot marketIndex={} invalid for TriggerOrder",
        spot_market.market_index
    )?;

    let oracle_price = oracle_price_data.price;

    let oracle_too_divergent_with_twap_5min = is_oracle_too_divergent_with_twap_5min(
        oracle_price_data.price,
        spot_market
            .historical_oracle_data
            .last_oracle_price_twap_5min,
        state
            .oracle_guard_rails
            .max_oracle_twap_5min_percent_divergence()
            .cast()?,
    )?;

    validate!(
        !oracle_too_divergent_with_twap_5min,
        DexError::OrderBreachesOraclePriceLimits,
        "oracle price vs twap too divergent"
    )?;

    let can_trigger = order_satisfies_trigger_condition(
        &user.orders[order_index],
        oracle_price.unsigned_abs().cast()?,
    )?;
    validate!(can_trigger, DexError::OrderDidNotSatisfyTriggerCondition)?;

    let position_index = user.get_spot_position_index(market_index)?;
    let signed_token_amount =
        user.spot_positions[position_index].get_signed_token_amount(&spot_market)?;

    let worst_case_simulation_before = user.spot_positions[position_index]
        .get_worst_case_fill_simulation(
            &spot_market,
            &strict_oracle_price,
            Some(signed_token_amount),
            MarginRequirementType::Initial,
        )?;

    {
        update_trigger_order_params(
            &mut user.orders[order_index],
            oracle_price_data,
            slot,
            30,
        )?;

        if user.orders[order_index].has_auction() {
            user.increment_open_auctions();
        }

        let direction = user.orders[order_index].direction;
        let base_asset_amount = user.orders[order_index].base_asset_amount;

        let user_position = user.force_get_spot_position_mut(market_index)?;
        increase_spot_open_bids_and_asks(user_position, &direction, base_asset_amount.cast()?)?;
    }

    let is_filler_taker = user_key == filler_key;
    let mut filler = if !is_filler_taker {
        Some(load_mut!(filler)?)
    } else {
        None
    };

    let mut quote_market = spot_market_map.get_quote_spot_market_mut()?;
    let filler_reward = pay_keeper_flat_reward_for_spot(
        user,
        filler.as_deref_mut(),
        &mut quote_market,
        state.spot_fee_structure.flat_filler_fee,
        slot,
    )?;

    let order_action_record = get_order_action_record(
        now,
        OrderAction::Trigger,
        OrderActionExplanation::None,
        market_index,
        Some(filler_key),
        None,
        Some(filler_reward),
        None,
        None,
        Some(filler_reward),
        None,
        None,
        None,
        None,
        Some(user_key),
        Some(user.orders[order_index]),
        None,
        None,
        oracle_price,
    )?;

    emit!(order_action_record);

    let worst_case_simulation_after = user
        .get_spot_position(market_index)?
        .get_worst_case_fill_simulation(
            &spot_market,
            &strict_oracle_price,
            Some(signed_token_amount),
            MarginRequirementType::Initial,
        )?;

    drop(spot_market);
    drop(quote_market);

    let is_risk_increasing =
        worst_case_simulation_before.risk_increasing(worst_case_simulation_after);

    // If order is risk increasing and user is below initial margin, cancel it
    if is_risk_increasing && !user.orders[order_index].reduce_only {
        let meets_initial_margin_requirement =
            meets_initial_margin_requirement(user, spot_market_map, oracle_map)?;

        if !meets_initial_margin_requirement {
            cancel_order(
                order_index,
                user,
                &user_key,
                spot_market_map,
                oracle_map,
                now,
                slot,
                OrderActionExplanation::InsufficientFreeCollateral,
                Some(&filler_key),
                0,
                false,
            )?;
        }
    }

    user.update_last_active_slot(slot);

    Ok(())
}

fn update_trigger_order_params(
    order: &mut Order,
    oracle_price_data: &OraclePriceData,
    slot: u64,
    min_auction_duration: u8,
) -> VortexDexResult {
    order.trigger_condition = match order.trigger_condition {
        OrderTriggerCondition::Above => OrderTriggerCondition::TriggeredAbove,
        OrderTriggerCondition::Below => OrderTriggerCondition::TriggeredBelow,
        _ => {
            return Err(print_error!(DexError::InvalidTriggerOrderCondition)());
        }
    };

    order.slot = slot;

    let (auction_duration, auction_start_price, auction_end_price) =
        calculate_auction_params_for_trigger_order(
            order,
            oracle_price_data,
            min_auction_duration,
        )?;

    msg!(
        "new auction duration {} start price {} end price {}",
        auction_duration,
        auction_start_price,
        auction_end_price
    );

    order.auction_duration = auction_duration;
    order.auction_start_price = auction_start_price;
    order.auction_end_price = auction_end_price;

    Ok(())
}