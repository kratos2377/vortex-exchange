use crate::{errors::{DexError, VortexDexResult}, state::user::SpotPosition, validate};

use super::constants::MAX_OPEN_ORDERS;





pub fn validate_spot_position(position: &SpotPosition) -> VortexDexResult {
    validate!(
        position.open_orders <= MAX_OPEN_ORDERS,
        DexError::InvalidSpotPositionDetected,
        "user spot={} position.open_orders={} is greater than MAX_OPEN_ORDERS={}",
        position.market_index,
        position.open_orders,
        MAX_OPEN_ORDERS,
    )?;

    validate!(
        position.open_bids >= 0,
        DexError::InvalidSpotPositionDetected,
        "user spot={} position.open_bids={} is less than 0",
        position.market_index,
        position.open_bids,
    )?;

    validate!(
        position.open_asks <= 0,
        DexError::InvalidSpotPositionDetected,
        "user spot={} position.open_asks={} is greater than 0",
        position.market_index,
        position.open_asks,
    )?;

    Ok(())
}