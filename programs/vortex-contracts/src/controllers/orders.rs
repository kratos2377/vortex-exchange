use anchor_lang::prelude::*;

use crate::{errors::VortexDexResult, get_struct_values, get_then_update_id, load_mut, state::{dex_state::DexState, events::{emit_stack, get_order_action_record, OrderAction, OrderActionExplanation, OrderActionRecord, OrderRecord}, oracle_map::OracleMap, order_params::{ModifyOrderParams, ModifyOrderPolicy, OrderParams, PlaceOrderOptions, PostOnlyParam}, position::PositionDirection, spot_market::{MarketStatus, SpotBalanceType}, spot_market_map::SpotMarketMap, user::{MarketType, Order, OrderStatus, OrderTriggerCondition, OrderType, User}}, utils::{constants::QUOTE_SPOT_MARKET_INDEX, spot_market_utils::get_signed_token_amount}, validate};

use super::{position, spot_position::decrease_spot_open_bids_and_asks};


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

    let is_perp_order = order_market_type == MarketType::Perp;

    validate!(order_status == OrderStatus::Open, ErrorCode::OrderNotOpen)?;

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

    validate!(!user.is_bankrupt(), ErrorCode::UserBankrupt)?;

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
            ErrorCode::UserReduceOnly,
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
        .ok_or(ErrorCode::MaxNumberOfOrders)?;

    if params.user_order_id > 0 {
        let user_order_id_already_used = user
            .orders
            .iter()
            .position(|order| order.user_order_id == params.user_order_id);

        if user_order_id_already_used.is_some() {
            msg!("user_order_id is already in use {}", params.user_order_id);
            return Err(ErrorCode::UserOrderIdAlreadyInUse);
        }
    }

    let market_index = params.market_index;
    let spot_market = &spot_market_map.get_ref(&market_index)?;
    let force_reduce_only = spot_market.is_reduce_only();
    let step_size = spot_market.order_step_size;

    validate!(
        !matches!(spot_market.status, MarketStatus::Initialized),
        ErrorCode::MarketBeingInitialized,
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
            ErrorCode::InvalidOrderSizeTooSmall,
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
            ErrorCode::InvalidOrderNotStepSizeMultiple,
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

    validate!(spot_market.orders_enabled, ErrorCode::SpotOrdersDisabled)?;

    validate!(
        params.market_index != QUOTE_SPOT_MARKET_INDEX,
        ErrorCode::InvalidOrderBaseQuoteAsset,
        "can not place order for quote asset"
    )?;

    validate!(
        params.market_type == MarketType::Spot,
        ErrorCode::InvalidOrderMarketType,
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
            perp_market_map,
            spot_market_map,
            oracle_map,
            options.risk_increasing,
        )?;
    }

    validate_spot_margin_trading(user, perp_market_map, spot_market_map, oracle_map)?;

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