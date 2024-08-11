use anchor_lang::prelude::*;

use crate::{errors::DexError, state::{dex_state::{DexState, ExchangeStatus}, spot_market::{MarketStatus, SpotMarket}, user::User, user_stats::UserStats}, validate};

pub fn can_sign_for_user(user: &AccountLoader<User>, signer: &Signer) -> anchor_lang::Result<bool> {
    user.load().map(|user| {
        user.authority.eq(signer.key)
            || (user.delegate.eq(signer.key) && !user.delegate.eq(&Pubkey::default()))
    })
}

pub fn is_stats_for_user(
    user: &AccountLoader<User>,
    user_stats: &AccountLoader<UserStats>,
) -> anchor_lang::Result<bool> {
    let user = user.load()?;
    let user_stats = user_stats.load()?;
    Ok(user_stats.authority.eq(&user.authority))
}

pub fn spot_market_valid(market: &AccountLoader<SpotMarket>) -> anchor_lang::Result<()> {
    if market.load()?.status == MarketStatus::Delisted {
        return Err(DexError::MarketDelisted.into());
    }
    Ok(())
}

pub fn valid_oracle_for_spot_market(
    oracle: &AccountInfo,
    market: &AccountLoader<SpotMarket>,
) -> anchor_lang::Result<()> {
    validate!(
        market.load()?.oracle.eq(oracle.key),
        DexError::InvalidOracle,
        "not valid_oracle_for_spot_market"
    )?;
    Ok(())
}


pub fn liq_not_paused(state: &Account<DexState>) -> anchor_lang::Result<()> {
    if state
        .get_exchange_status()?
        .contains(ExchangeStatus::LiqPaused)
    {
        return Err(DexError::ExchangePaused.into());
    }
    Ok(())
}

pub fn funding_not_paused(state: &Account<DexState>) -> anchor_lang::Result<()> {
    if state.funding_paused()? {
        return Err(DexError::ExchangePaused.into());
    }
    Ok(())
}

pub fn amm_not_paused(state: &Account<DexState>) -> anchor_lang::Result<()> {
    if state.amm_paused()? {
        return Err(DexError::ExchangePaused.into());
    }
    Ok(())
}

pub fn fill_not_paused(state: &Account<DexState>) -> anchor_lang::Result<()> {
    if state
        .get_exchange_status()?
        .contains(ExchangeStatus::FillPaused)
    {
        return Err(DexError::ExchangePaused.into());
    }
    Ok(())
}

pub fn deposit_not_paused(state: &Account<DexState>) -> anchor_lang::Result<()> {
    if state
        .get_exchange_status()?
        .contains(ExchangeStatus::DepositPaused)
    {
        return Err(DexError::ExchangePaused.into());
    }
    Ok(())
}

pub fn withdraw_not_paused(state: &Account<DexState>) -> anchor_lang::Result<()> {
    if state
        .get_exchange_status()?
        .contains(ExchangeStatus::WithdrawPaused)
    {
        return Err(DexError::ExchangePaused.into());
    }
    Ok(())
}

pub fn settle_pnl_not_paused(state: &Account<DexState>) -> anchor_lang::Result<()> {
    if state
        .get_exchange_status()?
        .contains(ExchangeStatus::SettlePnlPaused)
    {
        return Err(DexError::ExchangePaused.into());
    }
    Ok(())
}

pub fn exchange_not_paused(state: &Account<DexState>) -> anchor_lang::Result<()> {
    if state.get_exchange_status()?.is_all() {
        return Err(DexError::ExchangePaused.into());
    }
    Ok(())
}
