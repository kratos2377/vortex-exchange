use anchor_lang::prelude::*;
use anchor_spl::{token::TokenAccount, token_2022::spl_token_2022::extension::confidential_transfer::instruction, token_interface::TokenInterface};
use solana_program::{program::invoke, system_instruction::transfer};

use crate::{casting::Cast, controllers::{self, orders::ModifyOrderId, spot_position::charge_withdraw_fee}, errors::DexError, get_then_update_id, instructions::{account::{get_token_mint, load_maps, AccountMaps}, constraints::{can_sign_for_user, is_stats_for_user}}, load, load_mut, safe_increment, state::{dex_state::DexState, events::{DepositDirection, DepositExplanation, DepositRecord, NewUserAccountRecord, OrderActionExplanation}, order_params::ModifyOrderParams, position::PositionDirection, spot_market::{MarketStatus, SpotBalanceType}, spot_market_map::{get_writable_spot_market_set, MarketSet}, user::{MarketType, User}, user_stats::UserStats}, utils::{liquidation_utils::is_user_being_liquidated, margin_utils::{calculate_max_withdrawable_amount, validate_spot_margin_trading, MarginRequirementType}, spot_market_utils, token_utils}, validate};
use crate::instructions::constraints::{exchange_not_paused , withdraw_not_paused, deposit_not_paused};



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


#[access_control(
    deposit_not_paused(&ctx.accounts.state)
)]
pub fn handle_deposit<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, Deposit<'info>>,
    market_index: u16,
    amount: u64,
    reduce_only: bool,
) -> Result<()> {
    let user_key = ctx.accounts.user.key();
    let user = &mut load_mut!(ctx.accounts.user)?;

    let state = &ctx.accounts.state;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let slot = clock.slot;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        remaining_accounts_iter,
        &get_writable_spot_market_set(market_index),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let mint = get_token_mint(remaining_accounts_iter)?;

    if amount == 0 {
        return Err(DexError::InsufficientDeposit.into());
    }

    validate!(!user.is_bankrupt(), DexError::UserBankrupt)?;

    let mut spot_market = spot_market_map.get_ref_mut(&market_index)?;
    let oracle_price_data = &oracle_map.get_price_data(&spot_market.oracle)?.clone();

    validate!(
        !matches!(spot_market.status, MarketStatus::Initialized),
        DexError::MarketBeingInitialized,
        "Market is being initialized"
    )?;

    controllers::spot_balance::update_spot_market_cumulative_interest(
        &mut spot_market,
        Some(oracle_price_data),
        now,
    )?;

    let position_index = user.force_get_spot_position_index(spot_market.market_index)?;

    let is_borrow_before = user.spot_positions[position_index].is_borrow();

    let force_reduce_only = spot_market.is_reduce_only();

    // if reduce only, have to compare ix amount to current borrow amount
    let amount = if (force_reduce_only || reduce_only)
        && user.spot_positions[position_index].balance_type == SpotBalanceType::Borrow
    {
        user.spot_positions[position_index]
            .get_token_amount(&spot_market)?
            .cast::<u64>()?
            .min(amount)
    } else {
        amount
    };

    user.increment_total_deposits(
        amount,
        oracle_price_data.price,
        spot_market.get_precision().cast()?,
    )?;

    let total_deposits_after = user.total_deposits;
    let total_withdraws_after = user.total_withdraws;

    let spot_position = &mut user.spot_positions[position_index];
    controllers::spot_position::update_spot_balances_and_cumulative_deposits(
        amount as u128,
        &SpotBalanceType::Deposit,
        &mut spot_market,
        spot_position,
        false,
        None,
    )?;

    let token_amount = spot_position.get_token_amount(&spot_market)?;
    if token_amount == 0 {
        validate!(
            spot_position.scaled_balance == 0,
            DexError::InvalidSpotPosition,
            "deposit left user with invalid position. scaled balance = {} token amount = {}",
            spot_position.scaled_balance,
            token_amount
        )?;
    }

    if spot_position.balance_type == SpotBalanceType::Deposit && spot_position.scaled_balance > 0 {
        validate!(
            matches!(spot_market.status, MarketStatus::Active),
            DexError::MarketActionPaused,
            "spot_market not active",
        )?;
    }

    drop(spot_market);
    if user.is_being_liquidated() {
        // try to update liquidation status if user is was already being liq'd
        let is_being_liquidated = is_user_being_liquidated(
            user,
            &spot_market_map,
            &mut oracle_map,
            state.liquidation_margin_buffer_ratio,
        )?;

        if !is_being_liquidated {
            user.exit_liquidation();
        }
    }

    user.update_last_active_slot(slot);

    let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;

    controllers::token::receive(
        &ctx.accounts.token_program,
        &ctx.accounts.user_token_account,
        &ctx.accounts.spot_market_vault,
        &ctx.accounts.authority,
        amount,
        &mint,
    )?;
    ctx.accounts.spot_market_vault.reload()?;

    let deposit_record_id = get_then_update_id!(spot_market, next_deposit_record_id);
    let oracle_price = oracle_price_data.price;
    let explanation = if is_borrow_before {
        DepositExplanation::RepayBorrow
    } else {
        DepositExplanation::None
    };
    let deposit_record = DepositRecord {
        ts: now,
        deposit_record_id,
        user_authority: user.authority,
        user: user_key,
        direction: DepositDirection::Deposit,
        amount,
        oracle_price,
        market_deposit_balance: spot_market.deposit_balance,
        market_withdraw_balance: spot_market.borrow_balance,
        market_cumulative_deposit_interest: spot_market.cumulative_deposit_interest,
        market_cumulative_borrow_interest: spot_market.cumulative_borrow_interest,
        total_deposits_after,
        total_withdraws_after,
        market_index,
        explanation,
        transfer_user: None,
    };
    emit!(deposit_record);

    spot_market.validate_max_token_deposits_and_borrows(false)?;

    Ok(())
}

#[access_control(
    withdraw_not_paused(&ctx.accounts.state)
)]
pub fn handle_withdraw<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, Withdraw<'info>>,
    market_index: u16,
    amount: u64,
    reduce_only: bool,
) -> anchor_lang::Result<()> {
    let user_key = ctx.accounts.user.key();
    let user = &mut load_mut!(ctx.accounts.user)?;
    let mut user_stats = load_mut!(ctx.accounts.user_stats)?;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let slot = clock.slot;
    let state = &ctx.accounts.state;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        remaining_accounts_iter,
        &get_writable_spot_market_set(market_index),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let mint = get_token_mint(remaining_accounts_iter)?;

    validate!(!user.is_bankrupt(), ErrorCode::UserBankrupt)?;

    let spot_market_is_reduce_only = {
        let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;
        let oracle_price_data = oracle_map.get_price_data(&spot_market.oracle)?;

        controllers::spot_balance::update_spot_market_cumulative_interest(
            spot_market,
            Some(oracle_price_data),
            now,
        )?;

        spot_market.is_reduce_only()
    };

    let amount = {
        let reduce_only = reduce_only || spot_market_is_reduce_only;

        let position_index = user.force_get_spot_position_index(market_index)?;

        let mut amount = if reduce_only {
            validate!(
                user.spot_positions[position_index].balance_type == SpotBalanceType::Deposit,
                ErrorCode::ReduceOnlyWithdrawIncreasedRisk
            )?;

            let max_withdrawable_amount = calculate_max_withdrawable_amount(
                market_index,
                user,
                &spot_market_map,
                &mut oracle_map,
            )?;

            let spot_market = &spot_market_map.get_ref(&market_index)?;
            let existing_deposit_amount = user.spot_positions[position_index]
                .get_token_amount(spot_market)?
                .cast::<u64>()?;

            amount
                .min(max_withdrawable_amount)
                .min(existing_deposit_amount)
        } else {
            amount
        };

        let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;
        let oracle_price_data = oracle_map.get_price_data(&spot_market.oracle)?;

        if user.qualifies_for_withdraw_fee(&user_stats, slot) {
            let fee =
                charge_withdraw_fee(spot_market, oracle_price_data.price, user, &mut user_stats)?;
            amount = amount.safe_sub(fee.cast()?)?;
        }

        user.increment_total_withdraws(
            amount,
            oracle_price_data.price,
            spot_market.get_precision().cast()?,
        )?;

        // prevents withdraw when limits hit
        controllers::spot_position::update_spot_balances_and_cumulative_deposits_with_limits(
            amount as u128,
            &SpotBalanceType::Borrow,
            spot_market,
            user,
        )?;

        amount
    };

    user.meets_withdraw_margin_requirement_and_increment_fuel_bonus(
        &spot_market_map,
        &mut oracle_map,
        MarginRequirementType::Initial,
        market_index,
        amount as u128,
        &mut user_stats,
        now,
    )?;

    validate_spot_margin_trading(user, &spot_market_map, &mut oracle_map)?;

    if user.is_being_liquidated() {
        user.exit_liquidation();
    }

    user.update_last_active_slot(slot);

    let mut spot_market = spot_market_map.get_ref_mut(&market_index)?;
    let oracle_price = oracle_map.get_price_data(&spot_market.oracle)?.price;

    let is_borrow = user
        .get_spot_position(market_index)
        .map_or(false, |pos| pos.is_borrow());
    let deposit_explanation = if is_borrow {
        DepositExplanation::Borrow
    } else {
        DepositExplanation::None
    };

    let deposit_record_id = get_then_update_id!(spot_market, next_deposit_record_id);
    let deposit_record = DepositRecord {
        ts: now,
        deposit_record_id,
        user_authority: user.authority,
        user: user_key,
        direction: DepositDirection::Withdraw,
        oracle_price,
        amount,
        market_index,
        market_deposit_balance: spot_market.deposit_balance,
        market_withdraw_balance: spot_market.borrow_balance,
        market_cumulative_deposit_interest: spot_market.cumulative_deposit_interest,
        market_cumulative_borrow_interest: spot_market.cumulative_borrow_interest,
        total_deposits_after: user.total_deposits,
        total_withdraws_after: user.total_withdraws,
        explanation: deposit_explanation,
        transfer_user: None,
    };
    emit!(deposit_record);

    controllers::token::send_from_program_vault(
        &ctx.accounts.token_program,
        &ctx.accounts.spot_market_vault,
        &ctx.accounts.user_token_account,
        &ctx.accounts.drift_signer,
        state.signer_nonce,
        amount,
        &mint,
    )?;

    // reload the spot market vault balance so it's up-to-date
    ctx.accounts.spot_market_vault.reload()?;
    spot_market_utils::validate_spot_market_vault_amount(
        &spot_market,
        ctx.accounts.spot_market_vault.amount,
    )?;

    spot_market.validate_max_token_deposits_and_borrows(is_borrow)?;

    Ok(())
}

#[access_control(
    deposit_not_paused(&ctx.accounts.state)
    withdraw_not_paused(&ctx.accounts.state)
)]
pub fn handle_transfer_deposit<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, TransferDeposit<'info>>,
    market_index: u16,
    amount: u64,
) -> anchor_lang::Result<()> {
    let authority_key = ctx.accounts.authority.key;
    let to_user_key = ctx.accounts.to_user.key();
    let from_user_key = ctx.accounts.from_user.key();

    let state = &ctx.accounts.state;
    let clock = Clock::get()?;
    let slot = clock.slot;

    let to_user = &mut load_mut!(ctx.accounts.to_user)?;
    let from_user = &mut load_mut!(ctx.accounts.from_user)?;
    let user_stats = &mut load_mut!(ctx.accounts.user_stats)?;

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    validate!(
        !to_user.is_bankrupt(),
        ErrorCode::UserBankrupt,
        "to_user bankrupt"
    )?;
    validate!(
        !from_user.is_bankrupt(),
        ErrorCode::UserBankrupt,
        "from_user bankrupt"
    )?;

    validate!(
        from_user_key != to_user_key,
        ErrorCode::CantTransferBetweenSameUserAccount,
        "cant transfer between the same user account"
    )?;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &get_writable_spot_market_set(market_index),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    {
        let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;
        let oracle_price_data = oracle_map.get_price_data(&spot_market.oracle)?;
        controllers::spot_balance::update_spot_market_cumulative_interest(
            spot_market,
            Some(oracle_price_data),
            clock.unix_timestamp,
        )?;
    }

    let oracle_price = {
        let spot_market = &spot_market_map.get_ref(&market_index)?;
        oracle_map.get_price_data(&spot_market.oracle)?.price
    };

    {
        let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;

        from_user.increment_total_withdraws(
            amount,
            oracle_price,
            spot_market.get_precision().cast()?,
        )?;

        // prevents withdraw when limits hit
        controllers::spot_position::update_spot_balances_and_cumulative_deposits_with_limits(
            amount as u128,
            &SpotBalanceType::Borrow,
            spot_market,
            from_user,
        )?;
    }

    from_user.meets_withdraw_margin_requirement_and_increment_fuel_bonus(
        &spot_market_map,
        &mut oracle_map,
        MarginRequirementType::Initial,
        market_index,
        amount as u128,
        user_stats,
        now,
    )?;

    validate_spot_margin_trading(
        from_user,
        &spot_market_map,
        &mut oracle_map,
    )?;

    if from_user.is_being_liquidated() {
        from_user.exit_liquidation();
    }

    from_user.update_last_active_slot(slot);

    {
        let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;

        let deposit_record_id = get_then_update_id!(spot_market, next_deposit_record_id);
        let deposit_record = DepositRecord {
            ts: clock.unix_timestamp,
            deposit_record_id,
            user_authority: *authority_key,
            user: from_user_key,
            direction: DepositDirection::Withdraw,
            amount,
            oracle_price,
            market_index,
            market_deposit_balance: spot_market.deposit_balance,
            market_withdraw_balance: spot_market.borrow_balance,
            market_cumulative_deposit_interest: spot_market.cumulative_deposit_interest,
            market_cumulative_borrow_interest: spot_market.cumulative_borrow_interest,
            total_deposits_after: from_user.total_deposits,
            total_withdraws_after: from_user.total_withdraws,
            explanation: DepositExplanation::Transfer,
            transfer_user: Some(to_user_key),
        };
        emit!(deposit_record);
    }

    {
        let spot_market = &mut spot_market_map.get_ref_mut(&market_index)?;

        to_user.increment_total_deposits(
            amount,
            oracle_price,
            spot_market.get_precision().cast()?,
        )?;

        let total_deposits_after = to_user.total_deposits;
        let total_withdraws_after = to_user.total_withdraws;

        let to_spot_position = to_user.force_get_spot_position_mut(spot_market.market_index)?;

        controllers::spot_position::update_spot_balances_and_cumulative_deposits(
            amount as u128,
            &SpotBalanceType::Deposit,
            spot_market,
            to_spot_position,
            false,
            None,
        )?;

        let token_amount = to_spot_position.get_token_amount(spot_market)?;
        if token_amount == 0 {
            validate!(
                to_spot_position.scaled_balance == 0,
                ErrorCode::InvalidSpotPosition,
                "deposit left to_user with invalid position. scaled balance = {} token amount = {}",
                to_spot_position.scaled_balance,
                token_amount
            )?;
        }

        let deposit_record_id = get_then_update_id!(spot_market, next_deposit_record_id);
        let deposit_record = DepositRecord {
            ts: clock.unix_timestamp,
            deposit_record_id,
            user_authority: *authority_key,
            user: to_user_key,
            direction: DepositDirection::Deposit,
            amount,
            oracle_price,
            market_index,
            market_deposit_balance: spot_market.deposit_balance,
            market_withdraw_balance: spot_market.borrow_balance,
            market_cumulative_deposit_interest: spot_market.cumulative_deposit_interest,
            market_cumulative_borrow_interest: spot_market.cumulative_borrow_interest,
            total_deposits_after,
            total_withdraws_after,
            explanation: DepositExplanation::Transfer,
            transfer_user: Some(from_user_key),
        };
        emit!(deposit_record);
    }

    to_user.update_last_active_slot(slot);

    let spot_market = spot_market_map.get_ref(&market_index)?;
    spot_market_utils::validate_spot_market_vault_amount(
        &spot_market,
        ctx.accounts.spot_market_vault.amount,
    )?;

    Ok(())
}


#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_cancel_order<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, CancelOrder>,
    order_id: Option<u32>,
) -> Result<()> {
    let clock = &Clock::get()?;
    let state = &ctx.accounts.state;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let order_id = match order_id {
        Some(order_id) => order_id,
        None => load!(ctx.accounts.user)?.get_last_order_id(),
    };

    controllers::orders::cancel_order_by_order_id(
        order_id,
        &ctx.accounts.user,
        &spot_market_map,
        &mut oracle_map,
        clock,
    )?;

    Ok(())
}


#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_cancel_order_by_user_id<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, CancelOrder>,
    user_order_id: u8,
) -> Result<()> {
    let clock = &Clock::get()?;
    let state = &ctx.accounts.state;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    controllers::orders::cancel_order_by_user_order_id(
        user_order_id,
        &ctx.accounts.user,
        &spot_market_map,
        &mut oracle_map,
        clock,
    )?;

    Ok(())
}


#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_cancel_orders_by_ids<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, CancelOrder>,
    order_ids: Vec<u32>,
) -> Result<()> {
    let clock = &Clock::get()?;
    let state = &ctx.accounts.state;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    for order_id in order_ids {
        controllers::orders::cancel_order_by_order_id(
            order_id,
            &ctx.accounts.user,
            &spot_market_map,
            &mut oracle_map,
            clock,
        )?;
    }

    Ok(())
}

#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_cancel_orders<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, CancelOrder<'info>>,
    market_type: Option<MarketType>,
    market_index: Option<u16>,
    direction: Option<PositionDirection>,
) -> Result<()> {
    let clock = &Clock::get()?;
    let state = &ctx.accounts.state;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let user_key = ctx.accounts.user.key();
    let mut user = load_mut!(ctx.accounts.user)?;

    controllers::orders::cancel_orders(
        &mut user,
        &user_key,
        None,
        &spot_market_map,
        &mut oracle_map,
        clock.unix_timestamp,
        clock.slot,
        OrderActionExplanation::None,
        market_type,
        market_index,
        direction,
    )?;

    Ok(())
}

#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_modify_order<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, CancelOrder<'info>>,
    order_id: Option<u32>,
    modify_order_params: ModifyOrderParams,
) -> Result<()> {
    let clock = &Clock::get()?;
    let state = &ctx.accounts.state;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let order_id = match order_id {
        Some(order_id) => order_id,
        None => load!(ctx.accounts.user)?.get_last_order_id(),
    };

    controllers::orders::modify_order(
        ModifyOrderId::OrderId(order_id),
        modify_order_params,
        &ctx.accounts.user,
        state,
        &spot_market_map,
        &mut oracle_map,
        clock,
    )?;

    Ok(())
}

#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_modify_order_by_user_order_id<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, CancelOrder<'info>>,
    user_order_id: u8,
    modify_order_params: ModifyOrderParams,
) -> Result<()> {
    let clock = &Clock::get()?;
    let state = &ctx.accounts.state;

    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    controllers::orders::modify_order(
        ModifyOrderId::UserOrderId(user_order_id),
        modify_order_params,
        &ctx.accounts.user,
        state,
        &spot_market_map,
        &mut oracle_map,
        clock,
    )?;

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
        constraint = can_sign_for_user(&user, &authority)
    )]
    pub user: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&user, &user_stats)?
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"spot_market_vault".as_ref(), market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = &spot_market_vault.mint.eq(&user_token_account.mint),
        token::authority = authority
    )]
    pub user_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
#[instruction(market_index: u16,)]
pub struct Withdraw<'info> {
    pub state: Box<Account<'info, DexState>>,
    #[account(
        mut,
        has_one = authority,
    )]
    pub user: AccountLoader<'info, User>,
    #[account(
        mut,
        has_one = authority
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"spot_market_vault".as_ref(), market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        constraint = state.signer.eq(&drift_signer.key())
    )]
    /// CHECK: forced drift_signer
    pub drift_signer: AccountInfo<'info>,
    #[account(
        mut,
        constraint = &spot_market_vault.mint.eq(&user_token_account.mint)
    )]
    pub user_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
#[instruction(market_index: u16,)]
pub struct TransferDeposit<'info> {
    #[account(
        mut,
        has_one = authority,
    )]
    pub from_user: AccountLoader<'info, User>,
    #[account(
        mut,
        has_one = authority,
    )]
    pub to_user: AccountLoader<'info, User>,
    #[account(
        mut,
        has_one = authority
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
    pub authority: Signer<'info>,
    pub state: Box<Account<'info, DexState>>,
    #[account(
        seeds = [b"spot_market_vault".as_ref(), market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
}

#[derive(Accounts)]
pub struct CancelOrder<'info> {
    pub state: Box<Account<'info, DexState>>,
    #[account(
        mut,
        constraint = can_sign_for_user(&user, &authority)?
    )]
    pub user: AccountLoader<'info, User>,
    pub authority: Signer<'info>,
}
