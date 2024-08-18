#![allow(clippy::too_many_arguments)]
#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::comparison_chain)]
use anchor_lang::prelude::*;


pub mod errors;
pub mod state;
pub mod utils;
pub mod instructions;
pub mod macros;
pub mod ids;
pub mod safe_methods;
pub mod casting;
pub mod controllers;
use crate::instructions::{user::*, admins::*};
use crate::state::*;


#[cfg(feature = "devnet")]
declare_id!("HkApQpEsdzdfHsedkuZvNEbmcQXfabobbb9Yf8wdz7AZ");

#[program]
pub mod vortex_contracts {

    use super::*;
    use instructions::{admins::{handle_admin_initialize, handle_initialize_spot_market, Initialize}, executors::SpotFulfillmentType};
    use oracle::OracleSource;
    use order_params::{ModifyOrderParams, OrderParams};
    use position::PositionDirection;
    use spot_market::{AssetTier, SpotFulfillmentConfigStatus};
    use user::MarketType;

    
    // User instructions

    pub fn initialize_user<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, InitializeUserAccount<'info>>,
        name: [u8; 32],
    ) -> Result<()> {
        initialize_new_user_account(ctx, name)
    }


    pub fn initialize_user_stats<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, InitializeUserStats>,
    ) -> Result<()> {
        handle_initialize_user_stats(ctx)
    }


    pub fn deposit<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, Deposit<'info>>,
        market_index: u16,
        amount: u64,
        reduce_only: bool,
    ) -> Result<()> {
        handle_deposit(ctx, market_index, amount, reduce_only)
    }

    pub fn withdraw<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, Withdraw<'info>>,
        market_index: u16,
        amount: u64,
        reduce_only: bool,
    ) -> anchor_lang::Result<()> {
        handle_withdraw(ctx, market_index, amount, reduce_only)
    }

    pub fn transfer_deposit<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, TransferDeposit<'info>>,
        market_index: u16,
        amount: u64,
    ) -> anchor_lang::Result<()> {
        handle_transfer_deposit(ctx, market_index, amount)
    }

    pub fn cancel_order<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, CancelOrder>,
        order_id: Option<u32>,
    ) -> Result<()> {
        handle_cancel_order(ctx, order_id)
    }

    pub fn cancel_order_by_user_id<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, CancelOrder>,
        user_order_id: u8,
    ) -> Result<()> {
        handle_cancel_order_by_user_id(ctx, user_order_id)
    }

    pub fn cancel_orders<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, CancelOrder<'info>>,
        market_type: Option<MarketType>,
        market_index: Option<u16>,
        direction: Option<PositionDirection>,
    ) -> Result<()> {
        handle_cancel_orders(ctx, market_type, market_index, direction)
    }

    pub fn cancel_orders_by_ids<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, CancelOrder>,
        order_ids: Vec<u32>,
    ) -> Result<()> {
        handle_cancel_orders_by_ids(ctx, order_ids)
    }

    pub fn modify_order<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, CancelOrder<'info>>,
        order_id: Option<u32>,
        modify_order_params: ModifyOrderParams,
    ) -> Result<()> {
        handle_modify_order(ctx, order_id, modify_order_params)
    }

    pub fn modify_order_by_user_id<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, CancelOrder<'info>>,
        user_order_id: u8,
        modify_order_params: ModifyOrderParams,
    ) -> Result<()> {
        handle_modify_order_by_user_order_id(ctx, user_order_id, modify_order_params)
    }
    

    pub fn place_spot_order<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, PlaceOrder>,
        params: OrderParams,
    ) -> Result<()> {
        handle_place_spot_order(ctx, params)
    }

    pub fn place_and_take_spot_order<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, PlaceAndTake<'info>>,
        params: OrderParams,
        fulfillment_type: Option<SpotFulfillmentType>,
        maker_order_id: Option<u32>,
    ) -> Result<()> {
        handle_place_and_take_spot_order(
            ctx,
            params,
            fulfillment_type.unwrap_or(SpotFulfillmentType::Match),
            maker_order_id,
        )
    }

    pub fn place_and_make_spot_order<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, PlaceAndMake<'info>>,
        params: OrderParams,
        taker_order_id: u32,
        fulfillment_type: Option<SpotFulfillmentType>,
    ) -> Result<()> {
        handle_place_and_make_spot_order(
            ctx,
            params,
            taker_order_id,
            fulfillment_type.unwrap_or(SpotFulfillmentType::Match),
        )
    }

    pub fn place_orders<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, PlaceOrder>,
        params: Vec<OrderParams>,
    ) -> Result<()> {
        handle_place_orders(ctx, params)
    }

    pub fn begin_swap<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, Swap<'info>>,
        in_market_index: u16,
        out_market_index: u16,
        amount_in: u64,
    ) -> Result<()> {
        handle_begin_swap(ctx, in_market_index, out_market_index, amount_in)
    }

    pub fn end_swap<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, Swap<'info>>,
        in_market_index: u16,
        out_market_index: u16,
        limit_price: Option<u64>,
        reduce_only: Option<SwapReduceOnly>,
    ) -> Result<()> {
        handle_end_swap(
            ctx,
            in_market_index,
            out_market_index,
            limit_price,
            reduce_only,
        )
    }


    pub fn update_user_name(
        ctx: Context<UpdateUser>,
        _sub_account_id: u16,
        name: [u8; 32],
    ) -> Result<()> {
        handle_update_user_name(ctx, _sub_account_id, name)
    }

    pub fn update_user_custom_margin_ratio(
        ctx: Context<UpdateUser>,
        _sub_account_id: u16,
        margin_ratio: u32,
    ) -> Result<()> {
        handle_update_user_custom_margin_ratio(ctx, _sub_account_id, margin_ratio)
    }

    pub fn update_user_margin_trading_enabled<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, UpdateUser<'info>>,
        _sub_account_id: u16,
        margin_trading_enabled: bool,
    ) -> Result<()> {
        handle_update_user_margin_trading_enabled(ctx, _sub_account_id, margin_trading_enabled)
    }

    pub fn update_user_delegate(
        ctx: Context<UpdateUser>,
        _sub_account_id: u16,
        delegate: Pubkey,
    ) -> Result<()> {
        handle_update_user_delegate(ctx, _sub_account_id, delegate)
    }

    pub fn update_user_reduce_only(
        ctx: Context<UpdateUser>,
        _sub_account_id: u16,
        reduce_only: bool,
    ) -> Result<()> {
        handle_update_user_reduce_only(ctx, _sub_account_id, reduce_only)
    }

    pub fn update_user_advanced_lp(
        ctx: Context<UpdateUser>,
        _sub_account_id: u16,
        advanced_lp: bool,
    ) -> Result<()> {
        handle_update_user_advanced_lp(ctx, _sub_account_id, advanced_lp)
    }

    pub fn delete_user<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, DeleteUser>,
    ) -> Result<()> {
        handle_delete_user(ctx)
    }

    pub fn reclaim_rent(ctx: Context<ReclaimRent>) -> Result<()> {
        handle_reclaim_rent(ctx)
    }

    // Admin instructions
    // will initialize different trading markets from here 


    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        handle_admin_initialize(ctx)
    }

    pub fn initialize_spot_market(
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
        handle_initialize_spot_market(
            ctx,
            optimal_utilization,
            optimal_borrow_rate,
            max_borrow_rate,
            oracle_source,
            initial_asset_weight,
            maintenance_asset_weight,
            initial_liability_weight,
            maintenance_liability_weight,
            imf_factor,
            liquidator_fee,
            if_liquidation_fee,
            active_status,
            asset_tier,
            scale_initial_asset_weight_start,
            withdraw_guard_threshold,
            order_tick_size,
            order_step_size,
            if_total_factor,
            name,
        )
    }

    pub fn delete_initialized_spot_market(
        ctx: Context<DeleteInitializedSpotMarket>,
        market_index: u16,
    ) -> Result<()> {
        handle_delete_initialized_spot_market(ctx, market_index)
    }

    pub fn initialize_serum_fulfillment_config(
        ctx: Context<InitializeSerumFulfillmentConfig>,
        market_index: u16,
    ) -> Result<()> {
        handle_initialize_serum_fulfillment_config(ctx, market_index)
    }

    pub fn update_serum_fulfillment_config_status(
        ctx: Context<UpdateSerumFulfillmentConfig>,
        status: SpotFulfillmentConfigStatus,
    ) -> Result<()> {
        handle_update_serum_fulfillment_config_status(ctx, status)
    }


    pub fn update_serum_vault(ctx: Context<UpdateSerumVault>) -> Result<()> {
        handle_update_serum_vault(ctx)
    }


    pub fn settle_expired_market_pools_to_revenue_pool(
        ctx: Context<SettleExpiredMarketPoolsToRevenuePool>,
    ) -> Result<()> {
        handle_settle_expired_market_pools_to_revenue_pool(ctx)
    }

    pub fn deposit_into_spot_market_vault<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, DepositIntoSpotMarketVault<'info>>,
        amount: u64,
    ) -> Result<()> {
        handle_deposit_into_spot_market_vault(ctx, amount)
    }

    pub fn deposit_into_spot_market_revenue_pool<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, RevenuePoolDeposit<'info>>,
        amount: u64,
    ) -> Result<()> {
        handle_deposit_into_spot_market_revenue_pool(ctx, amount)
    }

    pub fn repeg_amm_curve(ctx: Context<RepegCurve>, new_peg_candidate: u128) -> Result<()> {
        handle_repeg_amm_curve(ctx, new_peg_candidate)
    }

    pub fn update_k(ctx: Context<AdminUpdateK>, sqrt_k: u128) -> Result<()> {
        handle_update_k(ctx, sqrt_k)
    }


    pub fn update_spot_market_liquidation_fee(
        ctx: Context<AdminUpdateSpotMarket>,
        liquidator_fee: u32,
        if_liquidation_fee: u32,
    ) -> Result<()> {
        handle_update_spot_market_liquidation_fee(ctx, liquidator_fee, if_liquidation_fee)
    }

    pub fn update_withdraw_guard_threshold(
        ctx: Context<AdminUpdateSpotMarket>,
        withdraw_guard_threshold: u64,
    ) -> Result<()> {
        handle_update_withdraw_guard_threshold(ctx, withdraw_guard_threshold)
    }

    pub fn update_spot_market_if_factor(
        ctx: Context<AdminUpdateSpotMarket>,
        spot_market_index: u16,
        user_if_factor: u32,
        total_if_factor: u32,
    ) -> Result<()> {
        handle_update_spot_market_if_factor(ctx, spot_market_index, user_if_factor, total_if_factor)
    }

    pub fn update_spot_market_revenue_settle_period(
        ctx: Context<AdminUpdateSpotMarket>,
        revenue_settle_period: i64,
    ) -> Result<()> {
        handle_update_spot_market_revenue_settle_period(ctx, revenue_settle_period)
    }

    pub fn update_spot_market_status(
        ctx: Context<AdminUpdateSpotMarket>,
        status: MarketStatus,
    ) -> Result<()> {
        handle_update_spot_market_status(ctx, status)
    }

    pub fn update_spot_market_paused_operations(
        ctx: Context<AdminUpdateSpotMarket>,
        paused_operations: u8,
    ) -> Result<()> {
        handle_update_spot_market_paused_operations(ctx, paused_operations)
    }

    pub fn update_spot_market_asset_tier(
        ctx: Context<AdminUpdateSpotMarket>,
        asset_tier: AssetTier,
    ) -> Result<()> {
        handle_update_spot_market_asset_tier(ctx, asset_tier)
    }

    pub fn update_spot_market_margin_weights(
        ctx: Context<AdminUpdateSpotMarket>,
        initial_asset_weight: u32,
        maintenance_asset_weight: u32,
        initial_liability_weight: u32,
        maintenance_liability_weight: u32,
        imf_factor: u32,
    ) -> Result<()> {
        handle_update_spot_market_margin_weights(
            ctx,
            initial_asset_weight,
            maintenance_asset_weight,
            initial_liability_weight,
            maintenance_liability_weight,
            imf_factor,
        )
    }

    pub fn update_spot_market_borrow_rate(
        ctx: Context<AdminUpdateSpotMarket>,
        optimal_utilization: u32,
        optimal_borrow_rate: u32,
        max_borrow_rate: u32,
        min_borrow_rate: Option<u8>,
    ) -> Result<()> {
        handle_update_spot_market_borrow_rate(
            ctx,
            optimal_utilization,
            optimal_borrow_rate,
            max_borrow_rate,
            min_borrow_rate,
        )
    }

    pub fn update_spot_market_max_token_deposits(
        ctx: Context<AdminUpdateSpotMarket>,
        max_token_deposits: u64,
    ) -> Result<()> {
        handle_update_spot_market_max_token_deposits(ctx, max_token_deposits)
    }

    pub fn update_spot_market_max_token_borrows(
        ctx: Context<AdminUpdateSpotMarket>,
        max_token_borrows_fraction: u16,
    ) -> Result<()> {
        handle_update_spot_market_max_token_borrows(ctx, max_token_borrows_fraction)
    }

    pub fn update_spot_market_scale_initial_asset_weight_start(
        ctx: Context<AdminUpdateSpotMarket>,
        scale_initial_asset_weight_start: u64,
    ) -> Result<()> {
        handle_update_spot_market_scale_initial_asset_weight_start(
            ctx,
            scale_initial_asset_weight_start,
        )
    }

    pub fn update_spot_market_oracle(
        ctx: Context<AdminUpdateSpotMarketOracle>,
        oracle: Pubkey,
        oracle_source: OracleSource,
    ) -> Result<()> {
        handle_update_spot_market_oracle(ctx, oracle, oracle_source)
    }

    pub fn update_spot_market_step_size_and_tick_size(
        ctx: Context<AdminUpdateSpotMarket>,
        step_size: u64,
        tick_size: u64,
    ) -> Result<()> {
        handle_update_spot_market_step_size_and_tick_size(ctx, step_size, tick_size)
    }

    pub fn update_spot_market_min_order_size(
        ctx: Context<AdminUpdateSpotMarket>,
        order_size: u64,
    ) -> Result<()> {
        handle_update_spot_market_min_order_size(ctx, order_size)
    }

    pub fn update_spot_market_orders_enabled(
        ctx: Context<AdminUpdateSpotMarket>,
        orders_enabled: bool,
    ) -> Result<()> {
        handle_update_spot_market_orders_enabled(ctx, orders_enabled)
    }

    pub fn update_spot_market_if_paused_operations(
        ctx: Context<AdminUpdateSpotMarket>,
        paused_operations: u8,
    ) -> Result<()> {
        handle_update_spot_market_if_paused_operations(ctx, paused_operations)
    }

    pub fn update_spot_market_name(
        ctx: Context<AdminUpdateSpotMarket>,
        name: [u8; 32],
    ) -> Result<()> {
        handle_update_spot_market_name(ctx, name)
    }


    pub fn update_spot_fee_structure(
        ctx: Context<AdminUpdateState>,
        fee_structure: FeeStructure,
    ) -> Result<()> {
        handle_update_spot_fee_structure(ctx, fee_structure)
    }

    pub fn update_initial_pct_to_liquidate(
        ctx: Context<AdminUpdateState>,
        initial_pct_to_liquidate: u16,
    ) -> Result<()> {
        handle_update_initial_pct_to_liquidate(ctx, initial_pct_to_liquidate)
    }

    pub fn update_liquidation_duration(
        ctx: Context<AdminUpdateState>,
        liquidation_duration: u8,
    ) -> Result<()> {
        handle_update_liquidation_duration(ctx, liquidation_duration)
    }

    pub fn update_liquidation_margin_buffer_ratio(
        ctx: Context<AdminUpdateState>,
        liquidation_margin_buffer_ratio: u32,
    ) -> Result<()> {
        handle_update_liquidation_margin_buffer_ratio(ctx, liquidation_margin_buffer_ratio)
    }

    pub fn update_oracle_guard_rails(
        ctx: Context<AdminUpdateState>,
        oracle_guard_rails: OracleGuardRails,
    ) -> Result<()> {
        handle_update_oracle_guard_rails(ctx, oracle_guard_rails)
    }

    pub fn update_state_settlement_duration(
        ctx: Context<AdminUpdateState>,
        settlement_duration: u16,
    ) -> Result<()> {
        handle_update_state_settlement_duration(ctx, settlement_duration)
    }

    pub fn update_state_max_number_of_sub_accounts(
        ctx: Context<AdminUpdateState>,
        max_number_of_sub_accounts: u16,
    ) -> Result<()> {
        handle_update_state_max_number_of_sub_accounts(ctx, max_number_of_sub_accounts)
    }

    pub fn update_state_max_initialize_user_fee(
        ctx: Context<AdminUpdateState>,
        max_initialize_user_fee: u16,
    ) -> Result<()> {
        handle_update_state_max_initialize_user_fee(ctx, max_initialize_user_fee)
    }


    pub fn update_amm_jit_intensity(
        ctx: Context<AdminUpdatePerpMarket>,
        amm_jit_intensity: u8,
    ) -> Result<()> {
        handle_update_amm_jit_intensity(ctx, amm_jit_intensity)
    }


    pub fn update_spot_market_fuel(
        ctx: Context<AdminUpdateSpotMarket>,
        fuel_boost_deposits: Option<u8>,
        fuel_boost_borrows: Option<u8>,
        fuel_boost_taker: Option<u8>,
        fuel_boost_maker: Option<u8>,
        fuel_boost_insurance: Option<u8>,
    ) -> Result<()> {
        handle_update_spot_market_fuel(
            ctx,
            fuel_boost_deposits,
            fuel_boost_borrows,
            fuel_boost_taker,
            fuel_boost_maker,
            fuel_boost_insurance,
        )
    }

    pub fn init_user_fuel(
        ctx: Context<InitUserFuel>,
        fuel_boost_deposits: Option<u32>,
        fuel_boost_borrows: Option<u32>,
        fuel_boost_taker: Option<u32>,
        fuel_boost_maker: Option<u32>,
        fuel_boost_insurance: Option<u32>,
    ) -> Result<()> {
        handle_init_user_fuel(
            ctx,
            fuel_boost_deposits,
            fuel_boost_borrows,
            fuel_boost_taker,
            fuel_boost_maker,
            fuel_boost_insurance,
        )
    }

    pub fn update_admin(ctx: Context<AdminUpdateState>, admin: Pubkey) -> Result<()> {
        handle_update_admin(ctx, admin)
    }

    pub fn update_whitelist_mint(
        ctx: Context<AdminUpdateState>,
        whitelist_mint: Pubkey,
    ) -> Result<()> {
        handle_update_whitelist_mint(ctx, whitelist_mint)
    }

    pub fn update_discount_mint(
        ctx: Context<AdminUpdateState>,
        discount_mint: Pubkey,
    ) -> Result<()> {
        handle_update_discount_mint(ctx, discount_mint)
    }

    pub fn update_exchange_status(
        ctx: Context<AdminUpdateState>,
        exchange_status: u8,
    ) -> Result<()> {
        handle_update_exchange_status(ctx, exchange_status)
    }


    pub fn update_spot_auction_duration(
        ctx: Context<AdminUpdateState>,
        default_spot_auction_duration: u8,
    ) -> Result<()> {
        handle_update_spot_auction_duration(ctx, default_spot_auction_duration)
    }

    pub fn initialize_protocol_if_shares_transfer_config(
        ctx: Context<InitializeProtocolIfSharesTransferConfig>,
    ) -> Result<()> {
        handle_initialize_protocol_if_shares_transfer_config(ctx)
    }

    pub fn update_protocol_if_shares_transfer_config(
        ctx: Context<UpdateProtocolIfSharesTransferConfig>,
        whitelisted_signers: Option<[Pubkey; 4]>,
        max_transfer_per_epoch: Option<u128>,
    ) -> Result<()> {
        handle_update_protocol_if_shares_transfer_config(
            ctx,
            whitelisted_signers,
            max_transfer_per_epoch,
        )
    }

    pub fn initialize_prelaunch_oracle(
        ctx: Context<InitializePrelaunchOracle>,
        params: PrelaunchOracleParams,
    ) -> Result<()> {
        handle_initialize_prelaunch_oracle(ctx, params)
    }

    pub fn update_prelaunch_oracle_params(
        ctx: Context<UpdatePrelaunchOracleParams>,
        params: PrelaunchOracleParams,
    ) -> Result<()> {
        handle_update_prelaunch_oracle_params(ctx, params)
    }

    pub fn delete_prelaunch_oracle(
        ctx: Context<DeletePrelaunchOracle>,
        perp_market_index: u16,
    ) -> Result<()> {
        handle_delete_prelaunch_oracle(ctx, perp_market_index)
    }

    pub fn initialize_pyth_pull_oracle(
        ctx: Context<InitPythPullPriceFeed>,
        feed_id: [u8; 32],
    ) -> Result<()> {
        handle_initialize_pyth_pull_oracle(ctx, feed_id)
    }

    // trader bots
    // bots will be responsible for taking orders and completing them
    // will also do settlement like P&L settlement
}