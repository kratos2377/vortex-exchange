use std::fmt;
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{TokenAccount, TokenInterface};
use borsh::{BorshDeserialize, BorshSerialize};

use crate::errors::VortexDexResult;

use super::{dex_state::DexState, position::PositionDirection, spot_market::{SpotBalanceType, SpotMarket}, user_stats::UserStats};
use crate::utils::{constants::{SPOT_WEIGHT_PRECISION, SPOT_WEIGHT_PRECISION_I128}, margin_utils::MarginRequirementType};


#[account(zero_copy(unsafe))]
#[repr(C)]
pub struct User {
    pub authority: Pubkey,
    pub name: [u8; 32],
    pub game_stake_positions: [GameStakePosition; 4],
    pub spot_positions: [SpotPosition; 8],
    pub perp_positions: [PerpPosition; 8],
    pub orders: [Order; 32],
    pub total_deposits: u64,
    pub total_withdraws: u64,
    pub status: u8,

    pub cumulative_spot_fees: i64,
    pub cumulative_perp_funding: i64,
    pub liquidation_margin_freed: u64,
    pub last_active_slot: u64,
    pub next_order_id: u32,
    pub max_margin_ratio: u32,
    pub next_liquidation_id: u16
}

impl User {
    pub const SIZE: usize = 4376;
}

#[zero_copy(unsafe)]
#[derive(Default, Eq, PartialEq, Debug, BorshDeserialize , BorshSerialize)]
#[repr(C)]
pub struct SpotPosition {
    pub scaled_balance: u64,
    pub open_bids: i64,
    pub open_asks: i64,
    pub cumulative_deposits: i64,
    pub market_index: u16,
    pub balance_type: SpotBalanceType,
    pub open_orders: u8,
    pub padding: [u8; 4],
}

#[zero_copy(unsafe)]
#[derive(Default, Eq, PartialEq, Debug , BorshDeserialize , BorshSerialize)]
#[repr(C)]
pub struct GameStakePosition {
    // will be uuid
    pub game_id: [u8; 16],
    pub total_stake_by_user: u64,
    //will be uuid
    pub player_staked_id: [u8; 16]
}

#[zero_copy(unsafe)]
#[derive(Default, Debug, Eq, PartialEq, BorshDeserialize , BorshSerialize)]
#[repr(C)]
pub struct PerpPosition {
    pub last_cumulative_funding_rate: i64,
    pub base_asset_amount: i64,
    pub quote_asset_amount: i64,
    pub quote_break_even_amount: i64,
    pub quote_entry_amount: i64,
    pub open_bids: i64,
    pub open_asks: i64,
    pub settled_pnl: i64,
    pub lp_shares: u64,
    pub last_base_asset_amount_per_lp: i64,
    pub last_quote_asset_amount_per_lp: i64,
    pub remainder_base_asset_amount: i32,
    pub market_index: u16,
    pub open_orders: u8,
    pub per_lp_base: i8,
}

#[zero_copy(unsafe)]
#[repr(C)]
#[derive(AnchorSerialize, AnchorDeserialize, PartialEq, Debug, Eq)]
pub struct Order {
    /// The slot the order was placed
    pub slot: u64,
    /// The limit price for the order (can be 0 for market orders)
    /// For orders with an auction, this price isn't used until the auction is complete
    /// precision: PRICE_PRECISION
    pub price: u64,
    /// The size of the order
    /// precision for perps: BASE_PRECISION
    /// precision for spot: token mint precision
    pub base_asset_amount: u64,
    /// The amount of the order filled
    /// precision for perps: BASE_PRECISION
    /// precision for spot: token mint precision
    pub base_asset_amount_filled: u64,
    /// The amount of quote filled for the order
    /// precision: QUOTE_PRECISION
    pub quote_asset_amount_filled: u64,
    /// At what price the order will be triggered. Only relevant for trigger orders
    /// precision: PRICE_PRECISION
    pub trigger_price: u64,
    /// The start price for the auction. Only relevant for market/oracle orders
    /// precision: PRICE_PRECISION
    pub auction_start_price: i64,
    /// The end price for the auction. Only relevant for market/oracle orders
    /// precision: PRICE_PRECISION
    pub auction_end_price: i64,
    /// The time when the order will expire
    pub max_ts: i64,
    /// If set, the order limit price is the oracle price + this offset
    /// precision: PRICE_PRECISION
    pub oracle_price_offset: i32,
    /// The id for the order. Each users has their own order id space
    pub order_id: u32,
    /// The perp/spot market index
    pub market_index: u16,
    /// Whether the order is open or unused
    pub status: OrderStatus,
    /// The type of order
    pub order_type: OrderType,
    /// Whether market is spot or perp
    pub market_type: MarketType,
    /// User generated order id. Can make it easier to place/cancel orders
    pub user_order_id: u8,
    /// What the users position was when the order was placed
    pub existing_position_direction: PositionDirection,
    /// Whether the user is going long or short. LONG = bid, SHORT = ask
    pub direction: PositionDirection,
    /// Whether the order is allowed to only reduce position size
    pub reduce_only: bool,
    /// Whether the order must be a maker
    pub post_only: bool,
    /// Whether the order must be canceled the same slot it is placed
    pub immediate_or_cancel: bool,
    /// Whether the order is triggered above or below the trigger price. Only relevant for trigger orders
    pub trigger_condition: OrderTriggerCondition,
    /// How many slots the auction lasts
    pub auction_duration: u8,
    pub padding: [u8; 3],
}


#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Debug)]
pub enum OrderStatus {
    /// The order is not in use
    Init,
    /// Order is open
    Open,
    /// Order has been filled
    Filled,
    /// Order has been canceled
    Canceled,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq, Default)]
pub enum OrderType {
    Market,
    #[default]
    Limit,
    TriggerMarket,
    TriggerLimit,
    /// Market order where the auction prices are oracle offsets
    Oracle,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq, Default)]
pub enum OrderTriggerCondition {
    #[default]
    Above,
    Below,
    TriggeredAbove, // above condition has been triggered
    TriggeredBelow, // below condition has been triggered
}

#[derive(Default, Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum MarketType {
    #[default]
    Spot,
    Perp,
}

impl fmt::Display for MarketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarketType::Spot => write!(f, "Spot"),
            MarketType::Perp => write!(f, "Perp"),
        }
    }
}


#[derive(Clone, Copy, Default, Eq, PartialEq, Debug)]
pub struct OrderFillSimulation {
    pub token_amount: i128,
    pub orders_value: i128,
    pub token_value: i128,
    pub weighted_token_value: i128,
    pub free_collateral_contribution: i128,
}

impl OrderFillSimulation {
    pub fn riskier_side(ask: Self, bid: Self) -> Self {
        if ask.free_collateral_contribution <= bid.free_collateral_contribution {
            ask
        } else {
            bid
        }
    }

    pub fn risk_increasing(&self, after: Self) -> bool {
        after.free_collateral_contribution < self.free_collateral_contribution
    }

    pub fn apply_user_custom_margin_ratio(
        mut self,
        spot_market: &SpotMarket,
        oracle_price: i64,
        user_custom_margin_ratio: u32,
    ) -> VortexDexResult<Self> {
        if user_custom_margin_ratio == 0 {
            return Ok(self);
        }

        if self.weighted_token_value < 0 {
            let max_liability_weight = spot_market
                .get_liability_weight(
                    self.token_amount.unsigned_abs(),
                    &MarginRequirementType::Initial,
                )?
                .max(user_custom_margin_ratio.safe_add(SPOT_WEIGHT_PRECISION)?);

            self.weighted_token_value = self
                .token_value
                .safe_mul(max_liability_weight.cast()?)?
                .safe_div(SPOT_WEIGHT_PRECISION_I128)?;
        } else if self.weighted_token_value > 0 {
            let min_asset_weight = spot_market
                .get_asset_weight(
                    self.token_amount.unsigned_abs(),
                    oracle_price,
                    &MarginRequirementType::Initial,
                )?
                .min(SPOT_WEIGHT_PRECISION.saturating_sub(user_custom_margin_ratio));

            self.weighted_token_value = self
                .token_value
                .safe_mul(min_asset_weight.cast()?)?
                .safe_div(SPOT_WEIGHT_PRECISION_I128)?;
        }

        self.free_collateral_contribution =
            self.weighted_token_value.safe_add(self.orders_value)?;

        Ok(self)
    }
}