
use anchor_lang::prelude::*;
use anchor_spl::{token::TokenAccount, token_interface::{Mint, TokenInterface}};
use arrayref::array_ref;
use std::{cell::RefMut, iter::Peekable};

use crate::{errors::{DexError, VortexDexResult}, state::{dex_state::OracleGuardRails, load_ref::load_ref_mut, oracle::{OracleSource, PrelaunchOracle}, oracle_map::OracleMap, spot_market_map::{MarketSet, SpotMarketMap}, user::User, user_stats::UserStats}, validate};
use std::slice::Iter;
pub struct AccountMaps<'a> {
    pub spot_market_map: SpotMarketMap<'a>,
    pub oracle_map: OracleMap<'a>,
}

pub fn load_maps<'a, 'b>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'a>>>,
    writable_spot_markets: &'b MarketSet,
    slot: u64,
    oracle_guard_rails: Option<OracleGuardRails>,
) -> VortexDexResult<AccountMaps<'a>> {
    let oracle_map = OracleMap::load(account_info_iter, slot, oracle_guard_rails)?;
    let spot_market_map = SpotMarketMap::load(writable_spot_markets, account_info_iter)?;
 

    Ok(AccountMaps {
        spot_market_map,
        oracle_map,
    })
}

pub fn get_maker_and_maker_stats<'a>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'a>>>,
) -> VortexDexResult<(AccountLoader<'a, User>, AccountLoader<'a, UserStats>)> {
    let maker_account_info =
        next_account_info(account_info_iter).or(Err(DexError::MakerNotFound))?;

    validate!(
        maker_account_info.is_writable,
        DexError::MakerMustBeWritable
    )?;

    let maker: AccountLoader<User> =
        AccountLoader::try_from(maker_account_info).or(Err(DexError::CouldNotDeserializeMaker))?;

    let maker_stats_account_info =
        next_account_info(account_info_iter).or(Err(DexError::MakerStatsNotFound))?;

    validate!(
        maker_stats_account_info.is_writable,
        DexError::MakerStatsMustBeWritable
    )?;

    let maker_stats: AccountLoader<UserStats> =
        AccountLoader::try_from(maker_stats_account_info)
            .or(Err(DexError::CouldNotDeserializeMakerStats))?;

    Ok((maker, maker_stats))
}

#[allow(clippy::type_complexity)]
pub fn get_referrer_and_referrer_stats<'a>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'a>>>,
) -> VortexDexResult<(
    Option<AccountLoader<'a, User>>,
    Option<AccountLoader<'a, UserStats>>,
)> {
    let referrer_account_info = account_info_iter.peek();

    if referrer_account_info.is_none() {
        return Ok((None, None));
    }

    let referrer_account_info = referrer_account_info.safe_unwrap()?;
    let data = referrer_account_info.try_borrow_data().map_err(|e| {
        msg!("{:?}", e);
        DexError::CouldNotDeserializeReferrer
    })?;

    if data.len() < User::SIZE {
        return Ok((None, None));
    }

    let user_discriminator: [u8; 8] = User::discriminator();
    let account_discriminator = array_ref![data, 0, 8];
    if account_discriminator != &user_discriminator {
        return Ok((None, None));
    }

    let referrer_account_info = next_account_info(account_info_iter).safe_unwrap()?;

    validate!(
        referrer_account_info.is_writable,
        DexError::ReferrerMustBeWritable
    )?;

    let referrer: AccountLoader<User> = AccountLoader::try_from(referrer_account_info)
        .or(Err(DexError::CouldNotDeserializeReferrer))?;

    let referrer_stats_account_info = account_info_iter.peek();
    if referrer_stats_account_info.is_none() {
        return Ok((None, None));
    }

    let referrer_stats_account_info = referrer_stats_account_info.safe_unwrap()?;
    let data = referrer_stats_account_info.try_borrow_data().map_err(|e| {
        msg!("{:?}", e);
        DexError::CouldNotDeserializeReferrerStats
    })?;

    if data.len() < UserStats::SIZE {
        return Ok((None, None));
    }

    let user_stats_discriminator: [u8; 8] = UserStats::discriminator();
    let account_discriminator = array_ref![data, 0, 8];
    if account_discriminator != &user_stats_discriminator {
        return Ok((None, None));
    }

    let referrer_stats_account_info = next_account_info(account_info_iter).safe_unwrap()?;

    validate!(
        referrer_stats_account_info.is_writable,
        DexError::ReferrerMustBeWritable
    )?;

    let referrer_stats: AccountLoader<UserStats> =
        AccountLoader::try_from(referrer_stats_account_info)
            .or(Err(DexError::CouldNotDeserializeReferrerStats))?;

    Ok((Some(referrer), Some(referrer_stats)))
}

pub fn get_whitelist_token<'a>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'a>>>,
) -> VortexDexResult<Account<'a, TokenAccount>> {
    let token_account_info = account_info_iter.peek();
    if token_account_info.is_none() {
        msg!("Could not find whitelist token");
        return Err(DexError::InvalidWhitelistToken);
    }

    let token_account_info = token_account_info.safe_unwrap()?;
    let whitelist_token: Account<TokenAccount> =
        Account::try_from(token_account_info).map_err(|e| {
            msg!("Unable to deserialize whitelist token");
            msg!("{:?}", e);
            DexError::InvalidWhitelistToken
        })?;

    Ok(whitelist_token)
}

pub fn get_token_interface<'a>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'a>>>,
) -> VortexDexResult<Option<Interface<'a, TokenInterface>>> {
    let token_interface_account_info = account_info_iter.peek();
    if token_interface_account_info.is_none() {
        return Ok(None);
    }

    let token_interface_account_info = account_info_iter.next().safe_unwrap()?;
    let token_interface: Interface<TokenInterface> =
        Interface::try_from(token_interface_account_info).map_err(|e| {
            msg!("Unable to deserialize token interface");
            msg!("{:?}", e);
            DexError::DefaultError
        })?;

    Ok(Some(token_interface))
}

pub fn get_token_mint<'a>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'a>>>,
) -> VortexDexResult<Option<InterfaceAccount<'a, Mint>>> {
    let mint_account_info = account_info_iter.peek();
    if mint_account_info.is_none() {
        return Ok(None);
    }

    let mint_account_info = account_info_iter.next().safe_unwrap()?;

    match InterfaceAccount::try_from(mint_account_info) {
        Ok(mint) => Ok(Some(mint)),
        Err(_) => Ok(None),
    }
}
