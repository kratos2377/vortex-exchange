use crate::errors::DexError;
use crate::math::fees::*;
use crate::state::*;
use crate::utils::token_utils::transfer_from_pool_vault_to_user;
use anchor_lang::prelude::*;
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{Mint, TokenAccount};
use config::{AmmConfig, AMM_CONFIG_SEED};
use pool::PoolState;
use std::ops::DerefMut;


pub fn handle_create_amm_config(
    ctx: Context<CreateAmmConfig>,
    index: u16,
    trade_fee_rate: u64,
    protocol_fee_rate: u64,
    fund_fee_rate: u64,
    create_pool_fee: u64,
) -> Result<()> {
    let amm_config = ctx.accounts.amm_config.deref_mut();
    amm_config.protocol_owner = ctx.accounts.owner.key();
    amm_config.bump = ctx.bumps.amm_config;
    amm_config.disable_create_pool = false;
    amm_config.index = index;
    amm_config.trade_fee_rate = trade_fee_rate;
    amm_config.protocol_fee_rate = protocol_fee_rate;
    amm_config.fund_fee_rate = fund_fee_rate;
    amm_config.create_pool_fee = create_pool_fee;
    amm_config.fund_owner = ctx.accounts.owner.key();
    Ok(())
}

pub fn handle_update_amm_config(ctx: Context<UpdateAmmConfig>, param: u8, value: u64) -> Result<()> {
    let amm_config = &mut ctx.accounts.amm_config;
    let match_param = Some(param);
    match match_param {
        Some(0) => update_trade_fee_rate(amm_config, value),
        Some(1) => update_protocol_fee_rate(amm_config, value),
        Some(2) => update_fund_fee_rate(amm_config, value),
        Some(3) => {
            let new_procotol_owner = *ctx.remaining_accounts.iter().next().unwrap().key;
            set_new_protocol_owner(amm_config, new_procotol_owner)?;
        }
        Some(4) => {
            let new_fund_owner = *ctx.remaining_accounts.iter().next().unwrap().key;
            set_new_fund_owner(amm_config, new_fund_owner)?;
        }
        Some(5) => amm_config.create_pool_fee = value,
        Some(6) => amm_config.disable_create_pool = if value == 0 { false } else { true },
        _ => return err!(DexError::InvalidInput),
    }

    Ok(())
}

fn update_protocol_fee_rate(amm_config: &mut Account<AmmConfig>, protocol_fee_rate: u64) {
    assert!(protocol_fee_rate <= FEE_RATE_DENOMINATOR_VALUE);
    assert!(protocol_fee_rate + amm_config.fund_fee_rate <= FEE_RATE_DENOMINATOR_VALUE);
    amm_config.protocol_fee_rate = protocol_fee_rate;
}

fn update_trade_fee_rate(amm_config: &mut Account<AmmConfig>, trade_fee_rate: u64) {
    assert!(trade_fee_rate < FEE_RATE_DENOMINATOR_VALUE);
    amm_config.trade_fee_rate = trade_fee_rate;
}

fn update_fund_fee_rate(amm_config: &mut Account<AmmConfig>, fund_fee_rate: u64) {
    assert!(fund_fee_rate <= FEE_RATE_DENOMINATOR_VALUE);
    assert!(fund_fee_rate + amm_config.protocol_fee_rate <= FEE_RATE_DENOMINATOR_VALUE);
    amm_config.fund_fee_rate = fund_fee_rate;
}

fn set_new_protocol_owner(amm_config: &mut Account<AmmConfig>, new_owner: Pubkey) -> Result<()> {
    require_keys_neq!(new_owner, Pubkey::default());
    #[cfg(feature = "enable-log")]
    msg!(
        "amm_config, old_protocol_owner:{}, new_owner:{}",
        amm_config.protocol_owner.to_string(),
        new_owner.key().to_string()
    );
    amm_config.protocol_owner = new_owner;
    Ok(())
}

fn set_new_fund_owner(amm_config: &mut Account<AmmConfig>, new_fund_owner: Pubkey) -> Result<()> {
    require_keys_neq!(new_fund_owner, Pubkey::default());
    #[cfg(feature = "enable-log")]
    msg!(
        "amm_config, old_fund_owner:{}, new_fund_owner:{}",
        amm_config.fund_owner.to_string(),
        new_fund_owner.key().to_string()
    );
    amm_config.fund_owner = new_fund_owner;
    Ok(())
}

pub fn update_pool_status(ctx: Context<UpdatePoolStatus>, status: u8) -> Result<()> {
    require_gte!(255, status);
    let mut pool_state = ctx.accounts.pool_state.load_mut()?;
    pool_state.set_status(status);
    pool_state.recent_epoch = Clock::get()?.epoch;
    Ok(())
}

pub fn collect_protocol_fee(
    ctx: Context<CollectProtocolFee>,
    amount_0_requested: u64,
    amount_1_requested: u64,
) -> Result<()> {
    let amount_0: u64;
    let amount_1: u64;
    let auth_bump: u8;
    {
        let mut pool_state = ctx.accounts.pool_state.load_mut()?;

        amount_0 = amount_0_requested.min(pool_state.protocol_fees_token_0);
        amount_1 = amount_1_requested.min(pool_state.protocol_fees_token_1);

        pool_state.protocol_fees_token_0 = pool_state
            .protocol_fees_token_0
            .checked_sub(amount_0)
            .unwrap();
        pool_state.protocol_fees_token_1 = pool_state
            .protocol_fees_token_1
            .checked_sub(amount_1)
            .unwrap();

        auth_bump = pool_state.auth_bump;
        pool_state.recent_epoch = Clock::get()?.epoch;
    }
    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_0_vault.to_account_info(),
        ctx.accounts.recipient_token_0_account.to_account_info(),
        ctx.accounts.vault_0_mint.to_account_info(),
        if ctx.accounts.vault_0_mint.to_account_info().owner == ctx.accounts.token_program.key {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_program_2022.to_account_info()
        },
        amount_0,
        ctx.accounts.vault_0_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[auth_bump]]],
    )?;

    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_1_vault.to_account_info(),
        ctx.accounts.recipient_token_1_account.to_account_info(),
        ctx.accounts.vault_1_mint.to_account_info(),
        if ctx.accounts.vault_1_mint.to_account_info().owner == ctx.accounts.token_program.key {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_program_2022.to_account_info()
        },
        amount_1,
        ctx.accounts.vault_1_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[auth_bump]]],
    )?;

    Ok(())
}


pub fn collect_fund_fee(
    ctx: Context<CollectFundFee>,
    amount_0_requested: u64,
    amount_1_requested: u64,
) -> Result<()> {
    let amount_0: u64;
    let amount_1: u64;
    let auth_bump: u8;
    {
        let mut pool_state = ctx.accounts.pool_state.load_mut()?;
        amount_0 = amount_0_requested.min(pool_state.fund_fees_token_0);
        amount_1 = amount_1_requested.min(pool_state.fund_fees_token_1);

        pool_state.fund_fees_token_0 = pool_state.fund_fees_token_0.checked_sub(amount_0).unwrap();
        pool_state.fund_fees_token_1 = pool_state.fund_fees_token_1.checked_sub(amount_1).unwrap();
        auth_bump = pool_state.auth_bump;
        pool_state.recent_epoch = Clock::get()?.epoch;
    }
    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_0_vault.to_account_info(),
        ctx.accounts.recipient_token_0_account.to_account_info(),
        ctx.accounts.vault_0_mint.to_account_info(),
        if ctx.accounts.vault_0_mint.to_account_info().owner == ctx.accounts.token_program.key {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_program_2022.to_account_info()
        },
        amount_0,
        ctx.accounts.vault_0_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[auth_bump]]],
    )?;

    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_1_vault.to_account_info(),
        ctx.accounts.recipient_token_1_account.to_account_info(),
        ctx.accounts.vault_1_mint.to_account_info(),
        if ctx.accounts.vault_1_mint.to_account_info().owner == ctx.accounts.token_program.key {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_program_2022.to_account_info()
        },
        amount_1,
        ctx.accounts.vault_1_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[auth_bump]]],
    )?;

    Ok(())
}


// State
#[derive(Accounts)]
#[instruction(index: u16)]
pub struct CreateAmmConfig<'info> {
    /// Address to be set as protocol owner.
    #[account(
        mut,
        address = crate::admin::id() @ DexError::InvalidOwner
    )]
    pub owner: Signer<'info>,

    /// Initialize config state account to store protocol owner address and fee rates.
    #[account(
        init,
        seeds = [
            AMM_CONFIG_SEED.as_bytes(),
            &index.to_be_bytes()
        ],
        bump,
        payer = owner,
        space = AmmConfig::LEN
    )]
    pub amm_config: Account<'info, AmmConfig>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct UpdateAmmConfig<'info> {
    /// The amm config owner or admin
    #[account(address = crate::admin::id() @ DexError::InvalidOwner)]
    pub owner: Signer<'info>,

    /// Amm config account to be changed
    #[account(mut)]
    pub amm_config: Account<'info, AmmConfig>,
}

#[derive(Accounts)]
pub struct UpdatePoolStatus<'info> {
    #[account(
        address = crate::admin::id()
    )]
    pub authority: Signer<'info>,

    #[account(mut)]
    pub pool_state: AccountLoader<'info, PoolState>,
}




#[derive(Accounts)]
pub struct CollectProtocolFee<'info> {
    /// Only admin or owner can collect fee now
    #[account(constraint = (owner.key() == amm_config.protocol_owner || owner.key() == crate::admin::id()) @ DexError::InvalidOwner)]
    pub owner: Signer<'info>,

    /// CHECK: pool vault and lp mint authority
    #[account(
        seeds = [
            crate::AUTH_SEED.as_bytes(),
        ],
        bump,
    )]
    pub authority: UncheckedAccount<'info>,

    /// Pool state stores accumulated protocol fee amount
    #[account(mut)]
    pub pool_state: AccountLoader<'info, PoolState>,

    /// Amm config account stores owner
    #[account(address = pool_state.load()?.amm_config)]
    pub amm_config: Account<'info, AmmConfig>,

    /// The address that holds pool tokens for token_0
    #[account(
        mut,
        constraint = token_0_vault.key() == pool_state.load()?.token_0_vault
    )]
    pub token_0_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The address that holds pool tokens for token_1
    #[account(
        mut,
        constraint = token_1_vault.key() == pool_state.load()?.token_1_vault
    )]
    pub token_1_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The mint of token_0 vault
    #[account(
        address = token_0_vault.mint
    )]
    pub vault_0_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The mint of token_1 vault
    #[account(
        address = token_1_vault.mint
    )]
    pub vault_1_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The address that receives the collected token_0 protocol fees
    #[account(mut)]
    pub recipient_token_0_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The address that receives the collected token_1 protocol fees
    #[account(mut)]
    pub recipient_token_1_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The SPL program to perform token transfers
    pub token_program: Program<'info, Token>,

    /// The SPL program 2022 to perform token transfers
    pub token_program_2022: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct CollectFundFee<'info> {
    /// Only admin or fund_owner can collect fee now
    #[account(constraint = (owner.key() == amm_config.fund_owner || owner.key() == crate::admin::id()) @ DexError::InvalidOwner)]
    pub owner: Signer<'info>,

    /// CHECK: pool vault and lp mint authority
    #[account(
        seeds = [
            crate::AUTH_SEED.as_bytes(),
        ],
        bump,
    )]
    pub authority: UncheckedAccount<'info>,

    /// Pool state stores accumulated protocol fee amount
    #[account(mut)]
    pub pool_state: AccountLoader<'info, PoolState>,

    /// Amm config account stores fund_owner
    #[account(address = pool_state.load()?.amm_config)]
    pub amm_config: Account<'info, AmmConfig>,

    /// The address that holds pool tokens for token_0
    #[account(
        mut,
        constraint = token_0_vault.key() == pool_state.load()?.token_0_vault
    )]
    pub token_0_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The address that holds pool tokens for token_1
    #[account(
        mut,
        constraint = token_1_vault.key() == pool_state.load()?.token_1_vault
    )]
    pub token_1_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The mint of token_0 vault
    #[account(
        address = token_0_vault.mint
    )]
    pub vault_0_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The mint of token_1 vault
    #[account(
        address = token_1_vault.mint
    )]
    pub vault_1_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The address that receives the collected token_0 fund fees
    #[account(mut)]
    pub recipient_token_0_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The address that receives the collected token_1 fund fees
    #[account(mut)]
    pub recipient_token_1_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The SPL program to perform token transfers
    pub token_program: Program<'info, Token>,

    /// The SPL program 2022 to perform token transfers
    pub token_program_2022: Program<'info, Token2022>,
}