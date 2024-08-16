use crate::{errors::VortexDexResult, state::{oracle::OraclePriceData, position::PositionDirection}};

use super::constants::AUCTION_DERIVE_PRICE_FRACTION;

pub fn calculate_auction_prices(
    oracle_price_data: &OraclePriceData,
    direction: PositionDirection,
    limit_price: u64,
) -> VortexDexResult<(i64, i64)> {
    let oracle_price = oracle_price_data.price;
    let limit_price = limit_price.cast::<i64>()?;
    if limit_price > 0 {
        let (auction_start_price, auction_end_price) = match direction {
            // Long and limit price is better than oracle price
            PositionDirection::Long if limit_price < oracle_price => {
                let limit_derive_start_price =
                    limit_price.safe_sub(limit_price / AUCTION_DERIVE_PRICE_FRACTION)?;
                let oracle_derive_start_price =
                    oracle_price.safe_sub(oracle_price / AUCTION_DERIVE_PRICE_FRACTION)?;

                (
                    limit_derive_start_price.min(oracle_derive_start_price),
                    limit_price,
                )
            }
            // Long and limit price is worse than oracle price
            PositionDirection::Long if limit_price >= oracle_price => {
                let oracle_derive_end_price =
                    oracle_price.safe_add(oracle_price / AUCTION_DERIVE_PRICE_FRACTION)?;

                (oracle_price, limit_price.min(oracle_derive_end_price))
            }
            // Short and limit price is better than oracle price
            PositionDirection::Short if limit_price > oracle_price => {
                let limit_derive_start_price =
                    limit_price.safe_add(limit_price / AUCTION_DERIVE_PRICE_FRACTION)?;
                let oracle_derive_start_price =
                    oracle_price.safe_add(oracle_price / AUCTION_DERIVE_PRICE_FRACTION)?;

                (
                    limit_derive_start_price.max(oracle_derive_start_price),
                    limit_price,
                )
            }
            // Short and limit price is worse than oracle price
            PositionDirection::Short if limit_price <= oracle_price => {
                let oracle_derive_end_price =
                    oracle_price.safe_sub(oracle_price / AUCTION_DERIVE_PRICE_FRACTION)?;

                (oracle_price, limit_price.max(oracle_derive_end_price))
            }
            _ => unreachable!(),
        };

        return Ok((auction_start_price, auction_end_price));
    }

    let auction_end_price = match direction {
        PositionDirection::Long => {
            oracle_price.safe_add(oracle_price / AUCTION_DERIVE_PRICE_FRACTION)?
        }
        PositionDirection::Short => {
            oracle_price.safe_sub(oracle_price / AUCTION_DERIVE_PRICE_FRACTION)?
        }
    };

    Ok((oracle_price, auction_end_price))
}