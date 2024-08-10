
use crate::errors::{DexError, VortexDexResult};
use crate::validate;
use anchor_lang::prelude::{Account, AccountInfo, Pubkey};
use anchor_spl::token::TokenAccount;
use solana_program::msg;
use std::slice::Iter;
use std::iter::Peekable;

pub fn validate_whitelist_token(
    whitelist_token: Account<TokenAccount>,
    whitelist_mint: &Pubkey,
    authority: &Pubkey,
) -> VortexDexResult {
    validate!(
        &whitelist_token.owner == authority,
        DexError::InvalidWhitelistToken,
        "Whitelist token owner ({:?}) does not match authority ({:?})",
        whitelist_token.owner,
        authority
    )?;

    validate!(
        &whitelist_token.mint == whitelist_mint,
        DexError::InvalidWhitelistToken,
        "Token mint ({:?}) does not whitelist mint ({:?})",
        whitelist_token.mint,
        whitelist_mint
    )?;

    validate!(
        whitelist_token.amount > 0,
        DexError::InvalidWhitelistToken,
        "Whitelist token amount must be > 0",
    )?;

    Ok(())
}


pub fn get_whitelist_token<'a>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'a>>>,
) -> VortexDexResult<Account<'a, TokenAccount>> {
    let token_account_info = account_info_iter.peek();
    if token_account_info.is_none() {
        msg!("Could not find whitelist token");
        return Err(DexError::InvalidWhitelistToken);
    }

    let token_account_info = token_account_info.unwrap();
    let whitelist_token: Account<TokenAccount> =
        Account::try_from(token_account_info).map_err(|e| {
            msg!("Unable to deserialize whitelist token");
            msg!("{:?}", e);
            DexError::InvalidWhitelistToken
        })?;

    Ok(whitelist_token)
}