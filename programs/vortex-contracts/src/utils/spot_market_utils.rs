use solana_program::msg;

use crate::{errors::{DexError, VortexDexResult}, safe_methods::SafeMath, state::{oracle::{OraclePriceData, StrictOraclePrice}, spot_market::{SpotBalanceType, SpotMarket}, user::{SpotPosition, User}}, validate};

use super::{constants::{ONE_YEAR, SPOT_RATE_PRECISION, SPOT_UTILIZATION_PRECISION, SPOT_WEIGHT_PRECISION_U128}, margin_utils::MarginRequirementType};



pub fn get_spot_balance(
    token_amount: u128,
    spot_market: &SpotMarket,
    balance_type: &SpotBalanceType,
    round_up: bool,
) -> VortexDexResult<u128> {
    let precision_increase = 10_u128.pow(19_u32.safe_sub(spot_market.decimals)?);

    let cumulative_interest = match balance_type {
        SpotBalanceType::Deposit => spot_market.cumulative_deposit_interest,
        SpotBalanceType::Borrow => spot_market.cumulative_borrow_interest,
    };

    let mut balance = token_amount
        .safe_mul(precision_increase)?
        .safe_div(cumulative_interest)?;

    if round_up && balance != 0 {
        balance = balance.safe_add(1)?;
    }

    Ok(balance)
}

pub fn get_token_amount(
    balance: u128,
    spot_market: &SpotMarket,
    balance_type: &SpotBalanceType,
) -> VortexDexResult<u128> {
    let precision_decrease = 10_u128.pow(19_u32.safe_sub(spot_market.decimals)?);

    let cumulative_interest = match balance_type {
        SpotBalanceType::Deposit => spot_market.cumulative_deposit_interest,
        SpotBalanceType::Borrow => spot_market.cumulative_borrow_interest,
    };

    let token_amount = match balance_type {
        SpotBalanceType::Deposit => balance
            .safe_mul(cumulative_interest)?
            .safe_div(precision_decrease)?,
        SpotBalanceType::Borrow => balance
            .safe_mul(cumulative_interest)?
            .safe_div_ceil(precision_decrease)?,
    };

    Ok(token_amount)
}

pub fn get_signed_token_amount(
    token_amount: u128,
    balance_type: &SpotBalanceType,
) -> VortexDexResult<i128> {
    match balance_type {
        SpotBalanceType::Deposit => token_amount.cast(),
        SpotBalanceType::Borrow => token_amount
            .cast::<i128>()
            .map(|token_amount| -token_amount),
    }
}

pub fn get_interest_token_amount(
    balance: u128,
    spot_market: &SpotMarket,
    interest: u128,
) -> VortexDexResult<u128> {
    let precision_decrease = 10_u128.pow(19_u32.safe_sub(spot_market.decimals)?);

    let token_amount = balance.safe_mul(interest)?.safe_div(precision_decrease)?;

    Ok(token_amount)
}

pub struct InterestAccumulated {
    pub borrow_interest: u128,
    pub deposit_interest: u128,
}

pub fn calculate_utilization(
    deposit_token_amount: u128,
    borrow_token_amount: u128,
) -> VortexDexResult<u128> {
    let utilization = borrow_token_amount
        .safe_mul(SPOT_UTILIZATION_PRECISION)?
        .checked_div(deposit_token_amount)
        .unwrap_or({
            if deposit_token_amount == 0 && borrow_token_amount == 0 {
                0_u128
            } else {
                // if there are borrows without deposits, default to maximum utilization rate
                SPOT_UTILIZATION_PRECISION
            }
        });

    Ok(utilization)
}

pub fn calculate_spot_market_utilization(spot_market: &SpotMarket) -> VortexDexResult<u128> {
    let deposit_token_amount = get_token_amount(
        spot_market.deposit_balance,
        spot_market,
        &SpotBalanceType::Deposit,
    )?;
    let borrow_token_amount = get_token_amount(
        spot_market.borrow_balance,
        spot_market,
        &SpotBalanceType::Borrow,
    )?;
    let utilization = calculate_utilization(deposit_token_amount, borrow_token_amount)?;

    Ok(utilization)
}

pub fn calculate_accumulated_interest(
    spot_market: &SpotMarket,
    now: i64,
) -> VortexDexResult<InterestAccumulated> {
    let utilization = calculate_spot_market_utilization(spot_market)?;

    if utilization == 0 {
        return Ok(InterestAccumulated {
            borrow_interest: 0,
            deposit_interest: 0,
        });
    }

    let borrow_rate = if utilization > spot_market.optimal_utilization.cast()? {
        let surplus_utilization = utilization.safe_sub(spot_market.optimal_utilization.cast()?)?;

        let borrow_rate_slope = spot_market
            .max_borrow_rate
            .cast::<u128>()?
            .safe_sub(spot_market.optimal_borrow_rate.cast()?)?
            .safe_mul(SPOT_UTILIZATION_PRECISION)?
            .safe_div(
                SPOT_UTILIZATION_PRECISION.safe_sub(spot_market.optimal_utilization.cast()?)?,
            )?;

        spot_market.optimal_borrow_rate.cast::<u128>()?.safe_add(
            surplus_utilization
                .safe_mul(borrow_rate_slope)?
                .safe_div(SPOT_UTILIZATION_PRECISION)?,
        )?
    } else {
        let borrow_rate_slope = spot_market
            .optimal_borrow_rate
            .cast::<u128>()?
            .safe_mul(SPOT_UTILIZATION_PRECISION)?
            .safe_div(spot_market.optimal_utilization.cast()?)?;

        utilization
            .safe_mul(borrow_rate_slope)?
            .safe_div(SPOT_UTILIZATION_PRECISION)?
    }
    .max(spot_market.get_min_borrow_rate()?.cast()?);

    let time_since_last_update = now
        .cast::<u64>()
        .or(Err(DexError::UnableToCastUnixTime))?
        .safe_sub(spot_market.last_interest_ts)?;

    // To save some compute units, have to multiply the rate by the `time_since_last_update` here
    // and then divide out by ONE_YEAR when calculating interest accumulated below
    let modified_borrow_rate = borrow_rate.safe_mul(time_since_last_update as u128)?;

    let modified_deposit_rate = modified_borrow_rate
        .safe_mul(utilization)?
        .safe_div(SPOT_UTILIZATION_PRECISION)?;

    let borrow_interest = spot_market
        .cumulative_borrow_interest
        .safe_mul(modified_borrow_rate)?
        .safe_div(ONE_YEAR)?
        .safe_div(SPOT_RATE_PRECISION)?
        .safe_add(1)?;

    let deposit_interest = spot_market
        .cumulative_deposit_interest
        .safe_mul(modified_deposit_rate)?
        .safe_div(ONE_YEAR)?
        .safe_div(SPOT_RATE_PRECISION)?;

    Ok(InterestAccumulated {
        borrow_interest,
        deposit_interest,
    })
}

pub fn get_balance_value_and_token_amount(
    spot_position: &SpotPosition,
    spot_market: &SpotMarket,
    oracle_price_data: &OraclePriceData,
) -> VortexDexResult<(u128, u128)> {
    let token_amount = spot_position.get_token_amount(spot_market)?;

    let precision_decrease = 10_u128.pow(spot_market.decimals);

    let value = token_amount
        .safe_mul(oracle_price_data.price.cast()?)?
        .safe_div(precision_decrease)?;

    Ok((value, token_amount))
}

pub fn get_strict_token_value(
    token_amount: i128,
    spot_decimals: u32,
    strict_price: &StrictOraclePrice,
) -> VortexDexResult<i128> {
    if token_amount == 0 {
        return Ok(0);
    }

    let precision_decrease = 10_i128.pow(spot_decimals);

    let price = if token_amount > 0 {
        strict_price.min()
    } else {
        strict_price.max()
    };

    let token_with_price = token_amount.safe_mul(price.cast()?)?;

    if token_with_price < 0 {
        token_with_price.safe_div_floor(precision_decrease)
    } else {
        token_with_price.safe_div(precision_decrease)
    }
}

pub fn get_token_value(
    token_amount: i128,
    spot_decimals: u32,
    oracle_price: i64,
) -> VortexDexResult<i128> {
    if token_amount == 0 {
        return Ok(0);
    }

    let precision_decrease = 10_i128.pow(spot_decimals);
    let token_with_oracle = token_amount.safe_mul(oracle_price.cast()?)?;

    if token_with_oracle < 0 {
        token_with_oracle.safe_div_floor(precision_decrease.abs())
    } else {
        token_with_oracle.safe_div(precision_decrease)
    }
}

pub fn get_balance_value(
    spot_position: &SpotPosition,
    spot_market: &SpotMarket,
    oracle_price_data: &OraclePriceData,
) -> VortexDexResult<u128> {
    let (value, _) =
        get_balance_value_and_token_amount(spot_position, spot_market, oracle_price_data)?;
    Ok(value)
}


pub fn calculate_max_borrow_token_amount(
    deposit_token_amount: u128,
    deposit_token_twap: u128,
    borrow_token_twap: u128,
    withdraw_guard_threshold: u128,
    max_token_borrows: u128,
) -> VortexDexResult<u128> {
    // maximum permitted borrows after withdrawal
    // allows at least up to the withdraw_guard_threshold
    // and between ~15-80% utilization with friction on twap in 10% increments

    let lesser_deposit_amount = deposit_token_amount.min(deposit_token_twap);

    let max_borrow_token = withdraw_guard_threshold
        .max(
            (lesser_deposit_amount / 6)
                .max(borrow_token_twap.safe_add(lesser_deposit_amount / 10)?)
                .min(lesser_deposit_amount.safe_sub(lesser_deposit_amount / 5)?),
        )
        .min(max_token_borrows);

    Ok(max_borrow_token)
}


pub fn check_user_exception_to_withdraw_limits(
    spot_market: &SpotMarket,
    user: Option<&User>,
    token_amount_withdrawn: Option<u128>,
) -> VortexDexResult<bool> {
    // allow a smaller user in a market to bypass and withdraw their principal
    let mut valid_user_withdraw = false;
    if let Some(user) = user {
        let spot_position = user.get_spot_position(spot_market.market_index)?;
        let net_deposits = user
            .total_deposits
            .cast::<i128>()?
            .safe_sub(user.total_withdraws.cast::<i128>()?)?;
        msg!(
            "net_deposits={}({}-{})",
            net_deposits,
            user.total_deposits,
            user.total_withdraws
        );
        if net_deposits >= 0
            && spot_position.cumulative_deposits >= 0
            && spot_position.balance_type == SpotBalanceType::Deposit
        {
            if let Some(token_amount_withdrawn) = token_amount_withdrawn {
                let user_deposit_token_amount = get_token_amount(
                    spot_position.scaled_balance.cast::<u128>()?,
                    spot_market,
                    &spot_position.balance_type,
                )?;

                if user_deposit_token_amount.safe_add(token_amount_withdrawn)?
                    < spot_market
                        .withdraw_guard_threshold
                        .cast::<u128>()?
                        .safe_div(10)?
                {
                    valid_user_withdraw = true;
                }
            }
        }
    }

    Ok(valid_user_withdraw)
}

pub fn calculate_token_utilization_limits(
    deposit_token_amount: u128,
    borrow_token_amount: u128,
    spot_market: &SpotMarket,
) -> VortexDexResult<(u128, u128)> {
    // Calculates the allowable minimum deposit and maximum borrow amounts after withdrawal based on market utilization.
    // First, it determines a maximum withdrawal utilization from the market's target and historic utilization.
    // Then, it deduces corresponding deposit/borrow amounts.
    // Note: For deposit sizes below the guard threshold, withdrawals aren't blocked.

    let max_withdraw_utilization: u128 = spot_market.optimal_utilization.cast::<u128>()?.max(
        spot_market.utilization_twap.cast::<u128>()?.safe_add(
            SPOT_UTILIZATION_PRECISION.saturating_sub(spot_market.utilization_twap.cast()?) / 2,
        )?,
    );

    let mut min_deposit_tokens_for_utilization = borrow_token_amount
        .safe_mul(SPOT_UTILIZATION_PRECISION)?
        .safe_div(max_withdraw_utilization)?;

    // dont block withdraws for deposit sizes below guard threshold
    min_deposit_tokens_for_utilization = min_deposit_tokens_for_utilization
        .min(deposit_token_amount.saturating_sub(spot_market.withdraw_guard_threshold.cast()?));

    let mut max_borrow_tokens_for_utilization = max_withdraw_utilization
        .safe_mul(deposit_token_amount)?
        .safe_div(SPOT_UTILIZATION_PRECISION)?;

    // dont block borrows for sizes below guard threshold
    max_borrow_tokens_for_utilization =
        max_borrow_tokens_for_utilization.max(spot_market.withdraw_guard_threshold.cast()?);

    Ok((
        min_deposit_tokens_for_utilization,
        max_borrow_tokens_for_utilization,
    ))
}

pub fn calculate_min_deposit_token_amount(
    deposit_token_twap: u128,
    withdraw_guard_threshold: u128,
) -> VortexDexResult<u128> {
    // minimum required deposit amount after withdrawal
    // minimum deposit amount lower of 75% of TWAP or withdrawal guard threshold below TWAP
    // for high withdrawal guard threshold, minimum deposit amount is 0

    let min_deposit_token = deposit_token_twap
        .safe_sub((deposit_token_twap / 4).max(withdraw_guard_threshold.min(deposit_token_twap)))?;

    Ok(min_deposit_token)
}

pub fn check_withdraw_limits(
    spot_market: &SpotMarket,
    user: Option<&User>,
    token_amount_withdrawn: Option<u128>,
) -> VortexDexResult<bool> {
    // calculates min/max deposit/borrow amounts permitted for immediate withdraw
    // takes the stricter of absolute caps on level changes and utilization changes vs 24hr moving averrages
    let deposit_token_amount = get_token_amount(
        spot_market.deposit_balance,
        spot_market,
        &SpotBalanceType::Deposit,
    )?;
    let borrow_token_amount = get_token_amount(
        spot_market.borrow_balance,
        spot_market,
        &SpotBalanceType::Borrow,
    )?;

    let max_token_borrows: u128 = if spot_market.max_token_borrows_fraction > 0 {
        spot_market
            .max_token_deposits
            .safe_mul(spot_market.max_token_borrows_fraction.cast()?)?
            .safe_div(10000)?
            .cast()?
    } else {
        u128::MAX
    };

    let max_borrow_token_for_twap = calculate_max_borrow_token_amount(
        deposit_token_amount,
        spot_market.deposit_token_twap.cast()?,
        spot_market.borrow_token_twap.cast()?,
        spot_market.withdraw_guard_threshold.cast()?,
        max_token_borrows,
    )?;

    let (min_deposit_token_for_utilization, max_borrow_token_for_utilization) =
        calculate_token_utilization_limits(deposit_token_amount, borrow_token_amount, spot_market)?;

    let max_borrow_token = max_borrow_token_for_twap.min(max_borrow_token_for_utilization);

    let min_deposit_token_for_twap = calculate_min_deposit_token_amount(
        spot_market.deposit_token_twap.cast()?,
        spot_market.withdraw_guard_threshold.cast()?,
    )?;

    let min_deposit_token = min_deposit_token_for_twap.max(min_deposit_token_for_utilization);

    // for resulting deposit or ZERO, check if deposits above minimum
    // for resulting borrow, check both deposit and borrow constraints
    let valid_global_withdrawal = if let Some(user) = user {
        let spot_position_index = user.get_spot_position_index(spot_market.market_index)?;
        if user.spot_positions[spot_position_index].balance_type() == &SpotBalanceType::Borrow {
            borrow_token_amount <= max_borrow_token && deposit_token_amount >= min_deposit_token
        } else {
            deposit_token_amount >= min_deposit_token
        }
    } else {
        deposit_token_amount >= min_deposit_token && borrow_token_amount <= max_borrow_token
    };

    let valid_withdrawal = if !valid_global_withdrawal {
        msg!(
            "withdraw_guard_threshold={:?}",
            spot_market.withdraw_guard_threshold
        );
        msg!("min_deposit_token={:?}", min_deposit_token);
        msg!("deposit_token_amount={:?}", deposit_token_amount);
        msg!("max_borrow_token={:?}", max_borrow_token);
        msg!("borrow_token_amount={:?}", borrow_token_amount);

        check_user_exception_to_withdraw_limits(spot_market, user, token_amount_withdrawn)?
    } else {
        true
    };

    Ok(valid_withdrawal)
}

pub fn validate_spot_balances(spot_market: &SpotMarket) -> VortexDexResult<i64> {
    let depositors_amount: u64 = get_token_amount(
        spot_market.deposit_balance,
        spot_market,
        &SpotBalanceType::Deposit,
    )?
    .cast()?;
    let borrowers_amount: u64 = get_token_amount(
        spot_market.borrow_balance,
        spot_market,
        &SpotBalanceType::Borrow,
    )?
    .cast()?;

    let revenue_amount: u64 = get_token_amount(
        spot_market.revenue_pool.scaled_balance,
        spot_market,
        &SpotBalanceType::Deposit,
    )?
    .cast()?;

    let depositors_claim = depositors_amount
        .cast::<i64>()?
        .safe_sub(borrowers_amount.cast()?)?;

    validate!(
        revenue_amount <= depositors_amount,
        DexError::SpotMarketVaultInvariantViolated,
        "revenue_amount={} greater or equal to the depositors_amount={} (depositors_claim={}, spot_market.deposit_balance={})",
        revenue_amount,
        depositors_amount,
        depositors_claim,
        spot_market.deposit_balance
    )?;

    Ok(depositors_claim)
}


pub fn validate_spot_market_vault_amount(
    spot_market: &SpotMarket,
    vault_amount: u64,
) -> VortexDexResult<i64> {
    let depositors_claim = validate_spot_balances(spot_market)?;

    validate!(
        vault_amount.cast::<i64>()? >= depositors_claim,
        DexError::SpotMarketVaultInvariantViolated,
        "spot market vault ={} holds less than remaining depositor claims = {}",
        vault_amount,
        depositors_claim
    )?;

    Ok(depositors_claim)
}


pub fn get_max_withdraw_for_market_with_token_amount(
    spot_market: &SpotMarket,
    token_amount: i128,
    is_leaving_vortex: bool,
) -> VortexDexResult<u128> {
    let deposit_token_amount = get_token_amount(
        spot_market.deposit_balance,
        spot_market,
        &SpotBalanceType::Deposit,
    )?;

    let borrow_token_amount = get_token_amount(
        spot_market.borrow_balance,
        spot_market,
        &SpotBalanceType::Borrow,
    )?;

    // if leaving vortex, need to consider utilization limits
    let (min_deposit_token_for_utilization, max_borrow_token_for_utilization) = if is_leaving_vortex
    {
        calculate_token_utilization_limits(deposit_token_amount, borrow_token_amount, spot_market)?
    } else {
        (0, u128::MAX)
    };

    let mut max_withdraw_amount = 0_u128;
    if token_amount > 0 {
        let min_deposit_token_for_twap = calculate_min_deposit_token_amount(
            spot_market.deposit_token_twap.cast()?,
            spot_market.withdraw_guard_threshold.cast()?,
        )?;
        let min_deposit_token = min_deposit_token_for_twap.max(min_deposit_token_for_utilization);
        let withdraw_limit = deposit_token_amount.saturating_sub(min_deposit_token);

        let token_amount = token_amount.unsigned_abs();
        if withdraw_limit <= token_amount && is_leaving_vortex {
            return Ok(withdraw_limit);
        }

        max_withdraw_amount = token_amount;
    }

    let max_token_borrows: u128 = if spot_market.max_token_borrows_fraction > 0 {
        spot_market
            .max_token_deposits
            .safe_mul(spot_market.max_token_borrows_fraction.cast()?)?
            .safe_div(10000)?
            .cast()?
    } else {
        u128::MAX
    };

    let max_borrow_token_for_twap = calculate_max_borrow_token_amount(
        deposit_token_amount,
        spot_market.deposit_token_twap.cast()?,
        spot_market.borrow_token_twap.cast()?,
        spot_market.withdraw_guard_threshold.cast()?,
        max_token_borrows,
    )?;

    let max_borrow_token = max_borrow_token_for_twap.min(max_borrow_token_for_utilization);

    let mut borrow_limit = max_borrow_token
        .saturating_sub(borrow_token_amount)
        .min(deposit_token_amount.saturating_sub(borrow_token_amount));

    if spot_market.max_token_borrows_fraction > 0 {
        // min with max allowed borrows
        let borrows = spot_market.get_borrows()?;
        let max_token_borrows = spot_market
            .max_token_deposits
            .safe_mul(spot_market.max_token_borrows_fraction.cast()?)?
            .safe_div(10000)?
            .cast::<u128>()?;
        borrow_limit = borrow_limit.min(max_token_borrows.saturating_sub(borrows));
    }

    max_withdraw_amount.safe_add(borrow_limit)
}


pub fn select_margin_type_for_swap(
    in_market: &SpotMarket,
    out_market: &SpotMarket,
    in_strict_price: &StrictOraclePrice,
    out_strict_price: &StrictOraclePrice,
    in_token_amount_before: i128,
    out_token_amount_before: i128,
    in_token_amount_after: i128,
    out_token_amount_after: i128,
    strict_margin_type: MarginRequirementType,
) -> VortexDexResult<MarginRequirementType> {
    let calculate_free_collateral_contribution =
        |market: &SpotMarket, strict_oracle_price: &StrictOraclePrice, token_amount: i128| {
            let token_value =
                get_strict_token_value(token_amount, market.decimals, strict_oracle_price)?;

            let weight = if token_amount >= 0 {
                market.get_asset_weight(
                    token_amount.unsigned_abs(),
                    strict_oracle_price.current,
                    &MarginRequirementType::Initial,
                )?
            } else {
                market.get_liability_weight(
                    token_amount.unsigned_abs(),
                    &MarginRequirementType::Initial,
                )?
            };

            token_value
                .safe_mul(weight.cast::<i128>()?)?
                .safe_div(SPOT_WEIGHT_PRECISION_U128.cast()?)
        };

    let in_free_collateral_contribution_before =
        calculate_free_collateral_contribution(in_market, in_strict_price, in_token_amount_before)?;

    let out_free_collateral_contribution_before = calculate_free_collateral_contribution(
        out_market,
        out_strict_price,
        out_token_amount_before,
    )?;

    let free_collateral_contribution_before =
        in_free_collateral_contribution_before.safe_add(out_free_collateral_contribution_before)?;

    let in_free_collateral_contribution_after =
        calculate_free_collateral_contribution(in_market, in_strict_price, in_token_amount_after)?;

    let out_free_collateral_contribution_after = calculate_free_collateral_contribution(
        out_market,
        out_strict_price,
        out_token_amount_after,
    )?;

    let free_collateral_contribution_after =
        in_free_collateral_contribution_after.safe_add(out_free_collateral_contribution_after)?;

    let margin_type = if free_collateral_contribution_after > free_collateral_contribution_before {
        MarginRequirementType::Maintenance
    } else {
        strict_margin_type
    };

    Ok(margin_type)
}
