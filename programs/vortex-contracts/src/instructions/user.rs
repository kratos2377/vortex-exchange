use crate::math::calculator::CurveCalculator;
use crate::math::calculator::RoundDirection;
use crate::math::calculator::TradeDirection;
use crate::errors::DexError;
use crate::state::*;
use crate::utils::token_utils::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program;
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};
use config::AmmConfig;
use events::LpChangeEvent;
use events::SwapEvent;
use oracle::ObservationState;
use pool::PoolState;
use pool::PoolStatusBitIndex;



#[derive(Accounts)]
pub struct Swap<'info> {
    /// The user performing the swap
    pub payer: Signer<'info>,

    /// CHECK: pool vault and lp mint authority
    #[account(
        seeds = [
            crate::AUTH_SEED.as_bytes(),
        ],
        bump,
    )]
    pub authority: UncheckedAccount<'info>,

    /// The factory state to read protocol fees
    #[account(address = pool_state.load()?.amm_config)]
    pub amm_config: Box<Account<'info, AmmConfig>>,

    /// The program account of the pool in which the swap will be performed
    #[account(mut)]
    pub pool_state: AccountLoader<'info, PoolState>,

    /// The user token account for input token
    #[account(mut)]
    pub input_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The user token account for output token
    #[account(mut)]
    pub output_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The vault token account for input token
    #[account(
        mut,
        constraint = input_vault.key() == pool_state.load()?.token_0_vault || input_vault.key() == pool_state.load()?.token_1_vault
    )]
    pub input_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The vault token account for output token
    #[account(
        mut,
        constraint = output_vault.key() == pool_state.load()?.token_0_vault || output_vault.key() == pool_state.load()?.token_1_vault
    )]
    pub output_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// SPL program for input token transfers
    pub input_token_program: Interface<'info, TokenInterface>,

    /// SPL program for output token transfers
    pub output_token_program: Interface<'info, TokenInterface>,

    /// The mint of input token
    #[account(
        address = input_vault.mint
    )]
    pub input_token_mint: Box<InterfaceAccount<'info, Mint>>,

    /// The mint of output token
    #[account(
        address = output_vault.mint
    )]
    pub output_token_mint: Box<InterfaceAccount<'info, Mint>>,
    /// The program account for the most recent oracle observation
    #[account(mut, address = pool_state.load()?.observation_key)]
    pub observation_state: AccountLoader<'info, ObservationState>,
}

pub fn swap_base_input(ctx: Context<Swap>, amount_in: u64, minimum_amount_out: u64) -> Result<()> {
    let block_timestamp = solana_program::clock::Clock::get()?.unix_timestamp as u64;
    let pool_id = ctx.accounts.pool_state.key();
    let pool_state = &mut ctx.accounts.pool_state.load_mut()?;
    if !pool_state.get_status_by_bit(PoolStatusBitIndex::Swap)
        || block_timestamp < pool_state.open_time
    {
        return err!(DexError::NotApproved);
    }

    let transfer_fee =
        get_transfer_fee(&ctx.accounts.input_token_mint.to_account_info(), amount_in)?;
    // Take transfer fees into account for actual amount transferred in
    let actual_amount_in = amount_in.saturating_sub(transfer_fee);
    require_gt!(actual_amount_in, 0);

    // Calculate the trade amounts and the price before swap
    let (
        trade_direction,
        total_input_token_amount,
        total_output_token_amount,
        token_0_price_x64,
        token_1_price_x64,
    ) = if ctx.accounts.input_vault.key() == pool_state.token_0_vault
        && ctx.accounts.output_vault.key() == pool_state.token_1_vault
    {
        let (total_input_token_amount, total_output_token_amount) = pool_state
            .vault_amount_without_fee(
                ctx.accounts.input_vault.amount,
                ctx.accounts.output_vault.amount,
            );
        let (token_0_price_x64, token_1_price_x64) = pool_state.token_price_x32(
            ctx.accounts.input_vault.amount,
            ctx.accounts.output_vault.amount,
        );

        (
            TradeDirection::ZeroForOne,
            total_input_token_amount,
            total_output_token_amount,
            token_0_price_x64,
            token_1_price_x64,
        )
    } else if ctx.accounts.input_vault.key() == pool_state.token_1_vault
        && ctx.accounts.output_vault.key() == pool_state.token_0_vault
    {
        let (total_output_token_amount, total_input_token_amount) = pool_state
            .vault_amount_without_fee(
                ctx.accounts.output_vault.amount,
                ctx.accounts.input_vault.amount,
            );
        let (token_0_price_x64, token_1_price_x64) = pool_state.token_price_x32(
            ctx.accounts.output_vault.amount,
            ctx.accounts.input_vault.amount,
        );

        (
            TradeDirection::OneForZero,
            total_input_token_amount,
            total_output_token_amount,
            token_0_price_x64,
            token_1_price_x64,
        )
    } else {
        return err!(DexError::InvalidVault);
    };
    let constant_before = u128::from(total_input_token_amount)
        .checked_mul(u128::from(total_output_token_amount))
        .unwrap();

    let result = CurveCalculator::swap_base_input(
        u128::from(actual_amount_in),
        u128::from(total_input_token_amount),
        u128::from(total_output_token_amount),
        ctx.accounts.amm_config.trade_fee_rate,
        ctx.accounts.amm_config.protocol_fee_rate,
        ctx.accounts.amm_config.fund_fee_rate,
    )
    .ok_or(DexError::ZeroTradingTokens)?;

    let constant_after = u128::from(
        result
            .new_swap_source_amount
            .checked_sub(result.trade_fee)
            .unwrap(),
    )
    .checked_mul(u128::from(result.new_swap_destination_amount))
    .unwrap();
    #[cfg(feature = "enable-log")]
    msg!(
        "source_amount_swapped:{}, destination_amount_swapped:{}, trade_fee:{}, constant_before:{},constant_after:{}",
        result.source_amount_swapped,
        result.destination_amount_swapped,
        result.trade_fee,
        constant_before,
        constant_after
    );
    require_eq!(
        u64::try_from(result.source_amount_swapped).unwrap(),
        actual_amount_in
    );
    let (input_transfer_amount, input_transfer_fee) = (amount_in, transfer_fee);
    let (output_transfer_amount, output_transfer_fee) = {
        let amount_out = u64::try_from(result.destination_amount_swapped).unwrap();
        let transfer_fee = get_transfer_fee(
            &ctx.accounts.output_token_mint.to_account_info(),
            amount_out,
        )?;
        let amount_received = amount_out.checked_sub(transfer_fee).unwrap();
        require_gt!(amount_received, 0);
        require_gte!(
            amount_received,
            minimum_amount_out,
            DexError::ExceededSlippage
        );
        (amount_out, transfer_fee)
    };

    let protocol_fee = u64::try_from(result.protocol_fee).unwrap();
    let fund_fee = u64::try_from(result.fund_fee).unwrap();

    match trade_direction {
        TradeDirection::ZeroForOne => {
            pool_state.protocol_fees_token_0 = pool_state
                .protocol_fees_token_0
                .checked_add(protocol_fee)
                .unwrap();
            pool_state.fund_fees_token_0 =
                pool_state.fund_fees_token_0.checked_add(fund_fee).unwrap();
        }
        TradeDirection::OneForZero => {
            pool_state.protocol_fees_token_1 = pool_state
                .protocol_fees_token_1
                .checked_add(protocol_fee)
                .unwrap();
            pool_state.fund_fees_token_1 =
                pool_state.fund_fees_token_1.checked_add(fund_fee).unwrap();
        }
    };

    emit!(SwapEvent {
        pool_id,
        input_vault_before: total_input_token_amount,
        output_vault_before: total_output_token_amount,
        input_amount: u64::try_from(result.source_amount_swapped).unwrap(),
        output_amount: u64::try_from(result.destination_amount_swapped).unwrap(),
        input_transfer_fee,
        output_transfer_fee,
        base_input: true
    });
    require_gte!(constant_after, constant_before);

    transfer_from_user_to_pool_vault(
        ctx.accounts.payer.to_account_info(),
        ctx.accounts.input_token_account.to_account_info(),
        ctx.accounts.input_vault.to_account_info(),
        ctx.accounts.input_token_mint.to_account_info(),
        ctx.accounts.input_token_program.to_account_info(),
        input_transfer_amount,
        ctx.accounts.input_token_mint.decimals,
    )?;

    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.output_vault.to_account_info(),
        ctx.accounts.output_token_account.to_account_info(),
        ctx.accounts.output_token_mint.to_account_info(),
        ctx.accounts.output_token_program.to_account_info(),
        output_transfer_amount,
        ctx.accounts.output_token_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[pool_state.auth_bump]]],
    )?;

    // update the previous price to the observation
    ctx.accounts.observation_state.load_mut()?.update(
        oracle::block_timestamp(),
        token_0_price_x64,
        token_1_price_x64,
    );
    pool_state.recent_epoch = Clock::get()?.epoch;

    Ok(())
}


pub fn swap_base_output(
    ctx: Context<Swap>,
    max_amount_in: u64,
    amount_out_less_fee: u64,
) -> Result<()> {
    let block_timestamp = solana_program::clock::Clock::get()?.unix_timestamp as u64;
    let pool_id = ctx.accounts.pool_state.key();
    let pool_state = &mut ctx.accounts.pool_state.load_mut()?;
    if !pool_state.get_status_by_bit(PoolStatusBitIndex::Swap)
        || block_timestamp < pool_state.open_time
    {
        return err!(DexError::NotApproved);
    }
    let out_transfer_fee = get_transfer_inverse_fee(
        &ctx.accounts.output_token_mint.to_account_info(),
        amount_out_less_fee,
    )?;
    let actual_amount_out = amount_out_less_fee.checked_add(out_transfer_fee).unwrap();

    // Calculate the trade amounts and the price before swap
    let (
        trade_direction,
        total_input_token_amount,
        total_output_token_amount,
        token_0_price_x64,
        token_1_price_x64,
    ) = if ctx.accounts.input_vault.key() == pool_state.token_0_vault
        && ctx.accounts.output_vault.key() == pool_state.token_1_vault
    {
        let (total_input_token_amount, total_output_token_amount) = pool_state
            .vault_amount_without_fee(
                ctx.accounts.input_vault.amount,
                ctx.accounts.output_vault.amount,
            );
        let (token_0_price_x64, token_1_price_x64) = pool_state.token_price_x32(
            ctx.accounts.input_vault.amount,
            ctx.accounts.output_vault.amount,
        );

        (
            TradeDirection::ZeroForOne,
            total_input_token_amount,
            total_output_token_amount,
            token_0_price_x64,
            token_1_price_x64,
        )
    } else if ctx.accounts.input_vault.key() == pool_state.token_1_vault
        && ctx.accounts.output_vault.key() == pool_state.token_0_vault
    {
        let (total_output_token_amount, total_input_token_amount) = pool_state
            .vault_amount_without_fee(
                ctx.accounts.output_vault.amount,
                ctx.accounts.input_vault.amount,
            );
        let (token_0_price_x64, token_1_price_x64) = pool_state.token_price_x32(
            ctx.accounts.output_vault.amount,
            ctx.accounts.input_vault.amount,
        );

        (
            TradeDirection::OneForZero,
            total_input_token_amount,
            total_output_token_amount,
            token_0_price_x64,
            token_1_price_x64,
        )
    } else {
        return err!(DexError::InvalidVault);
    };
    let constant_before = u128::from(total_input_token_amount)
        .checked_mul(u128::from(total_output_token_amount))
        .unwrap();

    let result = CurveCalculator::swap_base_output(
        u128::from(actual_amount_out),
        u128::from(total_input_token_amount),
        u128::from(total_output_token_amount),
        ctx.accounts.amm_config.trade_fee_rate,
        ctx.accounts.amm_config.protocol_fee_rate,
        ctx.accounts.amm_config.fund_fee_rate,
    )
    .ok_or(DexError::ZeroTradingTokens)?;

    let constant_after = u128::from(
        result
            .new_swap_source_amount
            .checked_sub(result.trade_fee)
            .unwrap(),
    )
    .checked_mul(u128::from(result.new_swap_destination_amount))
    .unwrap();

    #[cfg(feature = "enable-log")]
    msg!(
        "source_amount_swapped:{}, destination_amount_swapped:{}, trade_fee:{}, constant_before:{},constant_after:{}",
        result.source_amount_swapped,
        result.destination_amount_swapped,
        result.trade_fee,
        constant_before,
        constant_after
    );

    // Re-calculate the source amount swapped based on what the curve says
    let (input_transfer_amount, input_transfer_fee) = {
        let source_amount_swapped = u64::try_from(result.source_amount_swapped).unwrap();
        require_gt!(source_amount_swapped, 0);
        let transfer_fee = get_transfer_inverse_fee(
            &ctx.accounts.input_token_mint.to_account_info(),
            source_amount_swapped,
        )?;
        let input_transfer_amount = source_amount_swapped.checked_add(transfer_fee).unwrap();
        require_gte!(
            max_amount_in,
            input_transfer_amount,
            DexError::ExceededSlippage
        );
        (input_transfer_amount, transfer_fee)
    };
    require_eq!(
        u64::try_from(result.destination_amount_swapped).unwrap(),
        actual_amount_out
    );
    let (output_transfer_amount, output_transfer_fee) = (actual_amount_out, out_transfer_fee);

    let protocol_fee = u64::try_from(result.protocol_fee).unwrap();
    let fund_fee = u64::try_from(result.fund_fee).unwrap();

    match trade_direction {
        TradeDirection::ZeroForOne => {
            pool_state.protocol_fees_token_0 = pool_state
                .protocol_fees_token_0
                .checked_add(protocol_fee)
                .unwrap();
            pool_state.fund_fees_token_0 =
                pool_state.fund_fees_token_0.checked_add(fund_fee).unwrap();
        }
        TradeDirection::OneForZero => {
            pool_state.protocol_fees_token_1 = pool_state
                .protocol_fees_token_1
                .checked_add(protocol_fee)
                .unwrap();
            pool_state.fund_fees_token_1 =
                pool_state.fund_fees_token_1.checked_add(fund_fee).unwrap();
        }
    };

    emit!(SwapEvent {
        pool_id,
        input_vault_before: total_input_token_amount,
        output_vault_before: total_output_token_amount,
        input_amount: u64::try_from(result.source_amount_swapped).unwrap(),
        output_amount: u64::try_from(result.destination_amount_swapped).unwrap(),
        input_transfer_fee,
        output_transfer_fee,
        base_input: false
    });
    require_gte!(constant_after, constant_before);

    transfer_from_user_to_pool_vault(
        ctx.accounts.payer.to_account_info(),
        ctx.accounts.input_token_account.to_account_info(),
        ctx.accounts.input_vault.to_account_info(),
        ctx.accounts.input_token_mint.to_account_info(),
        ctx.accounts.input_token_program.to_account_info(),
        input_transfer_amount,
        ctx.accounts.input_token_mint.decimals,
    )?;

    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.output_vault.to_account_info(),
        ctx.accounts.output_token_account.to_account_info(),
        ctx.accounts.output_token_mint.to_account_info(),
        ctx.accounts.output_token_program.to_account_info(),
        output_transfer_amount,
        ctx.accounts.output_token_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[pool_state.auth_bump]]],
    )?;

    // update the previous price to the observation
    ctx.accounts.observation_state.load_mut()?.update(
        oracle::block_timestamp(),
        token_0_price_x64,
        token_1_price_x64,
    );
    pool_state.recent_epoch = Clock::get()?.epoch;

    Ok(())
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    /// Pays to mint the position
    pub owner: Signer<'info>,

    /// CHECK: pool vault and lp mint authority
    #[account(
        seeds = [
            crate::AUTH_SEED.as_bytes(),
        ],
        bump,
    )]
    pub authority: UncheckedAccount<'info>,

    /// Pool state account
    #[account(mut)]
    pub pool_state: AccountLoader<'info, PoolState>,

    /// Owner lp token account
    #[account(
        mut, 
        token::authority = owner
    )]
    pub owner_lp_token: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The token account for receive token_0, 
    #[account(
        mut,
        token::mint = token_0_vault.mint,
    )]
    pub token_0_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The token account for receive token_1
    #[account(
        mut,
        token::mint = token_1_vault.mint,
    )]
    pub token_1_account: Box<InterfaceAccount<'info, TokenAccount>>,

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

    /// token Program
    pub token_program: Program<'info, Token>,

    /// Token program 2022
    pub token_program_2022: Program<'info, Token2022>,

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

    /// Pool lp token mint
    #[account(
        mut,
        address = pool_state.load()?.lp_mint @ DexError::IncorrectLpMint)
    ]
    pub lp_mint: Box<InterfaceAccount<'info, Mint>>,

    /// memo program
    /// CHECK:
    #[account(
        address = spl_memo::id()
    )]
    pub memo_program: UncheckedAccount<'info>,
}

pub fn withdraw(
    ctx: Context<Withdraw>,
    lp_token_amount: u64,
    minimum_token_0_amount: u64,
    minimum_token_1_amount: u64,
) -> Result<()> {
    require_gt!(ctx.accounts.lp_mint.supply, 0);
    let pool_id = ctx.accounts.pool_state.key();
    let pool_state = &mut ctx.accounts.pool_state.load_mut()?;
    if !pool_state.get_status_by_bit(PoolStatusBitIndex::Withdraw) {
        return err!(DexError::NotApproved);
    }
    let (total_token_0_amount, total_token_1_amount) = pool_state.vault_amount_without_fee(
        ctx.accounts.token_0_vault.amount,
        ctx.accounts.token_1_vault.amount,
    );
    let results = CurveCalculator::lp_tokens_to_trading_tokens(
        u128::from(lp_token_amount),
        u128::from(pool_state.lp_supply),
        u128::from(total_token_0_amount),
        u128::from(total_token_1_amount),
        RoundDirection::Floor,
    )
    .ok_or(DexError::ZeroTradingTokens)?;

    let token_0_amount = u64::try_from(results.token_0_amount).unwrap();
    let token_0_amount = std::cmp::min(total_token_0_amount, token_0_amount);
    let (receive_token_0_amount, token_0_transfer_fee) = {
        let transfer_fee = get_transfer_fee(&ctx.accounts.vault_0_mint.to_account_info(), token_0_amount)?;
        (
            token_0_amount.checked_sub(transfer_fee).unwrap(),
            transfer_fee,
        )
    };

    let token_1_amount = u64::try_from(results.token_1_amount).unwrap();
    let token_1_amount = std::cmp::min(total_token_1_amount, token_1_amount);
    let (receive_token_1_amount, token_1_transfer_fee) = {
        let transfer_fee = get_transfer_fee(&ctx.accounts.vault_1_mint.to_account_info(), token_1_amount)?;
        (
            token_1_amount.checked_sub(transfer_fee).unwrap(),
            transfer_fee,
        )
    };

    #[cfg(feature = "enable-log")]
    msg!(
        "results.token_0_amount;{}, results.token_1_amount:{},receive_token_0_amount:{},token_0_transfer_fee:{},
            receive_token_1_amount:{},token_1_transfer_fee:{}",
        results.token_0_amount,
        results.token_1_amount,
        receive_token_0_amount,
        token_0_transfer_fee,
        receive_token_1_amount,
        token_1_transfer_fee
    );
    emit!(LpChangeEvent {
        pool_id,
        lp_amount_before: pool_state.lp_supply,
        token_0_vault_before: total_token_0_amount,
        token_1_vault_before: total_token_1_amount,
        token_0_amount: receive_token_0_amount,
        token_1_amount: receive_token_1_amount,
        token_0_transfer_fee,
        token_1_transfer_fee,
        change_type: 1
    });

    if receive_token_0_amount < minimum_token_0_amount
        || receive_token_1_amount < minimum_token_1_amount
    {
        return Err(DexError::ExceededSlippage.into());
    }

    pool_state.lp_supply = pool_state.lp_supply.checked_sub(lp_token_amount).unwrap();
    token_burn(
        ctx.accounts.owner.to_account_info(),
        ctx.accounts.token_program.to_account_info(),
        ctx.accounts.lp_mint.to_account_info(),
        ctx.accounts.owner_lp_token.to_account_info(),
        lp_token_amount,
        &[&[crate::AUTH_SEED.as_bytes(), &[pool_state.auth_bump]]],
    )?;

    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_0_vault.to_account_info(),
        ctx.accounts.token_0_account.to_account_info(),
        ctx.accounts.vault_0_mint.to_account_info(),
        if ctx.accounts.vault_0_mint.to_account_info().owner == ctx.accounts.token_program.key {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_program_2022.to_account_info()
        },
        token_0_amount,
        ctx.accounts.vault_0_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[pool_state.auth_bump]]],
    )?;

    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_1_vault.to_account_info(),
        ctx.accounts.token_1_account.to_account_info(),
        ctx.accounts.vault_1_mint.to_account_info(),
        if ctx.accounts.vault_1_mint.to_account_info().owner == ctx.accounts.token_program.key {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_program_2022.to_account_info()
        },
        token_1_amount,
        ctx.accounts.vault_1_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[pool_state.auth_bump]]],
    )?;
    pool_state.recent_epoch = Clock::get()?.epoch;

    Ok(())
}


#[derive(Accounts)]
pub struct Deposit<'info> {
    /// Pays to mint the position
    pub owner: Signer<'info>,

    /// CHECK: pool vault and lp mint authority
    #[account(
        seeds = [
            crate::AUTH_SEED.as_bytes(),
        ],
        bump,
    )]
    pub authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub pool_state: AccountLoader<'info, PoolState>,

    /// Owner lp tokan account
    #[account(mut,  token::authority = owner)]
    pub owner_lp_token: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The payer's token account for token_0
    #[account(
        mut,
        token::mint = token_0_vault.mint,
        token::authority = owner
    )]
    pub token_0_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The payer's token account for token_1
    #[account(
        mut,
        token::mint = token_1_vault.mint,
        token::authority = owner
    )]
    pub token_1_account: Box<InterfaceAccount<'info, TokenAccount>>,

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

    /// token Program
    pub token_program: Program<'info, Token>,

    /// Token program 2022
    pub token_program_2022: Program<'info, Token2022>,

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

    /// Lp token mint
    #[account(
        mut,
        address = pool_state.load()?.lp_mint @ DexError::IncorrectLpMint)
    ]
    pub lp_mint: Box<InterfaceAccount<'info, Mint>>,
}

pub fn deposit(
    ctx: Context<Deposit>,
    lp_token_amount: u64,
    maximum_token_0_amount: u64,
    maximum_token_1_amount: u64,
) -> Result<()> {
    let pool_id = ctx.accounts.pool_state.key();
    let pool_state = &mut ctx.accounts.pool_state.load_mut()?;
    if !pool_state.get_status_by_bit(PoolStatusBitIndex::Deposit) {
        return err!(DexError::NotApproved);
    }
    let (total_token_0_amount, total_token_1_amount) = pool_state.vault_amount_without_fee(
        ctx.accounts.token_0_vault.amount,
        ctx.accounts.token_1_vault.amount,
    );
    let results = CurveCalculator::lp_tokens_to_trading_tokens(
        u128::from(lp_token_amount),
        u128::from(pool_state.lp_supply),
        u128::from(total_token_0_amount),
        u128::from(total_token_1_amount),
        RoundDirection::Ceiling,
    )
    .ok_or(DexError::ZeroTradingTokens)?;

    let token_0_amount = u64::try_from(results.token_0_amount).unwrap();
    let (transfer_token_0_amount, transfer_token_0_fee) = {
        let transfer_fee =
            get_transfer_inverse_fee(&ctx.accounts.vault_0_mint.to_account_info(), token_0_amount)?;
        (
            token_0_amount.checked_add(transfer_fee).unwrap(),
            transfer_fee,
        )
    };

    let token_1_amount = u64::try_from(results.token_1_amount).unwrap();
    let (transfer_token_1_amount, transfer_token_1_fee) = {
        let transfer_fee =
            get_transfer_inverse_fee(&ctx.accounts.vault_1_mint.to_account_info(), token_1_amount)?;
        (
            token_1_amount.checked_add(transfer_fee).unwrap(),
            transfer_fee,
        )
    };

    #[cfg(feature = "enable-log")]
    msg!(
        "results.token_0_amount;{}, results.token_1_amount:{},transfer_token_0_amount:{},transfer_token_0_fee:{},
            transfer_token_1_amount:{},transfer_token_1_fee:{}",
        results.token_0_amount,
        results.token_1_amount,
        transfer_token_0_amount,
        transfer_token_0_fee,
        transfer_token_1_amount,
        transfer_token_1_fee
    );

    emit!(LpChangeEvent {
        pool_id,
        lp_amount_before: pool_state.lp_supply,
        token_0_vault_before: total_token_0_amount,
        token_1_vault_before: total_token_1_amount,
        token_0_amount,
        token_1_amount,
        token_0_transfer_fee: transfer_token_0_fee,
        token_1_transfer_fee: transfer_token_1_fee,
        change_type: 0
    });

    if transfer_token_0_amount > maximum_token_0_amount
        || transfer_token_1_amount > maximum_token_1_amount
    {
        return Err(DexError::ExceededSlippage.into());
    }

    transfer_from_user_to_pool_vault(
        ctx.accounts.owner.to_account_info(),
        ctx.accounts.token_0_account.to_account_info(),
        ctx.accounts.token_0_vault.to_account_info(),
        ctx.accounts.vault_0_mint.to_account_info(),
        if ctx.accounts.vault_0_mint.to_account_info().owner == ctx.accounts.token_program.key {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_program_2022.to_account_info()
        },
        transfer_token_0_amount,
        ctx.accounts.vault_0_mint.decimals,
    )?;

    transfer_from_user_to_pool_vault(
        ctx.accounts.owner.to_account_info(),
        ctx.accounts.token_1_account.to_account_info(),
        ctx.accounts.token_1_vault.to_account_info(),
        ctx.accounts.vault_1_mint.to_account_info(),
        if ctx.accounts.vault_1_mint.to_account_info().owner == ctx.accounts.token_program.key {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_program_2022.to_account_info()
        },
        transfer_token_1_amount,
        ctx.accounts.vault_1_mint.decimals,
    )?;

    pool_state.lp_supply = pool_state.lp_supply.checked_add(lp_token_amount).unwrap();

    token_mint_to(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_program.to_account_info(),
        ctx.accounts.lp_mint.to_account_info(),
        ctx.accounts.owner_lp_token.to_account_info(),
        lp_token_amount,
        &[&[crate::AUTH_SEED.as_bytes(), &[pool_state.auth_bump]]],
    )?;
    pool_state.recent_epoch = Clock::get()?.epoch;

    Ok(())
}
