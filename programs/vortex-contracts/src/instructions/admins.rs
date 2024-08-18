use std::{convert::identity, mem::size_of};

use anchor_lang::prelude::*;
use anchor_spl::{token::Token, token_2022::Token2022, token_interface::{Mint, TokenAccount, TokenInterface}};
use serum_dex::error::DexError;

use crate::{controllers::{self, token::close_vault}, dex_state::{DexState, ExchangeStatus, FeeStructure, OracleGuardRails}, fulfillment_params::serum::{SerumContext, SerumV3FulfillmentConfig}, get_then_update_id, load, load_mut, oracle::{get_oracle_price, HistoricalIndexData, HistoricalOracleData, OracleSource}, oracle_map::OracleMap, safe_decrement, spot_market::{AssetTier, MarketStatus, PoolBalance, SpotFulfillmentConfigStatus, SpotMarket}, utils::{constants::{DEFAULT_LIQUIDATION_MARGIN_BUFFER_RATIO, QUOTE_SPOT_MARKET_INDEX, SPOT_CUMULATIVE_INTEREST_PRECISION}, validation_utils::{validate_borrow_rate, validate_margin_weights}}, validate};



pub fn handle_admin_initialize(ctx: Context<Initialize>) -> Result<()> {
    let (drift_signer, drift_signer_nonce) =
        Pubkey::find_program_address(&[b"drift_signer".as_ref()], ctx.program_id);

    **ctx.accounts.state = DexState {
        admin: *ctx.accounts.admin.key,
        exchange_status: ExchangeStatus::active(),
        whitelist_mint: Pubkey::default(),
        discount_mint: Pubkey::default(),
        number_of_authorities: 0,
        number_of_sub_accounts: 0,
        number_of_markets: 0,
        number_of_spot_markets: 0,
        min_perp_auction_duration: 10,
        default_market_order_time_in_force: 60,
        default_spot_auction_duration: 10,
        liquidation_margin_buffer_ratio: DEFAULT_LIQUIDATION_MARGIN_BUFFER_RATIO,
        settlement_duration: 0, // extra duration after market expiry to allow settlement
        signer: drift_signer,
        signer_nonce: drift_signer_nonce,
        srm_vault: Pubkey::default(),
        perp_fee_structure: FeeStructure::perps_default(),
        spot_fee_structure: FeeStructure::spot_default(),
        lp_cooldown_time: 0,
        liquidation_duration: 0,
        initial_pct_to_liquidate: 0,
        max_initialize_user_fee: 0,
        padding: [0; 10],
    };

    Ok(())
}


pub fn handle_initialize_spot_market(
    ctx: Context<InitializeSpotMarket>,
    optimal_utilization: u32,
    optimal_borrow_rate: u32,
    max_borrow_rate: u32,
    oracle_source: OracleSource,
    initial_asset_weight: u32,
    maintenance_asset_weight: u32,
    initial_liability_weight: u32,
    maintenance_liability_weight: u32,
    imf_factor: u32,
    liquidator_fee: u32,
    if_liquidation_fee: u32,
    active_status: bool,
    asset_tier: AssetTier,
    scale_initial_asset_weight_start: u64,
    withdraw_guard_threshold: u64,
    order_tick_size: u64,
    order_step_size: u64,
    if_total_factor: u32,
    name: [u8; 32],
) -> Result<()> {
    let state = &mut ctx.accounts.state;
    let spot_market_pubkey = ctx.accounts.spot_market.key();

    // protocol must be authority of collateral vault
    if ctx.accounts.spot_market_vault.owner != state.signer {
        return Err(DexError::InvalidSpotMarketAuthority.into());
    }

    // protocol must be authority of collateral vault
    if ctx.accounts.insurance_fund_vault.owner != state.signer {
        return Err(DexError::InvalidInsuranceFundAuthority.into());
    }

    validate_borrow_rate(optimal_utilization, optimal_borrow_rate, max_borrow_rate, 0)?;

    let spot_market_index = get_then_update_id!(state, number_of_spot_markets);

    msg!("initializing spot market {}", spot_market_index);

    if oracle_source == OracleSource::QuoteAsset {
        // catches inconsistent parameters
        validate!(
            ctx.accounts.oracle.key == &Pubkey::default(),
            DexError::InvalidSpotMarketInitialization,
            "For OracleSource::QuoteAsset, oracle must be default public key"
        )?;

        validate!(
            spot_market_index == QUOTE_SPOT_MARKET_INDEX,
            DexError::InvalidSpotMarketInitialization,
            "For OracleSource::QuoteAsset, spot_market_index must be QUOTE_SPOT_MARKET_INDEX"
        )?;
    } else {
        OracleMap::validate_oracle_account_info(&ctx.accounts.oracle)?;
    }

    let oracle_price_data = get_oracle_price(
        &oracle_source,
        &ctx.accounts.oracle,
        Clock::get()?.unix_timestamp.cast()?,
    );

    let (historical_oracle_data_default, historical_index_data_default) =
        if spot_market_index == QUOTE_SPOT_MARKET_INDEX {
            validate!(
                ctx.accounts.oracle.key == &Pubkey::default(),
                DexError::InvalidSpotMarketInitialization,
                "For quote asset spot market, oracle must be default public key"
            )?;

            validate!(
                oracle_source == OracleSource::QuoteAsset,
                DexError::InvalidSpotMarketInitialization,
                "For quote asset spot market, oracle source must be QuoteAsset"
            )?;

            validate!(
                ctx.accounts.spot_market_mint.decimals == 6,
                DexError::InvalidSpotMarketInitialization,
                "For quote asset spot market, mint decimals must be 6"
            )?;

            (
                HistoricalOracleData::default_quote_oracle(),
                HistoricalIndexData::default_quote_oracle(),
            )
        } else {
            validate!(
                ctx.accounts.spot_market_mint.decimals >= 6,
                DexError::InvalidSpotMarketInitialization,
                "Mint decimals must be greater than or equal to 6"
            )?;

            validate!(
                oracle_price_data.is_ok(),
                DexError::InvalidSpotMarketInitialization,
                "Unable to read oracle price for {}",
                ctx.accounts.oracle.key,
            )?;

            (
                HistoricalOracleData::default_with_current_oracle(oracle_price_data?),
                HistoricalIndexData::default_with_current_oracle(oracle_price_data?)?,
            )
        };

    validate_margin_weights(
        spot_market_index,
        initial_asset_weight,
        maintenance_asset_weight,
        initial_liability_weight,
        maintenance_liability_weight,
        imf_factor,
    )?;

    let spot_market = &mut ctx.accounts.spot_market.load_init()?;
    let clock = Clock::get()?;
    let now = clock
        .unix_timestamp
        .cast()
        .or(Err(DexError::UnableToCastUnixTime))?;

    let decimals = ctx.accounts.spot_market_mint.decimals.cast::<u32>()?;

    let token_program = if ctx.accounts.token_program.key() == Token2022::id() {
        1_u8
    } else if ctx.accounts.token_program.key() == Token::id() {
        0_u8
    } else {
        msg!("unexpected program {:?}", ctx.accounts.token_program.key());
        return Err(DexError::DefaultError.into());
    };

    **spot_market = SpotMarket {
        market_index: spot_market_index,
        pubkey: spot_market_pubkey,
        status: if active_status {
            MarketStatus::Active
        } else {
            MarketStatus::Initialized
        },
        name,
        asset_tier,
        expiry_ts: 0,
        oracle: ctx.accounts.oracle.key(),
        oracle_source,
        historical_oracle_data: historical_oracle_data_default,
        historical_index_data: historical_index_data_default,
        mint: ctx.accounts.spot_market_mint.key(),
        vault: *ctx.accounts.spot_market_vault.to_account_info().key,
        revenue_pool: PoolBalance {
            scaled_balance: 0,
            market_index: spot_market_index,
            ..PoolBalance::default()
        }, // in base asset
        decimals,
        optimal_utilization,
        optimal_borrow_rate,
        max_borrow_rate,
        deposit_balance: 0,
        borrow_balance: 0,
        max_token_deposits: 0,
        deposit_token_twap: 0,
        borrow_token_twap: 0,
        utilization_twap: 0,
        cumulative_deposit_interest: SPOT_CUMULATIVE_INTEREST_PRECISION,
        cumulative_borrow_interest: SPOT_CUMULATIVE_INTEREST_PRECISION,
        total_social_loss: 0,
        total_quote_social_loss: 0,
        last_interest_ts: now,
        last_twap_ts: now,
        initial_asset_weight,
        maintenance_asset_weight,
        initial_liability_weight,
        maintenance_liability_weight,
        imf_factor,
        liquidator_fee,
        if_liquidation_fee, // 1%
        withdraw_guard_threshold,
        order_step_size,
        order_tick_size,
        min_order_size: order_step_size,
        max_position_size: 0,
        next_fill_record_id: 1,
        next_deposit_record_id: 1,
        spot_fee_pool: PoolBalance::default(), // in quote asset
        total_spot_fee: 0,
        orders_enabled: spot_market_index != 0,
        paused_operations: 0,
        if_paused_operations: 0,
        fee_adjustment: 0,
        max_token_borrows_fraction: 0,
        flash_loan_amount: 0,
        flash_loan_initial_token_amount: 0,
        total_swap_fee: 0,
        scale_initial_asset_weight_start,
        min_borrow_rate: 0,
        fuel_boost_deposits: 0,
        fuel_boost_borrows: 0,
        fuel_boost_taker: 0,
        fuel_boost_maker: 0,
        fuel_boost_insurance: 0,
        token_program,
        padding: [0; 41],
    };

    Ok(())
}


#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        init,
        seeds = [b"drift_state".as_ref()],
        space = DexState::SIZE,
        bump,
        payer = admin
    )]
    pub state: Box<Account<'info, DexState>>,
    pub quote_asset_mint: Box<InterfaceAccount<'info, Mint>>,
    /// CHECK: checked in `initialize`
    pub drift_signer: AccountInfo<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct InitializeSpotMarket<'info> {
    #[account(
        init,
        seeds = [b"spot_market", state.number_of_spot_markets.to_le_bytes().as_ref()],
        space = SpotMarket::SIZE,
        bump,
        payer = admin
    )]
    pub spot_market: AccountLoader<'info, SpotMarket>,
    pub spot_market_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        seeds = [b"spot_market_vault".as_ref(), state.number_of_spot_markets.to_le_bytes().as_ref()],
        bump,
        payer = admin,
        token::mint = spot_market_mint,
        token::authority = drift_signer
    )]
    pub spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        init,
        seeds = [b"insurance_fund_vault".as_ref(), state.number_of_spot_markets.to_le_bytes().as_ref()],
        bump,
        payer = admin,
        token::mint = spot_market_mint,
        token::authority = drift_signer
    )]
    pub insurance_fund_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        constraint = state.signer.eq(&drift_signer.key())
    )]
    /// CHECK: program signer
    pub drift_signer: AccountInfo<'info>,
    #[account(
        mut,
        has_one = admin
    )]
    pub state: Box<Account<'info, DexState>>,
    /// CHECK: checked in `initialize_spot_market`
    pub oracle: AccountInfo<'info>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}


pub fn handle_delete_initialized_spot_market(
    ctx: Context<DeleteInitializedSpotMarket>,
    market_index: u16,
) -> Result<()> {
    let spot_market = ctx.accounts.spot_market.load()?;
    msg!("spot market {}", spot_market.market_index);
    let state = &mut ctx.accounts.state;

    // to preserve all protocol invariants, can only remove the last market if it hasn't been "activated"

    validate!(
        state.number_of_spot_markets - 1 == market_index,
        DexError::InvalidMarketAccountforDeletion,
        "state.number_of_spot_markets={} != market_index={}",
        state.number_of_markets,
        market_index
    )?;
    validate!(
        spot_market.status == MarketStatus::Initialized,
        DexError::InvalidMarketAccountforDeletion,
        "spot_market.status != Initialized",
    )?;
    validate!(
        spot_market.deposit_balance == 0,
        DexError::InvalidMarketAccountforDeletion,
        "spot_market.number_of_users={} != 0",
        spot_market.deposit_balance,
    )?;
    validate!(
        spot_market.borrow_balance == 0,
        DexError::InvalidMarketAccountforDeletion,
        "spot_market.borrow_balance={} != 0",
        spot_market.borrow_balance,
    )?;
    validate!(
        spot_market.market_index == market_index,
        DexError::InvalidMarketAccountforDeletion,
        "market_index={} != spot_market.market_index={}",
        market_index,
        spot_market.market_index
    )?;

    safe_decrement!(state.number_of_spot_markets, 1);

    drop(spot_market);

    validate!(
        ctx.accounts.spot_market_vault.amount == 0,
        DexError::InvalidMarketAccountforDeletion,
        "ctx.accounts.spot_market_vault.amount={}",
        ctx.accounts.spot_market_vault.amount
    )?;

    close_vault(
        &ctx.accounts.token_program,
        &ctx.accounts.spot_market_vault,
        &ctx.accounts.admin.to_account_info(),
        &ctx.accounts.drift_signer,
        state.signer_nonce,
    )?;

    validate!(
        ctx.accounts.insurance_fund_vault.amount == 0,
        DexError::InvalidMarketAccountforDeletion,
        "ctx.accounts.insurance_fund_vault.amount={}",
        ctx.accounts.insurance_fund_vault.amount
    )?;

    close_vault(
        &ctx.accounts.token_program,
        &ctx.accounts.insurance_fund_vault,
        &ctx.accounts.admin.to_account_info(),
        &ctx.accounts.drift_signer,
        state.signer_nonce,
    )?;

    Ok(())
}


pub fn handle_initialize_serum_fulfillment_config(
    ctx: Context<InitializeSerumFulfillmentConfig>,
    market_index: u16,
) -> Result<()> {
    validate!(
        market_index != QUOTE_SPOT_MARKET_INDEX,
        DexError::InvalidSpotMarketAccount,
        "Cant add serum market to quote asset"
    )?;

    let base_spot_market = load!(&ctx.accounts.base_spot_market)?;
    let quote_spot_market = load!(&ctx.accounts.quote_spot_market)?;

    let serum_program_id = crate::ids::serum_program::id();
    validate!(
        ctx.accounts.serum_program.key() == serum_program_id,
        DexError::InvalidSerumProgram
    )?;

    let serum_market_key = ctx.accounts.serum_market.key();

    let serum_context = SerumContext {
        serum_program: &ctx.accounts.serum_program,
        serum_market: &ctx.accounts.serum_market,
        serum_open_orders: &ctx.accounts.serum_open_orders,
    };

    let market_state = serum_context.load_serum_market()?;

    validate!(
        identity(market_state.coin_mint) == base_spot_market.mint.to_aligned_bytes(),
        DexError::InvalidSerumMarket,
        "Invalid base mint"
    )?;

    validate!(
        identity(market_state.pc_mint) == quote_spot_market.mint.to_aligned_bytes(),
        DexError::InvalidSerumMarket,
        "Invalid quote mint"
    )?;

    let market_step_size = market_state.coin_lot_size;
    let valid_step_size = base_spot_market.order_step_size >= market_step_size
        && base_spot_market
            .order_step_size
            .rem_euclid(market_step_size)
            == 0;

    validate!(
        valid_step_size,
        DexError::InvalidSerumMarket,
        "base market step size ({}) not a multiple of serum step size ({})",
        base_spot_market.order_step_size,
        market_step_size
    )?;

    let market_tick_size = market_state.pc_lot_size;
    let valid_tick_size = base_spot_market.order_tick_size >= market_tick_size
        && base_spot_market
            .order_tick_size
            .rem_euclid(market_tick_size)
            == 0;

    validate!(
        valid_tick_size,
        DexError::InvalidSerumMarket,
        "base market tick size ({}) not a multiple of serum tick size ({})",
        base_spot_market.order_tick_size,
        market_tick_size
    )?;

    drop(market_state);

    let open_orders_seeds: &[&[u8]] = &[b"serum_open_orders", serum_market_key.as_ref()];
    controllers::pda::seed_and_create_pda(
        ctx.program_id,
        &ctx.accounts.admin.to_account_info(),
        &Rent::get()?,
        size_of::<serum_dex::state::OpenOrders>() + 12,
        &serum_program_id,
        &ctx.accounts.system_program.to_account_info(),
        &ctx.accounts.serum_open_orders,
        open_orders_seeds,
    )?;

    let open_orders = serum_context.load_open_orders()?;
    validate!(
        open_orders.account_flags == 0,
        DexError::InvalidSerumOpenOrders,
        "Serum open orders already initialized"
    )?;
    drop(open_orders);

    serum_context.invoke_init_open_orders(
        &ctx.accounts.drift_signer,
        &ctx.accounts.rent,
        ctx.accounts.state.signer_nonce,
    )?;

    let serum_fulfillment_config_key = ctx.accounts.serum_fulfillment_config.key();
    let mut serum_fulfillment_config = ctx.accounts.serum_fulfillment_config.load_init()?;
    *serum_fulfillment_config = serum_context
        .to_serum_v3_fulfillment_config(&serum_fulfillment_config_key, market_index)?;

    Ok(())
}

pub fn handle_update_serum_fulfillment_config_status(
    ctx: Context<UpdateSerumFulfillmentConfig>,
    status: SpotFulfillmentConfigStatus,
) -> Result<()> {
    let mut config = load_mut!(ctx.accounts.serum_fulfillment_config)?;
    msg!("config.status {:?} -> {:?}", config.status, status);
    config.status = status;
    Ok(())
}

pub fn handle_update_serum_vault(ctx: Context<UpdateSerumVault>) -> Result<()> {
    let vault = &ctx.accounts.srm_vault;
    validate!(
        vault.mint == crate::ids::srm_mint::id() || vault.mint == crate::ids::msrm_mint::id(),
        DexError::InvalidSrmVault,
        "vault did not hav srm or msrm mint"
    )?;

    validate!(
        vault.owner == ctx.accounts.state.signer,
        DexError::InvalidVaultOwner,
        "vault owner was not program signer"
    )?;

    let state = &mut ctx.accounts.state;

    msg!("state.srm_vault {:?} -> {:?}", state.srm_vault, vault.key());
    state.srm_vault = vault.key();

    Ok(())
}

#[derive(Accounts)]
#[instruction(market_index: u16)]
pub struct DeleteInitializedSpotMarket<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        mut,
        has_one = admin
    )]
    pub state: Box<Account<'info, DexState>>,
    #[account(mut, close = admin)]
    pub spot_market: AccountLoader<'info, SpotMarket>,
    #[account(
        mut,
        seeds = [b"spot_market_vault".as_ref(), market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [b"insurance_fund_vault".as_ref(), market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub insurance_fund_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: program signer
    pub drift_signer: AccountInfo<'info>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
#[instruction(market_index: u16)]
pub struct InitializeSerumFulfillmentConfig<'info> {
    #[account(
        seeds = [b"spot_market", market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub base_spot_market: AccountLoader<'info, SpotMarket>,
    #[account(
        seeds = [b"spot_market", 0_u16.to_le_bytes().as_ref()],
        bump,
    )]
    pub quote_spot_market: AccountLoader<'info, SpotMarket>,
    #[account(
        mut,
        has_one = admin
    )]
    pub state: Box<Account<'info, DexState>>,
    /// CHECK: checked in ix
    pub serum_program: AccountInfo<'info>,
    /// CHECK: checked in ix
    pub serum_market: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [b"serum_open_orders".as_ref(), serum_market.key.as_ref()],
        bump,
    )]
    /// CHECK: checked in ix
    pub serum_open_orders: AccountInfo<'info>,
    #[account(
        constraint = state.signer.eq(&drift_signer.key())
    )]
    /// CHECK: program signer
    pub drift_signer: AccountInfo<'info>,
    #[account(
        init,
        seeds = [b"serum_fulfillment_config".as_ref(), serum_market.key.as_ref()],
        space = SerumV3FulfillmentConfig::SIZE,
        bump,
        payer = admin,
    )]
    pub serum_fulfillment_config: AccountLoader<'info, SerumV3FulfillmentConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateSerumFulfillmentConfig<'info> {
    #[account(
        has_one = admin
    )]
    pub state: Box<Account<'info, DexState>>,
    #[account(mut)]
    pub serum_fulfillment_config: AccountLoader<'info, SerumV3FulfillmentConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
}


#[derive(Accounts)]
pub struct UpdateSerumVault<'info> {
    #[account(
        mut,
        has_one = admin
    )]
    pub state: Box<Account<'info, DexState>>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub srm_vault: Box<InterfaceAccount<'info, TokenAccount>>,
}