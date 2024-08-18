use anchor_lang::prelude::*;

use crate::{controllers, dex_state::DexState, fulfillment_params::{serum::SerumFulfillmentParams, vortex::MatchFulfillmentParams}, load, load_mut, profit_and_loss::SettlePnlMode, spot_fulfillment_params::SpotFulfillmentParams, spot_market_map::{get_writable_spot_market_set, get_writable_spot_market_set_from_many, MarketSet}, user::{MarketType, OrderStatus, User}, user_map::{load_user_maps, UserMap, UserStatsMap}, user_stats::UserStats, utils::{constants::{QUOTE_PRECISION_I128, QUOTE_SPOT_MARKET_INDEX}, margin_utils::calculate_user_equity, spot_market_utils::validate_spot_market_vault_amount, user_utils::validate_user_is_idle}};

use super::{account::{load_maps, AccountMaps}, constraints::{settle_pnl_not_paused , can_sign_for_user, fill_not_paused, is_stats_for_user , exchange_not_paused}};

#[derive(Clone, Copy, AnchorSerialize, AnchorDeserialize, PartialEq, Debug, Eq, Default)]
pub enum SpotFulfillmentType {
    #[default]
    SerumV3,
    Match,
}


#[access_control(
    fill_not_paused(&ctx.accounts.state)
)]
pub fn handle_fill_spot_order<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, FillOrder<'info>>,
    order_id: Option<u32>,
    fulfillment_type: Option<SpotFulfillmentType>,
    _maker_order_id: Option<u32>,
) -> Result<()> {
    let (order_id, market_index) = {
        let user = &load!(ctx.accounts.user)?;
        // if there is no order id, use the users last order id
        let order_id = order_id.unwrap_or_else(|| user.get_last_order_id());
        let market_index = user
            .get_order(order_id)
            .map(|order| order.market_index)
            .ok_or(ErrorCode::OrderDoesNotExist)?;

        (order_id, market_index)
    };

    let user_key = &ctx.accounts.user.key();
    fill_spot_order(
        ctx,
        order_id,
        market_index,
        fulfillment_type.unwrap_or(SpotFulfillmentType::Match),
    )
    .map_err(|e| {
        msg!("Err filling order id {} for user {}", order_id, user_key);
        e
    })?;

    Ok(())
}

fn fill_spot_order<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, FillOrder<'info>>,
    order_id: u32,
    market_index: u16,
    fulfillment_type: SpotFulfillmentType,
) -> Result<()> {
    let clock = Clock::get()?;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        remaining_accounts_iter,
        &get_writable_spot_market_set_from_many(vec![QUOTE_SPOT_MARKET_INDEX, market_index]),
        Clock::get()?.slot,
        None,
    )?;

    let (makers_and_referrer, makers_and_referrer_stats) = match fulfillment_type {
        SpotFulfillmentType::Match => load_user_maps(remaining_accounts_iter, true)?,
        _ => (UserMap::empty(), UserStatsMap::empty()),
    };

    let mut fulfillment_params: Box<dyn SpotFulfillmentParams> = match fulfillment_type {
        SpotFulfillmentType::SerumV3 => {
            let base_market = spot_market_map.get_ref(&market_index)?;
            let quote_market = spot_market_map.get_quote_spot_market()?;
            Box::new(SerumFulfillmentParams::new(
                remaining_accounts_iter,
                &ctx.accounts.state,
                &base_market,
                &quote_market,
                clock.unix_timestamp,
            )?)
        }
        SpotFulfillmentType::Match => {
            let base_market = spot_market_map.get_ref(&market_index)?;
            let quote_market = spot_market_map.get_quote_spot_market()?;
            Box::new(MatchFulfillmentParams::new(
                remaining_accounts_iter,
                &base_market,
                &quote_market,
            )?)
        }
    };

    controllers::orders::fill_spot_order(
        order_id,
        &ctx.accounts.state,
        &ctx.accounts.user,
        &ctx.accounts.user_stats,
        &spot_market_map,
        &mut oracle_map,
        &ctx.accounts.filler,
        &ctx.accounts.filler_stats,
        &makers_and_referrer,
        &makers_and_referrer_stats,
        None,
        &clock,
        fulfillment_params.as_mut(),
    )?;

    let base_market = spot_market_map.get_ref(&market_index)?;
    let quote_market = spot_market_map.get_quote_spot_market()?;
    fulfillment_params.validate_vault_amounts(&base_market, &quote_market)?;

    Ok(())
}

#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_trigger_order<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, TriggerOrder<'info>>,
    order_id: u32,
) -> Result<()> {
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        Clock::get()?.slot,
        None,
    )?;

    let market_type = match load!(ctx.accounts.user)?.get_order(order_id) {
        Some(order) => order.market_type,
        None => {
            msg!("order_id not found {}", order_id);
            return Ok(());
        }
    };

controllers::orders::trigger_spot_order(
            order_id,
            &ctx.accounts.state,
            &ctx.accounts.user,
            &spot_market_map,
            &mut oracle_map,
            &ctx.accounts.filler,
            &Clock::get()?,
        )?;
    

    Ok(())
}

#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_force_cancel_orders<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, ForceCancelOrder>,
) -> Result<()> {
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        Clock::get()?.slot,
        None,
    )?;

    controllers::orders::force_cancel_orders(
        &ctx.accounts.state,
        &ctx.accounts.user,
        &spot_market_map,
        &mut oracle_map,
        &ctx.accounts.filler,
        &Clock::get()?,
    )?;

    Ok(())
}

#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_update_user_idle<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, UpdateUserIdle<'info>>,
) -> Result<()> {
    let mut user = load_mut!(ctx.accounts.user)?;
    let clock = Clock::get()?;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        Clock::get()?.slot,
        None,
    )?;

    let (equity, _) =
        calculate_user_equity(&user, &spot_market_map, &mut oracle_map)?;

    // user flipped to idle faster if equity is less than 1000
    let accelerated = equity < QUOTE_PRECISION_I128 * 1000;

    validate_user_is_idle(&user, clock.slot, accelerated)?;

    user.idle = true;

    Ok(())
}


#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_update_user_open_orders_count<'info>(ctx: Context<UpdateUserIdle>) -> Result<()> {
    let mut user = load_mut!(ctx.accounts.user)?;

    let mut open_orders = 0_u8;
    let mut open_auctions = 0_u8;

    for order in user.orders.iter() {
        if order.status == OrderStatus::Open {
            open_orders += 1;
        }

        if order.has_auction() {
            open_auctions += 1;
        }
    }

    user.open_orders = open_orders;
    user.has_open_order = open_orders > 0;
    user.open_auctions = open_auctions;
    user.has_open_auction = open_auctions > 0;

    Ok(())
}

#[access_control(
    settle_pnl_not_paused(&ctx.accounts.state)
)]
pub fn handle_settle_pnl<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, SettlePNL>,
    market_index: u16,
) -> Result<()> {
    let clock = Clock::get()?;
    let state = &ctx.accounts.state;

    let user_key = ctx.accounts.user.key();
    let user = &mut load_mut!(ctx.accounts.user)?;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &get_writable_spot_market_set(QUOTE_SPOT_MARKET_INDEX),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;



        controllers::profit_and_loss::settle_pnl(
            market_index,
            user,
            ctx.accounts.authority.key,
            &user_key,
            &spot_market_map,
            &mut oracle_map,
            &clock,
            state,
            None,
            SettlePnlMode::MustSettle,
        )
        .map(|_| ErrorCode::InvalidOracleForSettlePnl)?;

        user.update_last_active_slot(clock.slot);
    

    let spot_market = spot_market_map.get_quote_spot_market()?;
    validate_spot_market_vault_amount(&spot_market, ctx.accounts.spot_market_vault.amount)?;

    Ok(())
}

#[access_control(
    settle_pnl_not_paused(&ctx.accounts.state)
)]
pub fn handle_settle_multiple_pnls<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, SettlePNL>,
    market_indexes: Vec<u16>,
    mode: SettlePnlMode,
) -> Result<()> {
    let clock = Clock::get()?;
    let state = &ctx.accounts.state;

    let user_key = ctx.accounts.user.key();
    let user = &mut load_mut!(ctx.accounts.user)?;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &get_writable_spot_market_set(QUOTE_SPOT_MARKET_INDEX),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let meets_margin_requirement = meets_settle_pnl_maintenance_margin_requirement(
        user,
        &spot_market_map,
        &mut oracle_map,
    )?;

    for market_index in market_indexes.iter() {
        let market_in_settlement =
            perp_market_map.get_ref(market_index)?.status == MarketStatus::Settlement;

        if market_in_settlement {
            amm_not_paused(state)?;

            controllers::profit_and_loss::settle_expired_position(
                *market_index,
                user,
                &user_key,
                &spot_market_map,
                &mut oracle_map,
                &clock,
                state,
            )?;

            user.update_last_active_slot(clock.slot);
        } else {
            controller::repeg::update_amm(
                *market_index,
                &perp_market_map,
                &mut oracle_map,
                state,
                &clock,
            )
            .map(|_| ErrorCode::InvalidOracleForSettlePnl)?;

            controller::pnl::settle_pnl(
                *market_index,
                user,
                ctx.accounts.authority.key,
                &user_key,
                &perp_market_map,
                &spot_market_map,
                &mut oracle_map,
                &clock,
                state,
                Some(meets_margin_requirement),
                mode,
            )
            .map(|_| ErrorCode::InvalidOracleForSettlePnl)?;

            user.update_last_active_slot(clock.slot);
        }
    }

    let spot_market = spot_market_map.get_quote_spot_market()?;
    validate_spot_market_vault_amount(&spot_market, ctx.accounts.spot_market_vault.amount)?;

    Ok(())
}

#[access_control(
    funding_not_paused(&ctx.accounts.state)
)]
pub fn handle_settle_funding_payment<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, SettleFunding>,
) -> Result<()> {
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let user_key = ctx.accounts.user.key();
    let user = &mut load_mut!(ctx.accounts.user)?;

    let AccountMaps {
        perp_market_map, ..
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &get_market_set_for_user_positions(&user.perp_positions),
        &MarketSet::new(),
        clock.slot,
        None,
    )?;

    controller::funding::settle_funding_payments(user, &user_key, &perp_market_map, now)?;
    user.update_last_active_slot(clock.slot);
    Ok(())
}

#[access_control(
    amm_not_paused(&ctx.accounts.state)
)]
pub fn handle_settle_lp<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, SettleLP>,
    market_index: u16,
) -> Result<()> {
    let user_key = ctx.accounts.user.key();
    let user = &mut load_mut!(ctx.accounts.user)?;

    let state = &ctx.accounts.state;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let AccountMaps {
        perp_market_map, ..
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &get_writable_perp_market_set(market_index),
        &MarketSet::new(),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let market = &mut perp_market_map.get_ref_mut(&market_index)?;
    controller::lp::settle_funding_payment_then_lp(user, &user_key, market, now)?;
    user.update_last_active_slot(clock.slot);

    Ok(())
}


#[derive(Accounts)]
pub struct FillOrder<'info> {
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = can_sign_for_user(&filler, &authority)?
    )]
    pub filler: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&filler, &filler_stats)?
    )]
    pub filler_stats: AccountLoader<'info, UserStats>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&user, &user_stats)?
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
}

#[derive(Accounts)]
pub struct ForceCancelOrder<'info> {
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = can_sign_for_user(&filler, &authority)?
    )]
    pub filler: AccountLoader<'info, User>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
}

#[derive(Accounts)]
pub struct UpdateUserIdle<'info> {
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = can_sign_for_user(&filler, &authority)?
    )]
    pub filler: AccountLoader<'info, User>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
}

#[derive(Accounts)]
pub struct TriggerOrder<'info> {
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = can_sign_for_user(&filler, &authority)?
    )]
    pub filler: AccountLoader<'info, User>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
}

#[derive(Accounts)]
pub struct SettlePNL<'info> {
    pub state: Box<Account<'info, DexState>>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
    pub authority: Signer<'info>,
    #[account(
        seeds = [b"spot_market_vault".as_ref(), 0_u16.to_le_bytes().as_ref()],
        bump
    )]
    pub spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
}