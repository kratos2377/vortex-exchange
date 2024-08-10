use anchor_lang::prelude::*;

use crate::{errors::DexError, load, load_mut, state::{dex_state::DexState, user::User, user_stats::UserStats}, utils::token_utils, validate};




pub fn handle_initialize_user<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, InitializeUserAccount<'info>>,
    sub_account_id: u16,
    name: [u8; 32],
) -> Result<()> {
    let user_key = ctx.accounts.user.key();
    let mut user = ctx
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


    emit!(NewUserRecord {
        ts: now_ts,
        user_authority: ctx.accounts.authority.key(),
        user: user_key,
        sub_account_id,
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