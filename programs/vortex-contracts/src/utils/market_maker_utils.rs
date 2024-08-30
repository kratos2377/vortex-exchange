use crate::{errors::VortexDexResult, safe_methods::SafeMath, utils::constants::DEFAULT_MAX_TWAP_UPDATE_PRICE_BAND_DENOMINATOR};

pub fn sanitize_new_price(
    new_price: i64,
    last_price_twap: i64,
    sanitize_clamp_denominator: Option<i64>,
) -> VortexDexResult<i64> {
    // when/if twap is 0, dont try to normalize new_price
    if last_price_twap == 0 {
        return Ok(new_price);
    }

    let new_price_spread = new_price.safe_sub(last_price_twap)?;

    // cap new oracle update to 100/MAX_TWAP_UPDATE_PRICE_BAND_DENOMINATOR% delta from twap
    let sanitize_clamp_denominator =
        if let Some(sanitize_clamp_denominator) = sanitize_clamp_denominator {
            sanitize_clamp_denominator
        } else {
            DEFAULT_MAX_TWAP_UPDATE_PRICE_BAND_DENOMINATOR
        };

    if sanitize_clamp_denominator == 0 {
        // no need to use price band check
        return Ok(new_price);
    }

    let price_twap_price_band = last_price_twap.safe_div(sanitize_clamp_denominator)?;

    let capped_update_price =
        if new_price_spread.unsigned_abs() > price_twap_price_band.unsigned_abs() {
            if new_price > last_price_twap {
                last_price_twap.safe_add(price_twap_price_band)?
            } else {
                last_price_twap.safe_sub(price_twap_price_band)?
            }
        } else {
            new_price
        };

    Ok(capped_update_price)
}