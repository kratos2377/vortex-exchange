#![allow(clippy::too_many_arguments)]
#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::comparison_chain)]
use anchor_lang::prelude::*;
use dex_state::{FeeStructure, OracleGuardRails};
use game_stake::BetType;
use oracle::OracleSource;
use order_params::{ModifyOrderParams, OrderParams};
use position::PositionDirection;
use spot_market::{AssetTier, MarketStatus, SpotFulfillmentConfigStatus};
use user::MarketType;

pub mod errors;
pub mod state;
pub mod utils;
pub mod instructions;
pub mod macros;
pub mod ids;
pub mod safe_methods;
pub mod casting;
pub mod controllers;
use crate::instructions::{user::*, admins::*, executors::*, game_stake::*};
use crate::state::*;


#[cfg(feature = "devnet")]
declare_id!("HkApQpEsdzdfHsedkuZvNEbmcQXfabobbb9Yf8wdz7AZ");

#[program]
pub mod vortex_contracts {

    use super::*;


    
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


    pub fn update_state_max_initialize_user_fee(
        ctx: Context<AdminUpdateState>,
        max_initialize_user_fee: u16,
    ) -> Result<()> {
        handle_update_state_max_initialize_user_fee(ctx, max_initialize_user_fee)
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

    // pub fn initialize_pyth_pull_oracle(
    //     ctx: Context<InitPythPullPriceFeed>,
    //     feed_id: [u8; 32],
    // ) -> Result<()> {
    //     handle_initialize_pyth_pull_oracle(ctx, feed_id)
    // }

    // trader bots
    // bots will be responsible for taking orders and completing them
    // will also do settlement like P&L settlement

    pub fn fill_spot_order<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, FillOrder<'info>>,
        order_id: Option<u32>,
        fulfillment_type: Option<SpotFulfillmentType>,
        maker_order_id: Option<u32>,
    ) -> Result<()> {
        handle_fill_spot_order(ctx, order_id, fulfillment_type, maker_order_id)
    }

    pub fn trigger_order<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, TriggerOrder<'info>>,
        order_id: u32,
    ) -> Result<()> {
        handle_trigger_order(ctx, order_id)
    }

    pub fn force_cancel_orders<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, ForceCancelOrder<'info>>,
    ) -> Result<()> {
        handle_force_cancel_orders(ctx)
    }

    pub fn update_user_idle<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, UpdateUserIdle<'info>>,
    ) -> Result<()> {
        handle_update_user_idle(ctx)
    }

    pub fn update_user_open_orders_count(ctx: Context<UpdateUserIdle>) -> Result<()> {
        handle_update_user_open_orders_count(ctx)
    }

    pub fn liquidate_spot<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, LiquidateSpot<'info>>,
        asset_market_index: u16,
        liability_market_index: u16,
        liquidator_max_liability_transfer: u128,
        limit_price: Option<u64>, // asset/liaiblity
    ) -> Result<()> {
        handle_liquidate_spot(
            ctx,
            asset_market_index,
            liability_market_index,
            liquidator_max_liability_transfer,
            limit_price,
        )
    }

    pub fn resolve_spot_bankruptcy<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, ResolveBankruptcy<'info>>,
        market_index: u16,
    ) -> Result<()> {
        handle_resolve_spot_bankruptcy(ctx, market_index)
    }

    pub fn update_spot_market_expiry(
        ctx: Context<AdminUpdateSpotMarket>,
        expiry_ts: i64,
    ) -> Result<()> {
        handle_update_spot_market_expiry(ctx, expiry_ts)
    }

    // Game Stake fns
    pub fn initialize_game(
        ctx: Context<InitGame>,
        game_id: [u8; 16],
        total_money_staked: u64,
    
    ) -> Result<()> {
        handle_init_game(ctx, game_id, total_money_staked)
    }


    pub fn initialize_player_bet(
        ctx: Context<InitPlayerBet>,
        game_id: [u8; 16],
        total_money_staked: u64,
        user_betting_on_id: [u8;16]
    
    ) -> Result<()> {
        handle_init_player_bet(ctx, game_id, total_money_staked, user_betting_on_id)
    }


    
    pub fn handle_user_bet(
        ctx: Context<MakeUserGameBet>,
        game_id: [u8; 16],
        user_betting_on_id: [u8;16],
        money_staked: u64,
        bet_type: BetType
    
    ) -> Result<()> {
        handle_user_bet(ctx, game_id, user_betting_on_id, money_staked, bet_type)
    }


        
    pub fn update_User_bet(
        ctx: Context<UpdateUserGameBet>,
        bet_type: BetType,
        game_id: [u8;16],
        user_betting_on_id: [u8;16],
        money_staked: u64,
    
    ) -> Result<()> {
        handle_update_bet(ctx, bet_type,  game_id, user_betting_on_id, money_staked)
    }

    pub fn settle_all_bets(
        ctx: Context<SettleAllBetsForGame>,
        bet_type: BetType,
        game_id: [u8;16],
        user_betting_on_id: [u8;16],
        winner_id: [u8;16],
    
    ) -> Result<()> {
        handle_settle_all_bets_for_game(ctx, bet_type,  game_id, user_betting_on_id, winner_id)
    }



}