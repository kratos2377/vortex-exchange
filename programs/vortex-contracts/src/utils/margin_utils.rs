use std::cmp::{max, min, Ordering};

use num_integer::Roots;
use solana_program::msg;

use crate::{casting::Cast, errors::{DexError, VortexDexResult}, safe_methods::SafeMath, state::{margin_calculation::{MarginCalculation, MarginContext, MarketIdentifier}, oracle::StrictOraclePrice, oracle_map::OracleMap, spot_market::{AssetTier, ContractTier, SpotBalanceType}, spot_market_map::SpotMarketMap, user::{MarketType, OrderFillSimulation, User}}, utils::{oracle_utils::{is_oracle_valid_for_action, VortexDexAction}, spot_market_utils::{get_strict_token_value, get_token_value}, validation_utils}, validate};

use super::constants::{MARGIN_PRECISION_U128, PRICE_PRECISION, SPOT_IMF_PRECISION_U128, SPOT_WEIGHT_PRECISION_U128};




#[derive(Clone, Copy, PartialEq, Debug, Eq)]
pub enum MarginRequirementType {
    Initial,
    Fill,
    Maintenance,
}

pub fn calculate_size_premium_liability_weight(
    size: u128, // AMM_RESERVE_PRECISION
    imf_factor: u32,
    liability_weight: u32,
    precision: u128,
) -> VortexDexResult<u32> {
    if imf_factor == 0 {
        return Ok(liability_weight);
    }

    let size_sqrt = ((size * 10) + 1).nth_root(2); //1e9 -> 1e10 -> 1e5

    let imf_factor_u128 = imf_factor.cast::<u128>()?;
    let liability_weight_u128 = liability_weight.cast::<u128>()?;
    let liability_weight_numerator =
        liability_weight_u128.safe_sub(liability_weight_u128.safe_div(5)?)?;

    // increases
    let size_premium_liability_weight = liability_weight_numerator
        .safe_add(
            size_sqrt // 1e5
                .safe_mul(imf_factor_u128)?
                .safe_div(100_000 * SPOT_IMF_PRECISION_U128 / precision)?, // 1e5 * 1e2
        )?
        .cast::<u32>()?;

    let max_liability_weight = max(liability_weight, size_premium_liability_weight);
    Ok(max_liability_weight)
}

pub fn calculate_size_discount_asset_weight(
    size: u128, // AMM_RESERVE_PRECISION
    imf_factor: u32,
    asset_weight: u32,
) -> VortexDexResult<u32> {
    if imf_factor == 0 {
        return Ok(asset_weight);
    }

    let size_sqrt = ((size * 10) + 1).nth_root(2); //1e9 -> 1e10 -> 1e5
    let imf_numerator = SPOT_IMF_PRECISION_U128 + SPOT_IMF_PRECISION_U128 / 10;

    let size_discount_asset_weight = imf_numerator
        .safe_mul(SPOT_WEIGHT_PRECISION_U128)?
        .safe_div(
            SPOT_IMF_PRECISION_U128
                .safe_add(size_sqrt.safe_mul(imf_factor.cast()?)?.safe_div(100_000)?)?,
        )?
        .cast::<u32>()?;

    let min_asset_weight = min(asset_weight, size_discount_asset_weight);

    Ok(min_asset_weight)
}



pub fn calculate_user_safest_position_tiers(
    user: &User,
    spot_market_map: &SpotMarketMap,
) -> VortexDexResult<(AssetTier)> {
    let mut safest_tier_spot_liablity: AssetTier = AssetTier::default();

    for spot_position in user.spot_positions.iter() {
        if spot_position.is_available() || spot_position.balance_type == SpotBalanceType::Deposit {
            continue;
        }
        let spot_market = spot_market_map.get_ref(&spot_position.market_index)?;
        safest_tier_spot_liablity = min(safest_tier_spot_liablity, spot_market.asset_tier);
    }


    Ok((safest_tier_spot_liablity))
}

pub fn calculate_margin_requirement_and_total_collateral_and_liability_info(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    context: MarginContext,
) -> VortexDexResult<MarginCalculation> {
    let mut calculation = MarginCalculation::new(context);

    let user_custom_margin_ratio = if context.margin_type == MarginRequirementType::Initial {
        user.max_margin_ratio
    } else {
        0_u32
    };

    for spot_position in user.spot_positions.iter() {
        validation_utils::validate_spot_position(spot_position)?;

        if spot_position.is_available() {
            continue;
        }

        let spot_market = spot_market_map.get_ref(&spot_position.market_index)?;
        let (oracle_price_data, oracle_validity) = oracle_map.get_price_data_and_validity(
            MarketType::Spot,
            spot_market.market_index,
            &spot_market.oracle,
            spot_market.historical_oracle_data.last_oracle_price_twap,
            spot_market.get_max_confidence_interval_multiplier()?,
        )?;

        calculation.update_all_oracles_valid(is_oracle_valid_for_action(
            oracle_validity,
            Some(VortexDexAction::MarginCalc),
        )?);

        let strict_oracle_price = StrictOraclePrice::new(
            oracle_price_data.price,
            spot_market
                .historical_oracle_data
                .last_oracle_price_twap_5min,
            calculation.context.strict,
        );
        strict_oracle_price.validate()?;

        if spot_market.market_index == 0 {
            let token_amount = spot_position.get_signed_token_amount(&spot_market)?;
            if token_amount == 0 {
                validate!(
                    spot_position.scaled_balance == 0,
                    DexError::InvalidMarginRatio,
                    "spot_position.scaled_balance={} when token_amount={}",
                    spot_position.scaled_balance,
                    token_amount,
                )?;
            }

            calculation.update_fuel_spot_bonus(&spot_market, token_amount, &strict_oracle_price)?;

            let token_value =
                get_strict_token_value(token_amount, spot_market.decimals, &strict_oracle_price)?;

            match spot_position.balance_type {
                SpotBalanceType::Deposit => {
                    calculation.add_total_collateral(token_value)?;

                    #[cfg(feature = "vortex-rs")]
                    calculation.add_spot_asset_value(token_value)?;
                }
                SpotBalanceType::Borrow => {
                    let token_value = token_value.unsigned_abs();

                    validate!(
                        token_value != 0,
                        DexError::InvalidMarginRatio,
                        "token_value=0 for token_amount={} in spot market_index={}",
                        token_amount,
                        spot_market.market_index,
                    )?;

                    calculation.add_margin_requirement(
                        token_value,
                        token_value,
                        MarketIdentifier::spot(0),
                    )?;

                    calculation.add_spot_liability()?;

                    #[cfg(feature = "vortex-rs")]
                    calculation.add_spot_liability_value(token_value)?;
                }
            }
        } else {
            let signed_token_amount = spot_position.get_signed_token_amount(&spot_market)?;

            calculation.update_fuel_spot_bonus(
                &spot_market,
                signed_token_amount,
                &strict_oracle_price,
            )?;

            let OrderFillSimulation {
                token_amount: worst_case_token_amount,
                orders_value: worst_case_orders_value,
                token_value: worst_case_token_value,
                weighted_token_value: worst_case_weighted_token_value,
                ..
            } = spot_position
                .get_worst_case_fill_simulation(
                    &spot_market,
                    &strict_oracle_price,
                    Some(signed_token_amount),
                    context.margin_type,
                )?
                .apply_user_custom_margin_ratio(
                    &spot_market,
                    strict_oracle_price.current,
                    user_custom_margin_ratio,
                )?;

            if worst_case_token_amount == 0 {
                validate!(
                    spot_position.scaled_balance == 0,
                    DexError::InvalidMarginRatio,
                    "spot_position.scaled_balance={} when worst_case_token_amount={}",
                    spot_position.scaled_balance,
                    worst_case_token_amount,
                )?;
            }

            calculation.add_margin_requirement(
                spot_position.margin_requirement_for_open_orders()?,
                0,
                MarketIdentifier::spot(spot_market.market_index),
            )?;

            match worst_case_token_value.cmp(&0) {
                Ordering::Greater => {
                    calculation
                        .add_total_collateral(worst_case_weighted_token_value.cast::<i128>()?)?;

                    #[cfg(feature = "vortex-rs")]
                    calculation.add_spot_asset_value(worst_case_token_value)?;
                }
                Ordering::Less => {
                    validate!(
                        worst_case_weighted_token_value.unsigned_abs() >= worst_case_token_value.unsigned_abs(),
                        DexError::InvalidMarginRatio,
                        "weighted_token_value < abs(worst_case_token_value) in spot market_index={}",
                        spot_market.market_index,
                    )?;

                    validate!(
                        worst_case_weighted_token_value != 0,
                        DexError::InvalidOracle,
                        "weighted_token_value=0 for worst_case_token_amount={} in spot market_index={}",
                        worst_case_token_amount,
                        spot_market.market_index,
                    )?;

                    calculation.add_margin_requirement(
                        worst_case_weighted_token_value.unsigned_abs(),
                        worst_case_token_value.unsigned_abs(),
                        MarketIdentifier::spot(spot_market.market_index),
                    )?;

                    calculation.add_spot_liability()?;
                    calculation.update_with_spot_isolated_liability(
                        spot_market.asset_tier == AssetTier::Isolated,
                    );

                    #[cfg(feature = "vortex-rs")]
                    calculation.add_spot_liability_value(worst_case_token_value.unsigned_abs())?;
                }
                Ordering::Equal => {
                    if spot_position.has_open_order() {
                        calculation.add_spot_liability()?;
                        calculation.update_with_spot_isolated_liability(
                            spot_market.asset_tier == AssetTier::Isolated,
                        );
                    }
                }
            }

            match worst_case_orders_value.cmp(&0) {
                Ordering::Greater => {
                    calculation.add_total_collateral(worst_case_orders_value.cast::<i128>()?)?;

                    #[cfg(feature = "vortex-rs")]
                    calculation.add_spot_asset_value(worst_case_orders_value)?;
                }
                Ordering::Less => {
                    calculation.add_margin_requirement(
                        worst_case_orders_value.unsigned_abs(),
                        worst_case_orders_value.unsigned_abs(),
                        MarketIdentifier::spot(0),
                    )?;

                    #[cfg(feature = "vortex-rs")]
                    calculation.add_spot_liability_value(worst_case_orders_value.unsigned_abs())?;
                }
                Ordering::Equal => {}
            }
        }
    }


    calculation.validate_num_spot_liabilities()?;

    Ok(calculation)
}

pub fn validate_any_isolated_tier_requirements(
    user: &User,
    calculation: MarginCalculation,
) -> VortexDexResult {


    if calculation.with_spot_isolated_liability && !user.is_reduce_only() {
        validate!(
            calculation.num_spot_liabilities == 1,
            DexError::IsolatedAssetTierViolation,
            "User attempting to increase liabilities above 1 with a isolated tier liability"
        )?;
    }

    Ok(())
}

pub fn meets_withdraw_margin_requirement(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    margin_requirement_type: MarginRequirementType,
) -> VortexDexResult<bool> {
    let strict = margin_requirement_type == MarginRequirementType::Initial;
    let context = MarginContext::standard(margin_requirement_type).strict(strict);

    let calculation = calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
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

    validate_any_isolated_tier_requirements(user, calculation)?;

    validate!(
        calculation.meets_margin_requirement(),
        DexError::InsufficientCollateral,
        "User attempting to withdraw where total_collateral {} is below initial_margin_requirement {}",
        calculation.total_collateral,
        calculation.margin_requirement
    )?;

    Ok(true)
}

pub fn meets_place_order_margin_requirement(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    risk_increasing: bool,
) -> VortexDexResult {
    let margin_type = if risk_increasing {
        MarginRequirementType::Initial
    } else {
        MarginRequirementType::Maintenance
    };
    let context = MarginContext::standard(margin_type).strict(true);

    let calculation = calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
        spot_market_map,
        oracle_map,
        context,
    )?;

    if !calculation.meets_margin_requirement() {
        msg!(
            "total_collateral={}, margin_requirement={} margin type = {:?}",
            calculation.total_collateral,
            calculation.margin_requirement,
            margin_type
        );
        return Err(DexError::InsufficientCollateral);
    }

    validate_any_isolated_tier_requirements(user, calculation)?;

    Ok(())
}

pub fn meets_initial_margin_requirement(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
) -> VortexDexResult<bool> {
    calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
        spot_market_map,
        oracle_map,
        MarginContext::standard(MarginRequirementType::Initial),
    )
    .map(|calc| calc.meets_margin_requirement())
}

pub fn meets_settle_pnl_maintenance_margin_requirement(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
) -> VortexDexResult<bool> {
    calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
        spot_market_map,
        oracle_map,
        MarginContext::standard(MarginRequirementType::Maintenance).strict(true),
    )
    .map(|calc| calc.meets_margin_requirement())
}

pub fn meets_maintenance_margin_requirement(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
) -> VortexDexResult<bool> {
    calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
        spot_market_map,
        oracle_map,
        MarginContext::standard(MarginRequirementType::Maintenance),
    )
    .map(|calc| calc.meets_margin_requirement())
}

pub fn calculate_max_withdrawable_amount(
    market_index: u16,
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
) -> VortexDexResult<u64> {
    let calculation = calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
        spot_market_map,
        oracle_map,
        MarginContext::standard(MarginRequirementType::Initial),
    )?;

    let spot_market = &mut spot_market_map.get_ref(&market_index)?;

    let token_amount = user
        .get_spot_position(market_index)?
        .get_token_amount(spot_market)?;

    let oracle_price = oracle_map.get_price_data(&spot_market.oracle)?.price;

    let asset_weight = spot_market.get_asset_weight(
        token_amount,
        oracle_price,
        &MarginRequirementType::Initial,
    )?;

    if asset_weight == 0 {
        return Ok(u64::MAX);
    }

    if calculation.get_num_of_liabilities()? == 0 {
        // user has small dust deposit and no liabilities
        // so return early with user tokens amount
        return token_amount.cast();
    }

    let free_collateral = calculation.get_free_collateral()?;

    let precision_increase = 10u128.pow(spot_market.decimals - 6);

    free_collateral
        .safe_mul(MARGIN_PRECISION_U128)?
        .safe_div(asset_weight.cast()?)?
        .safe_mul(PRICE_PRECISION)?
        .safe_div(oracle_price.cast()?)?
        .safe_mul(precision_increase)?
        .cast()
}

pub fn validate_spot_margin_trading(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
) -> VortexDexResult {

    let mut total_open_bids_value = 0_i128;
    for spot_position in &user.spot_positions {
        let asks = spot_position.open_asks;
        if asks < 0 {
            let spot_market = spot_market_map.get_ref(&spot_position.market_index)?;
            let signed_token_amount = spot_position.get_signed_token_amount(&spot_market)?;
            // The user can have:
            // 1. no open asks with an existing short
            // 2. open asks with a larger existing long
            validate!(
                signed_token_amount.safe_add(asks.cast()?)? >= 0,
                DexError::MarginTradingDisabled,
                "Open asks can lead to increased borrow in spot market {}",
                spot_position.market_index
            )?;
        }

        let bids = spot_position.open_bids;
        if bids > 0 {
            let spot_market = spot_market_map.get_ref(&spot_position.market_index)?;
            let oracle_price_data = oracle_map.get_price_data(&spot_market.oracle)?;
            let open_bids_value =
                get_token_value(-bids as i128, spot_market.decimals, oracle_price_data.price)?;

            total_open_bids_value = total_open_bids_value.safe_add(open_bids_value)?;
        }
    }

    let mut quote_token_amount = 0_i128;
    let quote_spot_position = user.get_quote_spot_position();
    if !quote_spot_position.is_available() {
        let quote_spot_market = spot_market_map.get_quote_spot_market()?;
        quote_token_amount = quote_spot_position.get_signed_token_amount(&quote_spot_market)?;
    }

    // The user can have open bids if their value is less than existing quote token amount
    validate!(
        total_open_bids_value == 0 || quote_token_amount.safe_add(total_open_bids_value)? >= 0,
        DexError::MarginTradingDisabled,
        "Open bids leads to increased borrow for spot market 0"
    )?;

    Ok(())
}

pub fn calculate_user_equity(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
) -> VortexDexResult<(i128, bool)> {
    let mut net_usd_value: i128 = 0;
    let mut all_oracles_valid = true;

    for spot_position in user.spot_positions.iter() {
        if spot_position.is_available() {
            continue;
        }

        let spot_market = spot_market_map.get_ref(&spot_position.market_index)?;
        let (oracle_price_data, oracle_validity) = oracle_map.get_price_data_and_validity(
            MarketType::Spot,
            spot_market.market_index,
            &spot_market.oracle,
            spot_market.historical_oracle_data.last_oracle_price_twap,
            spot_market.get_max_confidence_interval_multiplier()?,
        )?;
        all_oracles_valid &=
            is_oracle_valid_for_action(oracle_validity, Some(VortexDexAction::MarginCalc))?;

        let token_amount = spot_position.get_signed_token_amount(&spot_market)?;
        let oracle_price = oracle_price_data.price;
        let token_value = get_token_value(token_amount, spot_market.decimals, oracle_price)?;

        net_usd_value = net_usd_value.safe_add(token_value)?;
    }

    //Add support for pep positions later

    Ok((net_usd_value, all_oracles_valid))
}