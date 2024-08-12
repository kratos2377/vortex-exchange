use crate::{errors::VortexDexResult, state::spot_market::SpotMarket, utils::constants::{FUEL_WINDOW_U128, QUOTE_PRECISION, QUOTE_PRECISION_U64}};

pub fn calculate_spot_fuel_bonus(
    spot_market: &SpotMarket,
    signed_token_value: i128,
    fuel_bonus_numerator: i64,
) -> VortexDexResult<u64> {
    let result: u64 = if signed_token_value.unsigned_abs() < QUOTE_PRECISION {
        0_u64
    } else if signed_token_value > 0 {
        signed_token_value
            .unsigned_abs()
            .safe_mul(fuel_bonus_numerator.cast()?)?
            .safe_mul(spot_market.fuel_boost_deposits.cast()?)?
            .safe_div(FUEL_WINDOW_U128)?
            .cast::<u64>()?
            / (QUOTE_PRECISION_U64 / 10)
    } else {
        signed_token_value
            .unsigned_abs()
            .safe_mul(fuel_bonus_numerator.cast()?)?
            .safe_mul(spot_market.fuel_boost_borrows.cast()?)?
            .safe_div(FUEL_WINDOW_U128)?
            .cast::<u64>()?
            / (QUOTE_PRECISION_U64 / 10)
    };

    Ok(result)
}

pub fn calculate_insurance_fuel_bonus(
    spot_market: &SpotMarket,
    stake_amount: u64,
    stake_amount_delta: i64,
    fuel_bonus_numerator: u32,
) -> VortexDexResult<u64> {
    Ok(stake_amount
        .saturating_sub(stake_amount_delta.unsigned_abs())
        .cast::<u128>()?
        .safe_mul(fuel_bonus_numerator.cast()?)?
        .safe_mul(spot_market.fuel_boost_insurance.cast()?)?
        .safe_div(FUEL_WINDOW_U128)?
        .cast::<u64>()?
        / (QUOTE_PRECISION_U64 / 10))
}
