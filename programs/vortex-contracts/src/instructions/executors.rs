use anchor_lang::prelude::*;
use anchor_spl::token_interface::{TokenAccount, TokenInterface};

use crate::{controllers, dex_state::DexState, errors::DexError, fulfillment_params::{vortex::MatchFulfillmentParams}, load, load_mut, profit_and_loss::SettlePnlMode, spot_fulfillment_params::SpotFulfillmentParams, spot_market_map::{get_writable_spot_market_set, get_writable_spot_market_set_from_many, MarketSet}, user::{MarketType, OrderStatus, User}, user_map::{load_user_maps, UserMap, UserStatsMap}, user_stats::UserStats, utils::{self, constants::{QUOTE_PRECISION_I128, QUOTE_SPOT_MARKET_INDEX}, margin_utils::calculate_user_equity, spot_market_utils::validate_spot_market_vault_amount, user_utils::validate_user_is_idle}, validate};

use super::{account::{get_token_mint, load_maps, AccountMaps}, constraints::{can_sign_for_user, exchange_not_paused, fill_not_paused, is_stats_for_user, liq_not_paused, withdraw_not_paused}};

// Add multiple different dex support
#[derive(Clone, Copy, AnchorSerialize, AnchorDeserialize, PartialEq, Debug, Eq, Default)]
pub enum SpotFulfillmentType {
    #[default]
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
            .ok_or(DexError::OrderDoesNotExist)?;

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

    let mut fulfillment_params: Box<dyn SpotFulfillmentParams> = {
            let base_market = spot_market_map.get_ref(&market_index)?;
            let quote_market = spot_market_map.get_quote_spot_market()?;
            Box::new(MatchFulfillmentParams::new(
                remaining_accounts_iter,
                &base_market,
                &quote_market,
            )?)
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
    liq_not_paused(&ctx.accounts.state)
)]
pub fn handle_liquidate_spot<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, LiquidateSpot<'info>>,
    asset_market_index: u16,
    liability_market_index: u16,
    liquidator_max_liability_transfer: u128,
    limit_price: Option<u64>,
) -> Result<()> {
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let state = &ctx.accounts.state;

    let user_key = ctx.accounts.user.key();
    let liquidator_key = ctx.accounts.liquidator.key();

    validate!(
        user_key != liquidator_key,
        DexError::UserCantLiquidateThemself
    )?;

    let user = &mut load_mut!(ctx.accounts.user)?;
    let user_stats = &mut load_mut!(ctx.accounts.user_stats)?;
    let liquidator = &mut load_mut!(ctx.accounts.liquidator)?;
    let liquidator_stats = &mut load_mut!(ctx.accounts.liquidator_stats)?;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &get_writable_spot_market_set_from_many(vec![asset_market_index, liability_market_index]),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    controllers::liquidation::liquidate_spot(
        asset_market_index,
        liability_market_index,
        liquidator_max_liability_transfer,
        limit_price,
        user,
        &user_key,
        user_stats,
        liquidator,
        &liquidator_key,
        liquidator_stats,
        &spot_market_map,
        &mut oracle_map,
        now,
        clock.slot,
        state,
    )?;

    Ok(())
}




#[access_control(
    withdraw_not_paused(&ctx.accounts.state)
)]
pub fn handle_resolve_spot_bankruptcy<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, ResolveBankruptcy<'info>>,
    market_index: u16,
) -> Result<()> {
    let state = &ctx.accounts.state;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let user_key = ctx.accounts.user.key();
    let liquidator_key = ctx.accounts.liquidator.key();

    validate!(
        user_key != liquidator_key,
        DexError::UserCantLiquidateThemself
    )?;

    let user = &mut load_mut!(ctx.accounts.user)?;
    let liquidator = &mut load_mut!(ctx.accounts.liquidator)?;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        remaining_accounts_iter,
        &get_writable_spot_market_set(market_index),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let mint = get_token_mint(remaining_accounts_iter)?;

    {
        let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;

        // reload the spot market vault balance so it's up-to-date
        ctx.accounts.spot_market_vault.reload()?;
        ctx.accounts.insurance_fund_vault.reload()?;
        utils::spot_market_utils::validate_spot_market_vault_amount(
            spot_market,
            ctx.accounts.spot_market_vault.amount,
        )?;
    }

    let pay_from_insurance = controllers::liquidation::resolve_spot_bankruptcy(
        market_index,
        user,
        &user_key,
        liquidator,
        &liquidator_key,
        &spot_market_map,
        &mut oracle_map,
        now,
        ctx.accounts.insurance_fund_vault.amount,
    )?;

    if pay_from_insurance > 0 {
        controllers::token::send_from_program_vault(
            &ctx.accounts.token_program,
            &ctx.accounts.insurance_fund_vault,
            &ctx.accounts.spot_market_vault,
            &ctx.accounts.vortex_signer,
            ctx.accounts.state.signer_nonce,
            pay_from_insurance,
            &mint,
        )?;

        validate!(
            ctx.accounts.insurance_fund_vault.amount > 0,
            DexError::InvalidIFDetected,
            "insurance_fund_vault.amount must remain > 0"
        )?;
    }

    {
        let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;
        // reload the spot market vault balance so it's up-to-date
        ctx.accounts.spot_market_vault.reload()?;
        utils::spot_market_utils::validate_spot_market_vault_amount(
            spot_market,
            ctx.accounts.spot_market_vault.amount,
        )?;
    }

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
pub struct LiquidateSpot<'info> {
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = can_sign_for_user(&liquidator, &authority)?
    )]
    pub liquidator: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&liquidator, &liquidator_stats)?
    )]
    pub liquidator_stats: AccountLoader<'info, UserStats>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&user, &user_stats)?
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
}


#[derive(Accounts)]
#[instruction(spot_market_index: u16,)]
pub struct ResolveBankruptcy<'info> {
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = can_sign_for_user(&liquidator, &authority)?
    )]
    pub liquidator: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&liquidator, &liquidator_stats)?
    )]
    pub liquidator_stats: AccountLoader<'info, UserStats>,
    #[account(mut)]
    pub user: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&user, &user_stats)?
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
    #[account(
        mut,
        seeds = [b"spot_market_vault".as_ref(), spot_market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [b"insurance_fund_vault".as_ref(), spot_market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub insurance_fund_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        constraint = state.signer.eq(&vortex_signer.key())
    )]
    /// CHECK: forced vortex_signer
    pub vortex_signer: AccountInfo<'info>,
    pub token_program: Interface<'info, TokenInterface>,
}
