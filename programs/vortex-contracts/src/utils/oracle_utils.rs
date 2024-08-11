use std::{cmp::max, fmt};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::msg;

use crate::{errors::{DexError, VortexDexResult}, state::{constants::BID_ASK_SPREAD_PRECISION, dex_state::ValidityGuardRails, oracle::OraclePriceData, user::MarketType}};

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq, Default)]
pub enum OracleValidity {
    NonPositive,
    TooVolatile,
    TooUncertain,
    StaleForMargin,
    InsufficientDataPoints,
    StaleForAMM,
    #[default]
    Valid,
}

impl OracleValidity {
    pub fn get_error_code(&self) -> DexError {
        match self {
            OracleValidity::NonPositive => DexError::OracleNonPositive,
            OracleValidity::TooVolatile => DexError::OracleTooVolatile,
            OracleValidity::TooUncertain => DexError::OracleTooUncertain,
            OracleValidity::StaleForMargin => DexError::OracleStaleForMargin,
            OracleValidity::InsufficientDataPoints => DexError::OracleInsufficientDataPoints,
            OracleValidity::StaleForAMM => DexError::OracleStaleForAMM,
            OracleValidity::Valid => unreachable!(),
        }
    }
}

impl fmt::Display for OracleValidity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OracleValidity::NonPositive => write!(f, "NonPositive"),
            OracleValidity::TooVolatile => write!(f, "TooVolatile"),
            OracleValidity::TooUncertain => write!(f, "TooUncertain"),
            OracleValidity::StaleForMargin => write!(f, "StaleForMargin"),
            OracleValidity::InsufficientDataPoints => write!(f, "InsufficientDataPoints"),
            OracleValidity::StaleForAMM => write!(f, "StaleForAMM"),
            OracleValidity::Valid => write!(f, "Valid"),
        }
    }
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum VortexDexAction {
    UpdateFunding,
    SettlePnl,
    TriggerOrder,
    FillOrderMatch,
    FillOrderAmm,
    Liquidate,
    MarginCalc,
    UpdateTwap,
    UpdateAMMCurve,
    OracleOrderPrice,
}

pub fn is_oracle_valid_for_action(
    oracle_validity: OracleValidity,
    action: Option<VortexDexAction>,
) -> VortexDexResult<bool> {
    let is_ok = match action {
        Some(action) => match action {
            VortexDexAction::FillOrderAmm => {
                matches!(oracle_validity, OracleValidity::Valid)
            }
            // relax oracle staleness, later checks for sufficiently recent amm slot update for funding update
            VortexDexAction::UpdateFunding => {
                matches!(
                    oracle_validity,
                    OracleValidity::Valid
                        | OracleValidity::StaleForAMM
                        | OracleValidity::InsufficientDataPoints
                        | OracleValidity::StaleForMargin
                )
            }
            VortexDexAction::OracleOrderPrice => {
                matches!(
                    oracle_validity,
                    OracleValidity::Valid
                        | OracleValidity::StaleForAMM
                        | OracleValidity::InsufficientDataPoints
                )
            }
            VortexDexAction::MarginCalc => !matches!(
                oracle_validity,
                OracleValidity::NonPositive
                    | OracleValidity::TooVolatile
                    | OracleValidity::TooUncertain
                    | OracleValidity::StaleForMargin
            ),
            VortexDexAction::TriggerOrder => !matches!(
                oracle_validity,
                OracleValidity::NonPositive | OracleValidity::TooVolatile
            ),
            VortexDexAction::SettlePnl => matches!(
                oracle_validity,
                OracleValidity::Valid
                    | OracleValidity::StaleForAMM
                    | OracleValidity::InsufficientDataPoints
                    | OracleValidity::StaleForMargin
            ),
            VortexDexAction::FillOrderMatch => !matches!(
                oracle_validity,
                OracleValidity::NonPositive
                    | OracleValidity::TooVolatile
                    | OracleValidity::TooUncertain
            ),
            VortexDexAction::Liquidate => !matches!(
                oracle_validity,
                OracleValidity::NonPositive | OracleValidity::TooVolatile
            ),
            VortexDexAction::UpdateTwap => !matches!(oracle_validity, OracleValidity::NonPositive),
            VortexDexAction::UpdateAMMCurve => !matches!(oracle_validity, OracleValidity::NonPositive),
        },
        None => {
            matches!(oracle_validity, OracleValidity::Valid)
        }
    };

    Ok(is_ok)
}



#[derive(Default, Clone, Copy, Debug)]
pub struct OracleStatus {
    pub price_data: OraclePriceData,
    pub oracle_reserve_price_spread_pct: i64,
    pub mark_too_divergent: bool,
    pub oracle_validity: OracleValidity,
}



pub fn oracle_validity(
    market_type: MarketType,
    market_index: u16,
    last_oracle_twap: i64,
    oracle_price_data: &OraclePriceData,
    valid_oracle_guard_rails: &ValidityGuardRails,
    max_confidence_interval_multiplier: u64,
    log_validity: bool,
) -> VortexDexResult<OracleValidity> {
    let OraclePriceData {
        price: oracle_price,
        confidence: oracle_conf,
        delay: oracle_delay,
        has_sufficient_number_of_data_points,
        ..
    } = *oracle_price_data;

    let is_oracle_price_nonpositive = oracle_price <= 0;

    let is_oracle_price_too_volatile = (oracle_price.max(last_oracle_twap))
        .safe_div(last_oracle_twap.min(oracle_price).max(1))?
        .gt(&valid_oracle_guard_rails.too_volatile_ratio);

    let conf_pct_of_price = max(1, oracle_conf)
        .safe_mul(BID_ASK_SPREAD_PRECISION)?
        .safe_div(oracle_price.cast()?)?;

    // TooUncertain
    let is_conf_too_large = conf_pct_of_price.gt(&valid_oracle_guard_rails
        .confidence_interval_max_size
        .safe_mul(max_confidence_interval_multiplier)?);

    let is_stale_for_amm = oracle_delay.gt(&valid_oracle_guard_rails.slots_before_stale_for_amm);
    let is_stale_for_margin =
        oracle_delay.gt(&valid_oracle_guard_rails.slots_before_stale_for_margin);

    let oracle_validity = if is_oracle_price_nonpositive {
        OracleValidity::NonPositive
    } else if is_oracle_price_too_volatile {
        OracleValidity::TooVolatile
    } else if is_conf_too_large {
        OracleValidity::TooUncertain
    } else if is_stale_for_margin {
        OracleValidity::StaleForMargin
    } else if !has_sufficient_number_of_data_points {
        OracleValidity::InsufficientDataPoints
    } else if is_stale_for_amm {
        OracleValidity::StaleForAMM
    } else {
        OracleValidity::Valid
    };

    if log_validity {
        if !has_sufficient_number_of_data_points {
            msg!(
                "Invalid {} {} Oracle: Insufficient Data Points",
                market_type,
                market_index
            );
        }

        if is_oracle_price_nonpositive {
            msg!(
                "Invalid {} {} Oracle: Non-positive (oracle_price <=0)",
                market_type,
                market_index
            );
        }

        if is_oracle_price_too_volatile {
            msg!(
                "Invalid {} {} Oracle: Too Volatile (last_oracle_price_twap={:?} vs oracle_price={:?})",
                market_type,
                market_index,
                last_oracle_twap,
                oracle_price,
            );
        }

        if is_conf_too_large {
            msg!(
                "Invalid {} {} Oracle: Confidence Too Large (is_conf_too_large={:?})",
                market_type,
                market_index,
                conf_pct_of_price
            );
        }

        if is_stale_for_amm || is_stale_for_margin {
            msg!(
                "Invalid {} {} Oracle: Stale (oracle_delay={:?})",
                market_type,
                market_index,
                oracle_delay
            );
        }
    }

    Ok(oracle_validity)
}
