use crate::{errors::{DexError, VortexDexResult}, state::{margin_calculation::MarginContext, oracle_map::OracleMap, spot_market_map::SpotMarketMap, user::User}};

use super::margin_utils::calculate_margin_requirement_and_total_collateral_and_liability_info;


pub fn is_user_being_liquidated(
    user: &User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    liquidation_margin_buffer_ratio: u32,
) -> VortexDexResult<bool> {
    let margin_calculation = calculate_margin_requirement_and_total_collateral_and_liability_info(
        user,
        spot_market_map,
        oracle_map,
        MarginContext::liquidation(liquidation_margin_buffer_ratio),
    )?;

    let is_being_liquidated = !margin_calculation.can_exit_liquidation()?;

    Ok(is_being_liquidated)
}

pub fn validate_user_not_being_liquidated(
    user: &mut User,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    liquidation_margin_buffer_ratio: u32,
) -> VortexDexResult {
    if !user.is_being_liquidated() {
        return Ok(());
    }

    let is_still_being_liquidated = is_user_being_liquidated(
        user,
        spot_market_map,
        oracle_map,
        liquidation_margin_buffer_ratio,
    )?;

    if is_still_being_liquidated {
        return Err(DexError::UserIsBeingLiquidated);
    } else {
        user.exit_liquidation()
    }

    Ok(())
}