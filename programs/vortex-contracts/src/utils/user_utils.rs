use serum_dex::error::DexError;

use crate::{errors::VortexDexResult, spot_market::SpotBalanceType, user::{OrderStatus, User}, validate};

pub fn validate_user_is_idle(user: &User, slot: u64, accelerated: bool) -> VortexDexResult {
    let slots_since_last_active = slot.saturating_sub(user.last_active_slot);

    let slots_before_idle = if accelerated {
        9000_u64 // 60 * 60 / .4 (~1 hour)
    } else {
        1512000_u64 // 60 * 60 * 24 * 7 / .4 (~1 week)
    };

    validate!(
        slots_since_last_active >= slots_before_idle,
        DexError::UserNotInactive,
        "user only been idle for {} slot",
        slots_since_last_active
    )?;

    validate!(
        !user.is_bankrupt(),
        DexError::UserNotInactive,
        "user bankrupt"
    )?;

    validate!(
        !user.is_being_liquidated(),
        DexError::UserNotInactive,
        "user being liquidated"
    )?;


    // Perp wont be enabled for vortex-v1

    // for perp_position in &user.perp_positions {
    //     validate!(
    //         perp_position.is_available(),
    //         DexError::UserNotInactive,
    //         "user has perp position for market {}",
    //         perp_position.market_index
    //     )?;
    // }

    for spot_position in &user.spot_positions {
        validate!(
            spot_position.balance_type != SpotBalanceType::Borrow
                || spot_position.scaled_balance == 0,
            DexError::UserNotInactive,
            "user has borrow for market {}",
            spot_position.market_index
        )?;

        validate!(
            spot_position.open_orders == 0,
            DexError::UserNotInactive,
            "user has open order for market {}",
            spot_position.market_index
        )?;
    }

    for order in &user.orders {
        validate!(
            order.status == OrderStatus::Init,
            DexError::UserNotInactive,
            "user has an open order"
        )?;
    }

    Ok(())
}
