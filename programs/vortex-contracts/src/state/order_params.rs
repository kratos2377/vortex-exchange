use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};
use crate::{errors::VortexDexResult, utils::constants::PERCENTAGE_PRECISION_I64};

use super::{events::OrderActionExplanation, position::PositionDirection, user::{MarketType, OrderTriggerCondition, OrderType}};


#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default, Copy, Eq, PartialEq, Debug)]
pub struct OrderParams {
    pub order_type: OrderType,
    pub market_type: MarketType,
    pub direction: PositionDirection,
    pub user_order_id: u8,
    pub base_asset_amount: u64,
    pub price: u64,
    pub market_index: u16,
    pub reduce_only: bool,
    pub post_only: PostOnlyParam,
    pub immediate_or_cancel: bool,
    pub max_ts: Option<i64>,
    pub trigger_price: Option<u64>,
    pub trigger_condition: OrderTriggerCondition,
    pub oracle_price_offset: Option<i32>, // price offset from oracle for order (~ +/- 2147 max)
    pub auction_duration: Option<u8>,     // specified in slots
    pub auction_start_price: Option<i64>, // specified in price or oracle_price_offset
    pub auction_end_price: Option<i64>,   // specified in price or oracle_price_offset
}

impl OrderParams {

    pub fn get_auction_start_price_offset(self, oracle_price: i64) -> VortexDexResult<i64> {
        let start_offset = if self.order_type == OrderType::Oracle {
            self.auction_start_price.unwrap_or(0)
        } else if let Some(auction_start_price) = self.auction_start_price {
            auction_start_price.safe_sub(oracle_price)?
        } else {
            return Ok(0);
        };

        Ok(start_offset)
    }

    pub fn get_auction_end_price_offset(self, oracle_price: i64) -> VortexDexResult<i64> {
        let end_offset = if self.order_type == OrderType::Oracle {
            self.auction_end_price.unwrap_or(0)
        } else if let Some(auction_end_price) = self.auction_end_price {
            auction_end_price.safe_sub(oracle_price)?
        } else {
            return Ok(0);
        };

        Ok(end_offset)
    }

  

}


#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct ModifyOrderParams {
    pub direction: Option<PositionDirection>,
    pub base_asset_amount: Option<u64>,
    pub price: Option<u64>,
    pub reduce_only: Option<bool>,
    pub post_only: Option<PostOnlyParam>,
    pub immediate_or_cancel: Option<bool>,
    pub max_ts: Option<i64>,
    pub trigger_price: Option<u64>,
    pub trigger_condition: Option<OrderTriggerCondition>,
    pub oracle_price_offset: Option<i32>,
    pub auction_duration: Option<u8>,
    pub auction_start_price: Option<i64>,
    pub auction_end_price: Option<i64>,
    pub policy: Option<ModifyOrderPolicy>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Eq, PartialEq)]
pub enum ModifyOrderPolicy {
    TryModify,
    MustModify,
}

impl Default for ModifyOrderPolicy {
    fn default() -> Self {
        Self::TryModify
    }
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq, Default)]
pub enum PostOnlyParam {
    #[default]
    None,
    MustPostOnly, // Tx fails if order can't be post only
    TryPostOnly,  // Tx succeeds and order not placed if can't be post only
    Slide,        // Modify price to be post only if can't be post only
}

pub struct PlaceOrderOptions {
    pub try_expire_orders: bool,
    pub enforce_margin_check: bool,
    pub risk_increasing: bool,
    pub explanation: OrderActionExplanation,
}

impl Default for PlaceOrderOptions {
    fn default() -> Self {
        Self {
            try_expire_orders: true,
            enforce_margin_check: true,
            risk_increasing: false,
            explanation: OrderActionExplanation::None,
        }
    }
}

impl PlaceOrderOptions {
    pub fn update_risk_increasing(&mut self, risk_increasing: bool) {
        self.risk_increasing = self.risk_increasing || risk_increasing;
    }

    pub fn explanation(mut self, explanation: OrderActionExplanation) -> Self {
        self.explanation = explanation;
        self
    }

    pub fn is_liquidation(&self) -> bool {
        self.explanation == OrderActionExplanation::Liquidation
    }
}
