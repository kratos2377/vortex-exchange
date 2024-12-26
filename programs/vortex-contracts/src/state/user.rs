use std::{fmt, ops::Neg, panic::Location};
use anchor_lang::prelude::*;
use crate::math_error;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::{casting::Cast, errors::{DexError, VortexDexResult}, get_then_update_id, instructions::constraints::{can_sign_for_user, is_stats_for_user}, safe_increment, safe_methods::SafeMath, utils::{auction_utils::{calculate_auction_price, is_auction_complete}, constants::{FUEL_START_TS, OPEN_ORDER_MARGIN_REQUIREMENT, QUOTE_PRECISION_U64, QUOTE_SPOT_MARKET_INDEX}, margin_utils::{calculate_margin_requirement_and_total_collateral_and_liability_info, validate_any_isolated_tier_requirements}, order_utils::{standardize_base_asset_amount, standardize_price}, spot_market_utils::{get_signed_token_amount, get_strict_token_value, get_token_amount, get_token_value}}, validate};

use super::{margin_calculation::{MarginCalculation, MarginContext}, oracle::StrictOraclePrice, oracle_map::OracleMap, position::PositionDirection, spot_market::{SpotBalance, SpotBalanceType, SpotMarket}, spot_market_map::SpotMarketMap, user_stats::UserStats};
use crate::utils::{constants::{SPOT_WEIGHT_PRECISION, SPOT_WEIGHT_PRECISION_I128}, margin_utils::MarginRequirementType};


#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum UserStatus {
    // Active = 0
    BeingLiquidated = 0b00000001,
    Bankrupt = 0b00000010,
    ReduceOnly = 0b00000100,
    AdvancedLp = 0b00001000,
}


#[account(zero_copy(unsafe))]
#[repr(C)]
pub struct User {
    pub authority: Pubkey,
    pub name: [u8; 32],
    pub game_stake_positions: [GameStakePosition; 4],
    pub spot_positions: [SpotPosition; 8],
    pub orders: [Order; 32],
    pub total_deposits: u64,
    pub total_withdraws: u64,
    pub status: u8,
    pub total_social_loss: u64,
    pub cumulative_spot_fees: i64,
    pub liquidation_margin_freed: u64,
    pub last_active_slot: u64,
    pub next_order_id: u32,
    pub max_margin_ratio: u32,
    pub next_liquidation_id: u16,
    pub idle: bool,
    pub open_orders: u8,
    pub has_open_order: bool,
    pub open_auctions: u8,
    pub has_open_auction: bool,
    pub last_fuel_bonus_update_ts: u32,
    pub is_margin_trading_enabled: bool,
}

impl User {
    pub const SIZE: usize = 4376;


    pub fn is_being_liquidated(&self) -> bool {
        self.status & (UserStatus::BeingLiquidated as u8 | UserStatus::Bankrupt as u8) > 0
    }

    pub fn is_bankrupt(&self) -> bool {
        self.status & (UserStatus::Bankrupt as u8) > 0
    }

    pub fn is_reduce_only(&self) -> bool {
        self.status & (UserStatus::ReduceOnly as u8) > 0
    }

    pub fn is_advanced_lp(&self) -> bool {
        self.status & (UserStatus::AdvancedLp as u8) > 0
    }

    pub fn add_user_status(&mut self, status: UserStatus) {
        self.status |= status as u8;
    }

    pub fn remove_user_status(&mut self, status: UserStatus) {
        self.status &= !(status as u8);
    }

    pub fn get_spot_position_index(&self, market_index: u16) -> VortexDexResult<usize> {
        // first spot position is always quote asset
        if market_index == 0 {
            validate!(
                self.spot_positions[0].market_index == 0,
                DexError::DefaultError,
                "User position 0 not market_index=0"
            )?;
            return Ok(0);
        }

        self.spot_positions
            .iter()
            .position(|spot_position| spot_position.market_index == market_index)
            .ok_or(DexError::CouldNotFindSpotPosition)
    }

    pub fn get_spot_position(&self, market_index: u16) -> VortexDexResult<&SpotPosition> {
        self.get_spot_position_index(market_index)
            .map(|market_index| &self.spot_positions[market_index])
    }

    pub fn get_spot_position_mut(&mut self, market_index: u16) -> VortexDexResult<&mut SpotPosition> {
        self.get_spot_position_index(market_index)
            .map(move |market_index| &mut self.spot_positions[market_index])
    }

    pub fn get_quote_spot_position(&self) -> &SpotPosition {
        match self.get_spot_position(QUOTE_SPOT_MARKET_INDEX) {
            Ok(position) => position,
            Err(_) => unreachable!(),
        }
    }

    pub fn get_quote_spot_position_mut(&mut self) -> &mut SpotPosition {
        match self.get_spot_position_mut(QUOTE_SPOT_MARKET_INDEX) {
            Ok(position) => position,
            Err(_) => unreachable!(),
        }
    }

    pub fn add_spot_position(
        &mut self,
        market_index: u16,
        balance_type: SpotBalanceType,
    ) -> VortexDexResult<usize> {
        let new_spot_position_index = self
            .spot_positions
            .iter()
            .enumerate()
            .position(|(index, spot_position)| index != 0 && spot_position.is_available())
            .ok_or(DexError::NoSpotPositionAvailable)?;

        let new_spot_position = SpotPosition {
            market_index,
            balance_type,
            ..SpotPosition::default()
        };

        self.spot_positions[new_spot_position_index] = new_spot_position;

        Ok(new_spot_position_index)
    }

    pub fn force_get_spot_position_mut(
        &mut self,
        market_index: u16,
    ) -> VortexDexResult<&mut SpotPosition> {
        self.get_spot_position_index(market_index)
            .or_else(|_| self.add_spot_position(market_index, SpotBalanceType::Deposit))
            .map(move |market_index| &mut self.spot_positions[market_index])
    }

    pub fn force_get_spot_position_index(&mut self, market_index: u16) -> VortexDexResult<usize> {
        self.get_spot_position_index(market_index)
            .or_else(|_| self.add_spot_position(market_index, SpotBalanceType::Deposit))
    }

    pub fn get_order_index(&self, order_id: u32) -> VortexDexResult<usize> {
        self.orders
            .iter()
            .position(|order| order.order_id == order_id && order.status == OrderStatus::Open)
            .ok_or(DexError::OrderDoesNotExist)
    }

    pub fn get_order_index_by_user_order_id(&self, user_order_id: u8) -> VortexDexResult<usize> {
        self.orders
            .iter()
            .position(|order| {
                order.user_order_id == user_order_id && order.status == OrderStatus::Open
            })
            .ok_or(DexError::OrderDoesNotExist)
    }

    pub fn get_order(&self, order_id: u32) -> Option<&Order> {
        self.orders.iter().find(|order| order.order_id == order_id)
    }

    pub fn get_last_order_id(&self) -> u32 {
        if self.next_order_id == 1 {
            u32::MAX
        } else {
            self.next_order_id - 1
        }
    }

    pub fn increment_total_deposits(
        &mut self,
        amount: u64,
        price: i64,
        precision: u128,
    ) -> VortexDexResult {
        let value = amount
            .cast::<u128>()?
            .safe_mul(price.cast::<u128>()?)?
            .safe_div(precision)?
            .cast::<u64>()?;
        self.total_deposits = self.total_deposits.saturating_add(value);

        Ok(())
    }

    pub fn increment_total_withdraws(
        &mut self,
        amount: u64,
        price: i64,
        precision: u128,
    ) -> VortexDexResult {
        let value = amount
            .cast::<u128>()?
            .safe_mul(price.cast()?)?
            .safe_div(precision)?
            .cast::<u64>()?;
        self.total_withdraws = self.total_withdraws.saturating_add(value);

        Ok(())
    }

    pub fn increment_total_socialized_loss(&mut self, value: u64) -> VortexDexResult {
        self.total_social_loss = self.total_social_loss.saturating_add(value);

        Ok(())
    }

    pub fn update_cumulative_spot_fees(&mut self, amount: i64) -> VortexDexResult {
        safe_increment!(self.cumulative_spot_fees, amount);
        Ok(())
    }



    pub fn enter_liquidation(&mut self, slot: u64) -> VortexDexResult<u16> {
        if self.is_being_liquidated() {
            return self.next_liquidation_id.safe_sub(1);
        }

        self.add_user_status(UserStatus::BeingLiquidated);
        self.liquidation_margin_freed = 0;
        self.last_active_slot = slot;
        Ok(get_then_update_id!(self, next_liquidation_id))
    }

    pub fn exit_liquidation(&mut self) {
        self.remove_user_status(UserStatus::BeingLiquidated);
        self.remove_user_status(UserStatus::Bankrupt);
        self.liquidation_margin_freed = 0;
    }

    pub fn enter_bankruptcy(&mut self) {
        self.remove_user_status(UserStatus::BeingLiquidated);
        self.add_user_status(UserStatus::Bankrupt);
    }

    pub fn exit_bankruptcy(&mut self) {
        self.remove_user_status(UserStatus::BeingLiquidated);
        self.remove_user_status(UserStatus::Bankrupt);
        self.liquidation_margin_freed = 0;
    }

    pub fn increment_margin_freed(&mut self, margin_free: u64) -> VortexDexResult {
        self.liquidation_margin_freed = self.liquidation_margin_freed.safe_add(margin_free)?;
        Ok(())
    }

    pub fn update_last_active_slot(&mut self, slot: u64) {
        if !self.is_being_liquidated() {
            self.last_active_slot = slot;
        }
        self.idle = false;
    }

    pub fn increment_open_orders(&mut self, is_auction: bool) {
        self.open_orders = self.open_orders.saturating_add(1);
        self.has_open_order = self.open_orders > 0;
        if is_auction {
            self.increment_open_auctions();
        }
    }

    pub fn increment_open_auctions(&mut self) {
        self.open_auctions = self.open_auctions.saturating_add(1);
        self.has_open_auction = self.open_auctions > 0;
    }

    pub fn decrement_open_orders(&mut self, is_auction: bool) {
        self.open_orders = self.open_orders.saturating_sub(1);
        self.has_open_order = self.open_orders > 0;
        if is_auction {
            self.open_auctions = self.open_auctions.saturating_sub(1);
            self.has_open_auction = self.open_auctions > 0;
        }
    }

    pub fn qualifies_for_withdraw_fee(&self, user_stats: &UserStats, slot: u64) -> bool {
        // only qualifies for user with recent last_active_slot (~25 seconds)
        if slot.saturating_sub(self.last_active_slot) >= 50 {
            return false;
        }

        let min_total_withdraws = 10_000_000 * QUOTE_PRECISION_U64; // $10M

        // if total withdraws are greater than $10M and user has paid more than %.01 of it in fees
        self.total_withdraws >= min_total_withdraws
            && self.total_withdraws / user_stats.fees.total_fee_paid.max(1) > 10_000
    }

    pub fn update_reduce_only_status(&mut self, reduce_only: bool) -> VortexDexResult {
        if reduce_only {
            self.add_user_status(UserStatus::ReduceOnly);
        } else {
            self.remove_user_status(UserStatus::ReduceOnly);
        }

        Ok(())
    }

    pub fn update_advanced_lp_status(&mut self, advanced_lp: bool) -> VortexDexResult {
        if advanced_lp {
            self.add_user_status(UserStatus::AdvancedLp);
        } else {
            self.remove_user_status(UserStatus::AdvancedLp);
        }

        Ok(())
    }

    pub fn has_room_for_new_order(&self) -> bool {
        for order in self.orders.iter() {
            if order.status == OrderStatus::Init {
                return true;
            }
        }

        false
    }

    pub fn get_fuel_bonus_numerator(&self, now: i64) -> VortexDexResult<i64> {
        if self.last_fuel_bonus_update_ts > 0 {
            now.safe_sub(self.last_fuel_bonus_update_ts.cast()?)
        } else {
            // start ts for existing accounts pre fuel
            if now > FUEL_START_TS {
                return Ok(now.safe_sub(FUEL_START_TS)?);
            } else {
                return Ok(0);
            }
        }
    }

    pub fn calculate_margin_and_increment_fuel_bonus(
        &mut self,
        spot_market_map: &SpotMarketMap,
        oracle_map: &mut OracleMap,
        context: MarginContext,
        user_stats: &mut UserStats,
        now: i64,
    ) -> VortexDexResult<MarginCalculation> {
        let fuel_bonus_numerator = self.get_fuel_bonus_numerator(now)?;

        validate!(
            context.fuel_bonus_numerator == fuel_bonus_numerator,
            DexError::DefaultError,
            "Bad fuel bonus update attempt {} != {} (now = {})",
            context.fuel_bonus_numerator,
            fuel_bonus_numerator,
            now
        )?;

        let margin_calculation =
            calculate_margin_requirement_and_total_collateral_and_liability_info(
                self,
                spot_market_map,
                oracle_map,
                context,
            )?;

        user_stats.update_fuel_bonus(
            self,
            margin_calculation.fuel_deposits,
            margin_calculation.fuel_borrows,
            margin_calculation.fuel_positions,
            now,
        )?;

        Ok(margin_calculation)
    }

    pub fn meets_withdraw_margin_requirement_and_increment_fuel_bonus(
        &mut self,
        spot_market_map: &SpotMarketMap,
        oracle_map: &mut OracleMap,
        margin_requirement_type: MarginRequirementType,
        withdraw_market_index: u16,
        withdraw_amount: u128,
        user_stats: &mut UserStats,
        now: i64,
    ) -> VortexDexResult<bool> {
        let strict = margin_requirement_type == MarginRequirementType::Initial;
        let context = MarginContext::standard(margin_requirement_type)
            .strict(strict)
            .fuel_spot_delta(withdraw_market_index, withdraw_amount.cast::<i128>()?)
            .fuel_numerator(self, now);

        let calculation = calculate_margin_requirement_and_total_collateral_and_liability_info(
            self,
            spot_market_map,
            oracle_map,
            context,
        )?;

        if calculation.margin_requirement > 0 || calculation.get_num_of_liabilities()? > 0 {
            validate!(
                calculation.all_oracles_valid,
                DexError::InvalidOracle,
                "User attempting to withdraw with outstanding liabilities when an oracle is invalid"
            )?;
        }

        validate_any_isolated_tier_requirements(self, calculation)?;

        validate!(
            calculation.meets_margin_requirement(),
            DexError::InsufficientCollateral,
            "User attempting to withdraw where total_collateral {} is below initial_margin_requirement {}",
            calculation.total_collateral,
            calculation.margin_requirement
        )?;

        user_stats.update_fuel_bonus(
            self,
            calculation.fuel_deposits,
            calculation.fuel_borrows,
            calculation.fuel_positions,
            now,
        )?;

        Ok(true)
    }
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

impl SpotPosition {
    pub fn is_available(&self) -> bool {
        self.scaled_balance == 0 && self.open_orders == 0
    }

    pub fn has_open_order(&self) -> bool {
        self.open_orders != 0 || self.open_bids != 0 || self.open_asks != 0
    }

    pub fn margin_requirement_for_open_orders(&self) -> VortexDexResult<u128> {
        self.open_orders
            .cast::<u128>()?
            .safe_mul(OPEN_ORDER_MARGIN_REQUIREMENT)
    }

    pub fn get_token_amount(&self, spot_market: &SpotMarket) -> VortexDexResult<u128> {
        get_token_amount(self.scaled_balance.cast()?, spot_market, &self.balance_type)
    }

    pub fn get_signed_token_amount(&self, spot_market: &SpotMarket) -> VortexDexResult<i128> {
        get_signed_token_amount(
            get_token_amount(self.scaled_balance.cast()?, spot_market, &self.balance_type)?,
            &self.balance_type,
        )
    }

    pub fn get_worst_case_fill_simulation(
        &self,
        spot_market: &SpotMarket,
        strict_oracle_price: &StrictOraclePrice,
        token_amount: Option<i128>,
        margin_type: MarginRequirementType,
    ) -> VortexDexResult<OrderFillSimulation> {
        let [bid_simulation, ask_simulation] = self.simulate_fills_both_sides(
            spot_market,
            strict_oracle_price,
            token_amount,
            margin_type,
        )?;

        Ok(OrderFillSimulation::riskier_side(
            ask_simulation,
            bid_simulation,
        ))
    }

    pub fn simulate_fills_both_sides(
        &self,
        spot_market: &SpotMarket,
        strict_oracle_price: &StrictOraclePrice,
        token_amount: Option<i128>,
        margin_type: MarginRequirementType,
    ) -> VortexDexResult<[OrderFillSimulation; 2]> {
        let token_amount = match token_amount {
            Some(token_amount) => token_amount,
            None => self.get_signed_token_amount(spot_market)?,
        };

        let token_value =
            get_strict_token_value(token_amount, spot_market.decimals, strict_oracle_price)?;

        let calculate_weighted_token_value = |token_amount: i128, token_value: i128| {
            if token_value > 0 {
                let asset_weight = spot_market.get_asset_weight(
                    token_amount.unsigned_abs(),
                    strict_oracle_price.current,
                    &margin_type,
                )?;

                token_value
                    .safe_mul(asset_weight.cast()?)?
                    .safe_div(SPOT_WEIGHT_PRECISION_I128)
            } else if token_value < 0 {
                let liability_weight =
                    spot_market.get_liability_weight(token_amount.unsigned_abs(), &margin_type)?;

                token_value
                    .safe_mul(liability_weight.cast()?)?
                    .safe_div(SPOT_WEIGHT_PRECISION_I128)
            } else {
                Ok(0)
            }
        };

        if self.open_bids == 0 && self.open_asks == 0 {
            let weighted_token_value = calculate_weighted_token_value(token_amount, token_value)?;

            let calculation = OrderFillSimulation {
                token_amount,
                orders_value: 0,
                token_value,
                weighted_token_value,
                free_collateral_contribution: weighted_token_value,
            };

            return Ok([calculation, calculation]);
        }

        let simulate_side = |strict_oracle_price: &StrictOraclePrice,
                             token_amount: i128,
                             open_orders: i128| {
            let order_value = get_token_value(
                -open_orders,
                spot_market.decimals,
                strict_oracle_price.max(),
            )?;
            let token_amount_after_fill = token_amount.safe_add(open_orders)?;
            let token_value_after_fill = token_value.safe_add(order_value.neg())?;

            let weighted_token_value_after_fill =
                calculate_weighted_token_value(token_amount_after_fill, token_value_after_fill)?;

            let free_collateral_contribution =
                weighted_token_value_after_fill.safe_add(order_value)?;

            Ok(OrderFillSimulation {
                token_amount: token_amount_after_fill,
                orders_value: order_value,
                token_value: token_value_after_fill,
                weighted_token_value: weighted_token_value_after_fill,
                free_collateral_contribution,
            })
        };

        let bid_simulation =
            simulate_side(strict_oracle_price, token_amount, self.open_bids.cast()?)?;

        let ask_simulation =
            simulate_side(strict_oracle_price, token_amount, self.open_asks.cast()?)?;

        Ok([bid_simulation, ask_simulation])
    }

    pub fn is_borrow(&self) -> bool {
        self.scaled_balance > 0 && self.balance_type == SpotBalanceType::Borrow
    }
}


impl SpotBalance for SpotPosition {
    fn market_index(&self) -> u16 {
        self.market_index
    }

    fn balance_type(&self) -> &SpotBalanceType {
        &self.balance_type
    }

    fn balance(&self) -> u128 {
        self.scaled_balance as u128
    }

    fn increase_balance(&mut self, delta: u128) -> VortexDexResult {
        self.scaled_balance = self.scaled_balance.safe_add(delta.cast()?)?;
        Ok(())
    }

    fn decrease_balance(&mut self, delta: u128) -> VortexDexResult {
        self.scaled_balance = self.scaled_balance.safe_sub(delta.cast()?)?;
        Ok(())
    }

    fn update_balance_type(&mut self, balance_type: SpotBalanceType) -> VortexDexResult {
        self.balance_type = balance_type;
        Ok(())
    }
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
    /// precision for spot: token mint precision
    pub base_asset_amount: u64,
    /// The amount of the order filled
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
    /// The spot market index
    pub market_index: u16,
    /// Whether the order is open or unused
    pub status: OrderStatus,
    /// The type of order
    pub order_type: OrderType,
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

impl Default for Order {
    fn default() -> Self {
        Self {
            status: OrderStatus::Init,
            order_type: OrderType::Limit,
            market_type: MarketType::Spot,
            slot: 0,
            order_id: 0,
            user_order_id: 0,
            market_index: 0,
            price: 0,
            existing_position_direction: PositionDirection::Long,
            base_asset_amount: 0,
            base_asset_amount_filled: 0,
            quote_asset_amount_filled: 0,
            direction: PositionDirection::Long,
            reduce_only: false,
            post_only: false,
            immediate_or_cancel: false,
            trigger_price: 0,
            trigger_condition: OrderTriggerCondition::Above,
            oracle_price_offset: 0,
            auction_start_price: 0,
            auction_end_price: 0,
            auction_duration: 0,
            max_ts: 0,
            padding: [0; 3],
        }
    }
}


impl Order {
    pub fn seconds_til_expiry(self, now: i64) -> i64 {
        (self.max_ts - now).max(0)
    }

    pub fn has_oracle_price_offset(self) -> bool {
        self.oracle_price_offset != 0
    }

    pub fn get_limit_price(
        &self,
        valid_oracle_price: Option<i64>,
        fallback_price: Option<u64>,
        slot: u64,
        tick_size: u64,
    ) -> VortexDexResult<Option<u64>> {
        let price = if self.has_auction_price(self.slot, self.auction_duration, slot)? {
            Some(calculate_auction_price(
                self,
                slot,
                tick_size,
                valid_oracle_price,
            )?)
        } else if self.has_oracle_price_offset() {
            let oracle_price = valid_oracle_price.ok_or_else(|| {
                msg!("Could not find oracle too calculate oracle offset limit price");
                DexError::OracleNotFound
            })?;

            let limit_price = oracle_price
                .safe_add(self.oracle_price_offset.cast()?)?
                .max(tick_size.cast()?);

            Some(standardize_price(
                limit_price.cast::<u64>()?,
                tick_size,
                self.direction,
            )?)
        } else if self.price == 0 {
            match fallback_price {
                Some(price) => Some(standardize_price(price, tick_size, self.direction)?),
                None => None,
            }
        } else {
            Some(self.price)
        };

        Ok(price)
    }

    #[track_caller]
    #[inline(always)]
    pub fn force_get_limit_price(
        &self,
        valid_oracle_price: Option<i64>,
        fallback_price: Option<u64>,
        slot: u64,
        tick_size: u64,
    ) -> VortexDexResult<u64> {
        match self.get_limit_price(valid_oracle_price, fallback_price, slot, tick_size)? {
            Some(price) => Ok(price),
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not get limit price at {}:{}",
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToGetLimitPrice)
            }
        }
    }

    pub fn has_limit_price(self, slot: u64) -> VortexDexResult<bool> {
        Ok(self.price > 0
            || self.has_oracle_price_offset()
            || !is_auction_complete(self.slot, self.auction_duration, slot)?)
    }

    pub fn is_auction_complete(self, slot: u64) -> VortexDexResult<bool> {
        is_auction_complete(self.slot, self.auction_duration, slot)
    }

    pub fn has_auction(&self) -> bool {
        self.auction_duration != 0
    }

    pub fn has_auction_price(
        &self,
        order_slot: u64,
        auction_duration: u8,
        slot: u64,
    ) -> VortexDexResult<bool> {
        let auction_complete = is_auction_complete(order_slot, auction_duration, slot)?;
        let has_auction_prices = self.auction_start_price != 0 || self.auction_end_price != 0;
        Ok(!auction_complete && has_auction_prices)
    }

    /// Passing in an existing_position forces the function to consider the order's reduce only status
    pub fn get_base_asset_amount_unfilled(
        &self,
        existing_position: Option<i64>,
    ) -> VortexDexResult<u64> {
        let base_asset_amount_unfilled = self
            .base_asset_amount
            .safe_sub(self.base_asset_amount_filled)?;

        let existing_position = match existing_position {
            Some(existing_position) => existing_position,
            None => {
                return Ok(base_asset_amount_unfilled);
            }
        };

        // if order is post only, can disregard reduce only
        if !self.reduce_only || self.post_only {
            return Ok(base_asset_amount_unfilled);
        }

        if existing_position == 0 {
            return Ok(0);
        }

        match self.direction {
            PositionDirection::Long => {
                if existing_position > 0 {
                    Ok(0)
                } else {
                    Ok(base_asset_amount_unfilled.min(existing_position.unsigned_abs()))
                }
            }
            PositionDirection::Short => {
                if existing_position < 0 {
                    Ok(0)
                } else {
                    Ok(base_asset_amount_unfilled.min(existing_position.unsigned_abs()))
                }
            }
        }
    }

    /// Stardardizes the base asset amount unfilled to the nearest step size
    /// Particularly important for spot positions where existing position can be dust
    pub fn get_standardized_base_asset_amount_unfilled(
        &self,
        existing_position: Option<i64>,
        step_size: u64,
    ) -> VortexDexResult<u64> {
        standardize_base_asset_amount(
            self.get_base_asset_amount_unfilled(existing_position)?,
            step_size,
        )
    }

    pub fn must_be_triggered(&self) -> bool {
        matches!(
            self.order_type,
            OrderType::TriggerMarket | OrderType::TriggerLimit
        )
    }

    pub fn triggered(&self) -> bool {
        matches!(
            self.trigger_condition,
            OrderTriggerCondition::TriggeredAbove | OrderTriggerCondition::TriggeredBelow
        )
    }

    pub fn is_jit_maker(&self) -> bool {
        self.post_only && self.immediate_or_cancel
    }

    pub fn is_open_order_for_market(&self, market_index: u16, market_type: &MarketType) -> bool {
        self.market_index == market_index
            && self.status == OrderStatus::Open
            && &self.market_type == market_type
    }

    pub fn get_spot_position_update_direction(&self, asset_type: AssetType) -> SpotBalanceType {
        match (self.direction, asset_type) {
            (PositionDirection::Long, AssetType::Base) => SpotBalanceType::Deposit,
            (PositionDirection::Long, AssetType::Quote) => SpotBalanceType::Borrow,
            (PositionDirection::Short, AssetType::Base) => SpotBalanceType::Borrow,
            (PositionDirection::Short, AssetType::Quote) => SpotBalanceType::Deposit,
        }
    }

    pub fn is_market_order(&self) -> bool {
        matches!(
            self.order_type,
            OrderType::Market | OrderType::TriggerMarket | OrderType::Oracle
        )
    }

    pub fn is_limit_order(&self) -> bool {
        matches!(self.order_type, OrderType::Limit | OrderType::TriggerLimit)
    }

    pub fn is_resting_limit_order(&self, slot: u64) -> VortexDexResult<bool> {
        if !self.is_limit_order() {
            return Ok(false);
        }

        if self.order_type == OrderType::TriggerLimit {
            return match self.direction {
                PositionDirection::Long if self.trigger_price < self.price => {
                    return Ok(false);
                }
                PositionDirection::Short if self.trigger_price > self.price => {
                    return Ok(false);
                }
                _ => self.is_auction_complete(slot),
            };
        }

        Ok(self.post_only || self.is_auction_complete(slot)?)
    }
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
}

impl fmt::Display for MarketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarketType::Spot => write!(f, "Spot"),
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


#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Debug)]
pub enum AssetType {
    Base,
    Quote,
}

