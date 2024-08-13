#![allow(clippy::too_many_arguments)]
#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::comparison_chain)]
use anchor_lang::prelude::*;

use instructions::{user::*};
pub mod errors;
pub mod state;
pub mod utils;
pub mod instructions;
pub mod macros;
pub mod ids;
pub mod safe_methods;
pub mod casting;
pub mod controllers;


#[cfg(feature = "devnet")]
declare_id!("HkApQpEsdzdfHsedkuZvNEbmcQXfabobbb9Yf8wdz7AZ");

#[program]
pub mod vortex_contracts {
  

    use state::{order_params::{ModifyOrderParams, OrderParams}, position::PositionDirection, user::MarketType};

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


    // trader bots
    // bots will be responsible for taking orders and completing them
    // will also do settlement like P&L settlement
}