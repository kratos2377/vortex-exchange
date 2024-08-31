use anchor_lang::prelude::*;
use std::cell::Ref;
use solana_program::msg;
use crate::safe_methods::SafeUnwrap;
use crate::state::load_ref::load_ref;
use crate::{casting::Cast, errors::{DexError, VortexDexResult}, safe_methods::SafeMath, utils::constants::{PRICE_PRECISION, PRICE_PRECISION_I64, PRICE_PRECISION_U64}, validate};




#[derive(Default, AnchorSerialize, AnchorDeserialize, Clone, Copy, Eq, PartialEq, Debug)]
pub struct HistoricalOracleData {
    /// precision: PRICE_PRECISION
    pub last_oracle_price: i64,
    /// precision: PRICE_PRECISION
    pub last_oracle_conf: u64,
    /// number of slots since last update
    pub last_oracle_delay: i64,
    /// precision: PRICE_PRECISION
    pub last_oracle_price_twap: i64,
    /// precision: PRICE_PRECISION
    pub last_oracle_price_twap_5min: i64,
    /// unix_timestamp of last snapshot
    pub last_oracle_price_twap_ts: i64,
}

impl HistoricalOracleData {
    pub fn default_quote_oracle() -> Self {
        HistoricalOracleData {
            last_oracle_price: PRICE_PRECISION_I64,
            last_oracle_conf: 0,
            last_oracle_delay: 0,
            last_oracle_price_twap: PRICE_PRECISION_I64,
            last_oracle_price_twap_5min: PRICE_PRECISION_I64,
            ..HistoricalOracleData::default()
        }
    }

    pub fn default_price(price: i64) -> Self {
        HistoricalOracleData {
            last_oracle_price: price,
            last_oracle_conf: 0,
            last_oracle_delay: 10,
            last_oracle_price_twap: price,
            last_oracle_price_twap_5min: price,
            ..HistoricalOracleData::default()
        }
    }

    pub fn default_with_current_oracle(oracle_price_data: OraclePriceData) -> Self {
        HistoricalOracleData {
            last_oracle_price: oracle_price_data.price,
            last_oracle_conf: oracle_price_data.confidence,
            last_oracle_delay: oracle_price_data.delay,
            last_oracle_price_twap: oracle_price_data.price,
            last_oracle_price_twap_5min: oracle_price_data.price,
            // last_oracle_price_twap_ts: now,
            ..HistoricalOracleData::default()
        }
    }
}

#[derive(Default, AnchorSerialize, AnchorDeserialize, Clone, Copy, Eq, PartialEq, Debug)]
pub struct HistoricalIndexData {
    /// precision: PRICE_PRECISION
    pub last_index_bid_price: u64,
    /// precision: PRICE_PRECISION
    pub last_index_ask_price: u64,
    /// precision: PRICE_PRECISION
    pub last_index_price_twap: u64,
    /// precision: PRICE_PRECISION
    pub last_index_price_twap_5min: u64,
    /// unix_timestamp of last snapshot
    pub last_index_price_twap_ts: i64,
}

impl HistoricalIndexData {
    pub fn default_quote_oracle() -> Self {
        HistoricalIndexData {
            last_index_bid_price: PRICE_PRECISION_U64,
            last_index_ask_price: PRICE_PRECISION_U64,
            last_index_price_twap: PRICE_PRECISION_U64,
            last_index_price_twap_5min: PRICE_PRECISION_U64,
            ..HistoricalIndexData::default()
        }
    }

    pub fn default_with_current_oracle(oracle_price_data: OraclePriceData) -> VortexDexResult<Self> {
        let price = oracle_price_data.price.cast::<u64>().safe_unwrap()?;
        Ok(HistoricalIndexData {
            last_index_bid_price: price,
            last_index_ask_price: price,
            last_index_price_twap: price,
            last_index_price_twap_5min: price,
            ..HistoricalIndexData::default()
        })
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Eq, PartialEq, Debug, Default)]
pub enum OracleSource {
    #[default]
    Pyth,
    QuoteAsset,
    Pyth1K,
    Pyth1M,
    PythStableCoin,
    Prelaunch,
    PythPull,
    Pyth1KPull,
    Pyth1MPull,
    PythStableCoinPull,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct OraclePriceData {
    pub price: i64,
    pub confidence: u64,
    pub delay: i64,
    pub has_sufficient_number_of_data_points: bool,
}

impl OraclePriceData {
    pub fn default_usd() -> Self {
        OraclePriceData {
            price: PRICE_PRECISION_I64,
            confidence: 1,
            delay: 0,
            has_sufficient_number_of_data_points: true,
        }
    }
}

pub fn get_oracle_price(
    oracle_source: &OracleSource,
    price_oracle: &AccountInfo,
    clock_slot: u64,
) -> VortexDexResult<OraclePriceData> {
    match oracle_source {
        OracleSource::Pyth => get_pyth_price(price_oracle, clock_slot, 1, false),
        OracleSource::Pyth1K => get_pyth_price(price_oracle, clock_slot, 1000, false),
        OracleSource::Pyth1M => get_pyth_price(price_oracle, clock_slot, 1000000, false),
        OracleSource::PythStableCoin => get_pyth_stable_coin_price(price_oracle, clock_slot, false),
        OracleSource::QuoteAsset => Ok(OraclePriceData {
            price: PRICE_PRECISION_I64,
            confidence: 1,
            delay: 0,
            has_sufficient_number_of_data_points: true,
        }),
        OracleSource::Prelaunch => get_prelaunch_price(price_oracle, clock_slot),
        OracleSource::PythPull => get_pyth_price(price_oracle, clock_slot, 1, true),
        OracleSource::Pyth1KPull => get_pyth_price(price_oracle, clock_slot, 1000, true),
        OracleSource::Pyth1MPull => get_pyth_price(price_oracle, clock_slot, 1000000, true),
        OracleSource::PythStableCoinPull => {
            get_pyth_stable_coin_price(price_oracle, clock_slot, true)
        }
    }
}

pub fn get_pyth_price(
    price_oracle: &AccountInfo,
    clock_slot: u64,
    multiple: u128,
    is_pull_oracle: bool,
) -> VortexDexResult<OraclePriceData> {
    let mut pyth_price_data: &[u8] = &price_oracle
        .try_borrow_data()
        .or(Err(crate::errors::DexError::UnableToLoadOracle))?;

    let oracle_price: i64;
    let oracle_conf: u64;
    let mut has_sufficient_number_of_data_points: bool = true;
    let mut oracle_precision: u128;
    let published_slot: u64;

    if is_pull_oracle {
        // Pyth program not working right now, will fix it later
        // let price_message = pyth_solana_receiver_sdk::price_update::PriceUpdateV2::try_deserialize(
        //     &mut pyth_price_data,
        // )
        // .unwrap();
         oracle_price = 0;
         oracle_conf = 0;
         oracle_precision = 0;
        // published_slot = price_message.posted_slot;
        published_slot = 0
    } else {
        let price_data = pyth_client::cast::<pyth_client::Price>(pyth_price_data);
        oracle_price = price_data.agg.price;
        oracle_conf = price_data.agg.conf;
        let min_publishers = price_data.num.min(3);
        let publisher_count = price_data.num_qt;

        #[cfg(feature = "mainnet-beta")]
        {
            has_sufficient_number_of_data_points = publisher_count >= min_publishers;
        }
        #[cfg(not(feature = "mainnet-beta"))]
        {
            has_sufficient_number_of_data_points = true;
        }

        oracle_precision = 10_u128.pow(price_data.expo.unsigned_abs());
        published_slot = price_data.valid_slot;
    }

    if oracle_precision <= multiple {
        msg!("Multiple larger than oracle precision");
        return Err(crate::errors::DexError::InvalidOracle);
    }
    oracle_precision = oracle_precision.safe_div(multiple)?;

    let mut oracle_scale_mult = 1;
    let mut oracle_scale_div = 1;

    if oracle_precision > PRICE_PRECISION {
        oracle_scale_div = oracle_precision.safe_div(PRICE_PRECISION)?;
    } else {
        oracle_scale_mult = PRICE_PRECISION.safe_div(oracle_precision)?;
    }

    let oracle_price_scaled = (oracle_price)
        .cast::<i128>()?
        .safe_mul(oracle_scale_mult.cast()?)?
        .safe_div(oracle_scale_div.cast()?)?
        .cast::<i64>()?;

    let oracle_conf_scaled = (oracle_conf)
        .cast::<u128>()?
        .safe_mul(oracle_scale_mult)?
        .safe_div(oracle_scale_div)?
        .cast::<u64>()?;

    let oracle_delay: i64 = clock_slot.cast::<i64>()?.safe_sub(published_slot.cast()?)?;

    Ok(OraclePriceData {
        price: oracle_price_scaled,
        confidence: oracle_conf_scaled,
        delay: oracle_delay,
        has_sufficient_number_of_data_points,
    })
}

pub fn get_pyth_stable_coin_price(
    price_oracle: &AccountInfo,
    clock_slot: u64,
    is_pull_oracle: bool,
) -> VortexDexResult<OraclePriceData> {
    let mut oracle_price_data = get_pyth_price(price_oracle, clock_slot, 1, is_pull_oracle)?;

    let price = oracle_price_data.price;
    let confidence = oracle_price_data.confidence;
    let five_bps = 500_i64;

    if price.safe_sub(PRICE_PRECISION_I64)?.abs() <= five_bps.min(confidence.cast()?) {
        oracle_price_data.price = PRICE_PRECISION_I64;
    }

    Ok(oracle_price_data)
}



pub fn get_prelaunch_price(price_oracle: &AccountInfo, slot: u64) -> VortexDexResult<OraclePriceData> {
    let oracle: Ref<PrelaunchOracle> = load_ref(price_oracle).or(Err(DexError::UnableToLoadOracle))?;

    Ok(OraclePriceData {
        price: oracle.price,
        confidence: oracle.confidence,
        delay: oracle.amm_last_update_slot.saturating_sub(slot).cast()?,
        has_sufficient_number_of_data_points: true,
    })
}

#[derive(Clone, Copy)]
pub struct StrictOraclePrice {
    pub current: i64,
    pub twap_5min: Option<i64>,
}

impl StrictOraclePrice {
    pub fn new(price: i64, twap_5min: i64, enabled: bool) -> Self {
        Self {
            current: price,
            twap_5min: if enabled { Some(twap_5min) } else { None },
        }
    }

    pub fn max(&self) -> i64 {
        match self.twap_5min {
            Some(twap) => self.current.max(twap),
            None => self.current,
        }
    }

    pub fn min(&self) -> i64 {
        match self.twap_5min {
            Some(twap) => self.current.min(twap),
            None => self.current,
        }
    }

    pub fn validate(&self) -> VortexDexResult {
        validate!(
            self.current > 0,
            DexError::InvalidOracle,
            "oracle_price_data={} (<= 0)",
            self.current,
        )?;

        if let Some(twap) = self.twap_5min {
            validate!(
                twap > 0,
                DexError::InvalidOracle,
                "oracle_price_twap={} (<= 0)",
                twap
            )?;
        }

        Ok(())
    }
}


#[account(zero_copy(unsafe))]
#[derive(Eq, PartialEq, Debug)]
#[repr(C)]
pub struct PrelaunchOracle {
    pub price: i64,
    pub max_price: i64,
    pub confidence: u64,
    // last slot oracle was updated, should be greater than or equal to last_update_slot
    pub last_update_slot: u64,
    // amm.last_update_slot at time oracle was updated
    pub amm_last_update_slot: u64,
    pub perp_market_index: u16,
    pub padding: [u8; 70],
}

impl PrelaunchOracle {
    
   pub const SIZE: usize = 112 + 8;
}

impl Default for PrelaunchOracle {

    fn default() -> Self {
        PrelaunchOracle {
            price: 0,
            max_price: 0,
            confidence: 0,
            last_update_slot: 0,
            amm_last_update_slot: 0,
            perp_market_index: 0,
            padding: [0; 70],
        }
    }
}


impl PrelaunchOracle {
    pub fn validate(&self) -> VortexDexResult {
        validate!(self.price != 0, DexError::InvalidOracle, "price == 0",)?;

        validate!(self.max_price != 0, DexError::InvalidOracle, "max price == 0",)?;

        validate!(
            self.price <= self.max_price,
            DexError::InvalidOracle,
            "price {} > max price {}",
            self.price,
            self.max_price
        )?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, AnchorSerialize, AnchorDeserialize, PartialEq, Eq)]
pub struct PrelaunchOracleParams {
    pub perp_market_index: u16,
    pub price: Option<i64>,
    pub max_price: Option<i64>,
}
