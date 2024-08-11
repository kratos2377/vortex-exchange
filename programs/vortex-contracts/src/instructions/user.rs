use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::extension::confidential_transfer::instruction;
use solana_program::{program::invoke, system_instruction::transfer};

use crate::{casting::Cast, errors::DexError, load, load_mut, safe_increment, state::{dex_state::DexState, events::NewUserAccountRecord, user::User, user_stats::UserStats}, utils::token_utils, validate};




pub fn initialize_new_user_account<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, InitializeUserAccount<'info>>,
    name: [u8; 32],
) -> Result<()> {
    let user_key = ctx.accounts.user.key();
    let mut user   = ctx
        .accounts
        .user
        .load_init()
        .or(Err(DexError::UnableToLoadAccountLoader))?;
    user.authority = ctx.accounts.authority.key();
    user.name = name;
    user.next_order_id = 1;
    user.next_liquidation_id = 1;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();

    let mut user_stats = load_mut!(ctx.accounts.user_stats)?;

    let whitelist_mint = &ctx.accounts.state.whitelist_mint;
    if !whitelist_mint.eq(&Pubkey::default()) {
        token_utils::validate_whitelist_token(
            token_utils::get_whitelist_token(remaining_accounts_iter)?,
            whitelist_mint,
            &ctx.accounts.authority.key(),
        )?;
    }

    let state = &mut ctx.accounts.state;

    let now_ts = Clock::get()?.unix_timestamp;


    emit!(NewUserAccountRecord {
        ts: now_ts,
        user_authority: ctx.accounts.authority.key(),
        user: user_key,
        name
    });

    drop(user);

    let init_fee = state.get_init_user_fee()?;

    if init_fee > 0 {
        let payer_lamports = ctx.accounts.payer.to_account_info().try_lamports()?;
        if payer_lamports < init_fee {
            msg!("payer lamports {} init fee {}", payer_lamports, init_fee);
            return Err(DexError::CantPayUserInitFee.into());
        }

        invoke(
            &transfer(
                &ctx.accounts.payer.key(),
                &ctx.accounts.user.key(),
                init_fee,
            ),
            &[
                ctx.accounts.payer.to_account_info().clone(),
                ctx.accounts.user.to_account_info().clone(),
                ctx.accounts.system_program.to_account_info().clone(),
            ],
        )?;
    }

    Ok(())
}


pub fn handle_initialize_user_stats<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, InitializeUserStats<'info>>
) -> Result<()> {
    let clock = Clock::get()?;

    let mut user_stats = ctx
        .accounts
        .user_stats
        .load_init()
        .or(Err(DexError::UnableToLoadAccountLoader))?;

    *user_stats = UserStats {
        authority: ctx.accounts.authority.key(),
        number_of_sub_accounts: 0,
        last_taker_volume_30d_ts: clock.unix_timestamp,
        last_maker_volume_30d_ts: clock.unix_timestamp,
        last_filler_volume_30d_ts: clock.unix_timestamp,
        last_fuel_if_bonus_update_ts: clock.unix_timestamp.cast()?,
        ..UserStats::default()
    };

    let state = &mut ctx.accounts.state;
    safe_increment!(state.number_of_authorities, 1);


    Ok(())
}


#[derive(Accounts)]
pub struct InitializeUserAccount<'info>{
    #[account(
        init,
        seeds = [b"user", authority.key.as_ref()],
        space = User::SIZE,
        bump,
        payer = payer
    )]
    pub user: AccountLoader<'info, User>,
    #[account(
        mut,
        has_one = authority
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
    #[account(mut)]
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeUserStats<'info>{
    #[account(
        init,
        seeds = [b"user_stats", authority.key.as_ref()],
        space = UserStats::SIZE,
        bump,
        payer = payer
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
    #[account(mut)]
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
#[instruction(market_index: u16)]
pub struct Deposit<'info> {
    pub state: Box<Account<'info,DexState>>,
    #[account(
        mut,
        constrain = can_sign_for_user(&user, &authority)
    )]
}