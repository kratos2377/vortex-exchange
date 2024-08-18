use anchor_lang::prelude::*;
use anchor_spl::{associated_token::AssociatedToken, token::{Token, TokenAccount}, token_2022::{spl_token_2022::extension::confidential_transfer::instruction, Token2022}, token_interface::TokenInterface};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{program::invoke, system_instruction::transfer, sysvar::instructions};

use crate::{casting::Cast, controllers::{self, orders::ModifyOrderId, spot_balance::update_revenue_pool_balances, spot_position::{charge_withdraw_fee, update_spot_balances_and_cumulative_deposits, update_spot_balances_and_cumulative_deposits_with_limits}}, errors::DexError, get_then_update_id, ids::{jupiter_mainnet_4, jupiter_mainnet_6, marinade_mainnet, serum_program}, instructions::{account::{get_token_mint, load_maps, AccountMaps}, constraints::{can_sign_for_user, is_stats_for_user}}, load, load_mut, print_error, safe_decrement, safe_increment, state::{dex_state::{DexState, ExchangeStatus}, events::{DepositDirection, DepositExplanation, DepositRecord, NewUserAccountRecord, OrderActionExplanation, SwapRecord}, fulfillment_params::{serum::SerumFulfillmentParams, vortex::MatchFulfillmentParams}, operations::SpotOperation, oracle::StrictOraclePrice, order_params::{ModifyOrderParams, OrderParams, PlaceOrderOptions, PostOnlyParam}, position::PositionDirection, spot_fulfillment_params::SpotFulfillmentParams, spot_market::{MarketStatus, SpotBalanceType}, spot_market_map::{get_writable_spot_market_set, get_writable_spot_market_set_from_many, MarketSet}, user::{MarketType, OrderType, User}, user_map::{load_user_maps, UserMap, UserStatsMap}, user_stats::UserStats}, utils::{self, constants::{QUOTE_SPOT_MARKET_INDEX, THIRTEEN_DAY}, liquidation_utils::is_user_being_liquidated, margin_utils::{calculate_max_withdrawable_amount, meets_withdraw_margin_requirement, validate_spot_margin_trading, MarginRequirementType}, spot_market_utils::{self, get_token_value}, swap_utils::{calculate_swap_price, validate_price_bands_for_swap}, token_utils, validation_utils::validate_user_deletion}, validate};
use crate::instructions::constraints::{fill_not_paused, exchange_not_paused , withdraw_not_paused, deposit_not_paused};

use super::{account::get_token_interface, executors::SpotFulfillmentType};



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

    validate!(!user.is_bankrupt(), DexError::UserBankrupt)?;

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
                DexError::ReduceOnlyWithdrawIncreasedRisk
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
        DexError::UserBankrupt,
        "to_user bankrupt"
    )?;
    validate!(
        !from_user.is_bankrupt(),
        DexError::UserBankrupt,
        "from_user bankrupt"
    )?;

    validate!(
        from_user_key != to_user_key,
        DexError::CantTransferBetweenSameUserAccount,
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
                DexError::InvalidSpotPosition,
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


pub fn handle_place_spot_order<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, PlaceOrder>,
    params: OrderParams,
) -> Result<()> {
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        &mut ctx.remaining_accounts.iter().peekable(),
        &MarketSet::new(),
        Clock::get()?.slot,
        None,
    )?;

    if params.immediate_or_cancel {
        msg!("immediate_or_cancel order must be in place_and_make or place_and_take");
        return Err(print_error!(DexError::InvalidOrderIOC)().into());
    }

    let user_key = ctx.accounts.user.key();
    let mut user = load_mut!(ctx.accounts.user)?;

    controllers::orders::place_spot_order(
        &ctx.accounts.state,
        &mut user,
        user_key,
        &spot_market_map,
        &mut oracle_map,
        &Clock::get()?,
        params,
        PlaceOrderOptions::default(),
    )?;

    Ok(())
}

#[access_control(
    fill_not_paused(&ctx.accounts.state)
)]
pub fn handle_place_and_take_spot_order<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, PlaceAndTake<'info>>,
    params: OrderParams,
    fulfillment_type: SpotFulfillmentType,
    _maker_order_id: Option<u32>,
) -> Result<()> {
    let clock = Clock::get()?;
    let market_index = params.market_index;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        remaining_accounts_iter,
        &get_writable_spot_market_set_from_many(vec![QUOTE_SPOT_MARKET_INDEX, market_index]),
        clock.slot,
        None,
    )?;

    if params.post_only != PostOnlyParam::None {
        msg!("post_only cant be used in place_and_take");
        return Err(print_error!(DexError::InvalidOrderPostOnly)().into());
    }

    let (makers_and_referrer, makers_and_referrer_stats) = match fulfillment_type {
        SpotFulfillmentType::Match => load_user_maps(remaining_accounts_iter, true)?,
        _ => (UserMap::empty(), UserStatsMap::empty()),
    };

    let is_immediate_or_cancel = params.immediate_or_cancel;

    let mut fulfillment_params: Box<dyn SpotFulfillmentParams> = match fulfillment_type {
        SpotFulfillmentType::SerumV3 => {
            let base_market = spot_market_map.get_ref(&market_index)?;
            let quote_market = spot_market_map.get_quote_spot_market()?;
            Box::new(SerumFulfillmentParams::new(
                remaining_accounts_iter,
                &ctx.accounts.state,
                &base_market,
                &quote_market,
                clock.unix_timestamp,
            )?)
        }

        SpotFulfillmentType::Match => {
            let base_market = spot_market_map.get_ref(&market_index)?;
            let quote_market = spot_market_map.get_quote_spot_market()?;
            Box::new(MatchFulfillmentParams::new(
                remaining_accounts_iter,
                &base_market,
                &quote_market,
            )?)
        }
    };

    let user_key = ctx.accounts.user.key();
    let mut user = load_mut!(ctx.accounts.user)?;

    controllers::orders::place_spot_order(
        &ctx.accounts.state,
        &mut user,
        user_key,
        &spot_market_map,
        &mut oracle_map,
        &clock,
        params,
        PlaceOrderOptions::default(),
    )?;

    drop(user);

    let user = &mut ctx.accounts.user;
    let order_id = load!(user)?.get_last_order_id();

    controllers::orders::fill_spot_order(
        order_id,
        &ctx.accounts.state,
        user,
        &ctx.accounts.user_stats,
        &spot_market_map,
        &mut oracle_map,
        &user.clone(),
        &ctx.accounts.user_stats.clone(),
        &makers_and_referrer,
        &makers_and_referrer_stats,
        None,
        &clock,
        fulfillment_params.as_mut(),
    )?;

    let order_exists = load!(ctx.accounts.user)?
        .orders
        .iter()
        .any(|order| order.order_id == order_id);

    if is_immediate_or_cancel && order_exists {
        controllers::orders::cancel_order_by_order_id(
            order_id,
            &ctx.accounts.user,
            &spot_market_map,
            &mut oracle_map,
            &clock,
        )?;
    }

    let base_market = spot_market_map.get_ref(&market_index)?;
    let quote_market = spot_market_map.get_quote_spot_market()?;
    fulfillment_params.validate_vault_amounts(&base_market, &quote_market)?;

    Ok(())
}

#[access_control(
    fill_not_paused(&ctx.accounts.state)
)]
pub fn handle_place_and_make_spot_order<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, PlaceAndMake<'info>>,
    params: OrderParams,
    taker_order_id: u32,
    fulfillment_type: SpotFulfillmentType,
) -> Result<()> {
    let clock = &Clock::get()?;
    let state = &ctx.accounts.state;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        remaining_accounts_iter,
        &get_writable_spot_market_set_from_many(vec![QUOTE_SPOT_MARKET_INDEX, params.market_index]),
        Clock::get()?.slot,
        None,
    )?;

   // let (_referrer, _referrer_stats) = get_referrer_and_referrer_stats(remaining_accounts_iter)?;

    if !params.immediate_or_cancel
        || params.post_only == PostOnlyParam::None
        || params.order_type != OrderType::Limit
    {
        msg!("place_and_make must use IOC post only limit order");
        return Err(print_error!(DexError::InvalidOrderIOCPostOnly)().into());
    }

    let market_index = params.market_index;

    let mut fulfillment_params: Box<dyn SpotFulfillmentParams> = match fulfillment_type {
        SpotFulfillmentType::SerumV3 => {
            let base_market = spot_market_map.get_ref(&market_index)?;
            let quote_market = spot_market_map.get_quote_spot_market()?;
            Box::new(SerumFulfillmentParams::new(
                remaining_accounts_iter,
                &ctx.accounts.state,
                &base_market,
                &quote_market,
                clock.unix_timestamp,
            )?)
        }

        SpotFulfillmentType::Match => {
            let base_market = spot_market_map.get_ref(&market_index)?;
            let quote_market = spot_market_map.get_quote_spot_market()?;
            Box::new(MatchFulfillmentParams::new(
                remaining_accounts_iter,
                &base_market,
                &quote_market,
            )?)
        }
    };

    let user_key = ctx.accounts.user.key();
    let mut user = load_mut!(ctx.accounts.user)?;
    let authority = user.authority;

    controllers::orders::place_spot_order(
        state,
        &mut user,
        user_key,
        &spot_market_map,
        &mut oracle_map,
        clock,
        params,
        PlaceOrderOptions::default(),
    )?;

    drop(user);

    let order_id = load!(ctx.accounts.user)?.get_last_order_id();

    let mut makers_and_referrer = UserMap::empty();
    let mut makers_and_referrer_stats = UserStatsMap::empty();
    makers_and_referrer.insert(ctx.accounts.user.key(), ctx.accounts.user.clone())?;
    makers_and_referrer_stats.insert(authority, ctx.accounts.user_stats.clone())?;

    controllers::orders::fill_spot_order(
        taker_order_id,
        state,
        &ctx.accounts.taker,
        &ctx.accounts.taker_stats,
        &spot_market_map,
        &mut oracle_map,
        &ctx.accounts.user.clone(),
        &ctx.accounts.user_stats.clone(),
        &makers_and_referrer,
        &makers_and_referrer_stats,
        Some(order_id),
        clock,
        fulfillment_params.as_mut(),
    )?;

    let order_exists = load!(ctx.accounts.user)?
        .orders
        .iter()
        .any(|order| order.order_id == order_id);

    if order_exists {
        controllers::orders::cancel_order_by_order_id(
            order_id,
            &ctx.accounts.user,
            &spot_market_map,
            &mut oracle_map,
            clock,
        )?;
    }

    let base_market = spot_market_map.get_ref(&market_index)?;
    let quote_market = spot_market_map.get_quote_spot_market()?;
    fulfillment_params.validate_vault_amounts(&base_market, &quote_market)?;

    Ok(())
}


#[access_control(
    exchange_not_paused(&ctx.accounts.state)
)]
pub fn handle_place_orders<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, PlaceOrder>,
    params: Vec<OrderParams>,
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

    validate!(
        params.len() <= 32,
        DexError::DefaultError,
        "max 32 order params"
    )?;

    let user_key = ctx.accounts.user.key();
    let mut user = load_mut!(ctx.accounts.user)?;

    let num_orders = params.len();
    for (i, params) in params.iter().enumerate() {
        validate!(
            !params.immediate_or_cancel,
            DexError::InvalidOrderIOC,
            "immediate_or_cancel order must be in place_and_make or place_and_take"
        )?;

        // only enforce margin on last order and only try to expire on first order
        let options = PlaceOrderOptions {
            enforce_margin_check: i == num_orders - 1,
            try_expire_orders: i == 0,
            risk_increasing: false,
            explanation: OrderActionExplanation::None,
        };

            controllers::orders::place_spot_order(
                &ctx.accounts.state,
                &mut user,
                user_key,
                &spot_market_map,
                &mut oracle_map,
                clock,
                *params,
                options,
            )?;
        
    }

    Ok(())
}


#[access_control(
    fill_not_paused(&ctx.accounts.state)
)]
pub fn handle_begin_swap<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, Swap<'info>>,
    in_market_index: u16,
    out_market_index: u16,
    amount_in: u64,
) -> Result<()> {
    let state = &ctx.accounts.state;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        remaining_accounts_iter,
        &get_writable_spot_market_set_from_many(vec![in_market_index, out_market_index]),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;

    let _token_interface = get_token_interface(remaining_accounts_iter)?;
    let mint = get_token_mint(remaining_accounts_iter)?;

    let mut user = load_mut!(&ctx.accounts.user)?;
    let delegate_is_signer = user.delegate == ctx.accounts.authority.key();

    validate!(!user.is_bankrupt(), DexError::UserBankrupt)?;

    utils::liquidation_utils::validate_user_not_being_liquidated(
        &mut user,
        &spot_market_map,
        &mut oracle_map,
        ctx.accounts.state.liquidation_margin_buffer_ratio,
    )?;

    let mut in_spot_market = spot_market_map.get_ref_mut(&in_market_index)?;

    validate!(
        in_spot_market.fills_enabled(),
        DexError::MarketFillOrderPaused,
        "Swaps disabled for {}",
        in_market_index
    )?;

    validate!(
        in_spot_market.flash_loan_initial_token_amount == 0
            && in_spot_market.flash_loan_amount == 0,
        DexError::InvalidSwap,
        "begin_swap ended in invalid state"
    )?;

    let in_oracle_data = oracle_map.get_price_data(&in_spot_market.oracle)?;
    controllers::spot_balance::update_spot_market_cumulative_interest(
        &mut in_spot_market,
        Some(in_oracle_data),
        now,
    )?;

    let mut out_spot_market = spot_market_map.get_ref_mut(&out_market_index)?;

    validate!(
        out_spot_market.fills_enabled(),
        DexError::MarketFillOrderPaused,
        "Swaps disabled for {}",
        out_market_index
    )?;

    validate!(
        out_spot_market.flash_loan_initial_token_amount == 0
            && out_spot_market.flash_loan_amount == 0,
        DexError::InvalidSwap,
        "begin_swap ended in invalid state"
    )?;

    let out_oracle_data = oracle_map.get_price_data(&out_spot_market.oracle)?;
    controllers::spot_balance::update_spot_market_cumulative_interest(
        &mut out_spot_market,
        Some(out_oracle_data),
        now,
    )?;

    validate!(
        in_market_index != out_market_index,
        DexError::InvalidSwap,
        "in and out market the same"
    )?;

    validate!(
        amount_in != 0,
        DexError::InvalidSwap,
        "amount_out cannot be zero"
    )?;

    let in_vault = &ctx.accounts.in_spot_market_vault;
    let in_token_account = &ctx.accounts.in_token_account;

    in_spot_market.flash_loan_amount = amount_in;
    in_spot_market.flash_loan_initial_token_amount = in_token_account.amount;

    let out_token_account = &ctx.accounts.out_token_account;

    out_spot_market.flash_loan_initial_token_amount = out_token_account.amount;

    controllers::token::send_from_program_vault(
        &ctx.accounts.token_program,
        in_vault,
        &ctx.accounts.in_token_account,
        &ctx.accounts.drift_signer,
        state.signer_nonce,
        amount_in,
        &mint,
    )?;

    let ixs = ctx.accounts.instructions.as_ref();
    let current_index = instructions::load_current_index_checked(ixs)? as usize;

    let current_ix = instructions::load_instruction_at_checked(current_index, ixs)?;
    validate!(
        current_ix.program_id == *ctx.program_id,
        DexError::InvalidSwap,
        "SwapBegin must be a top-level instruction (cant be cpi)"
    )?;

    // The only other drift program allowed is SwapEnd
    let mut index = current_index + 1;
    let mut found_end = false;
    loop {
        let ix = match instructions::load_instruction_at_checked(index, ixs) {
            Ok(ix) => ix,
            Err(ProgramError::InvalidArgument) => break,
            Err(e) => return Err(e.into()),
        };

        // Check that the drift program key is not used
        if ix.program_id == crate::id() {
            // must be the last ix -- this could possibly be relaxed
            validate!(
                !found_end,
                DexError::InvalidSwap,
                "the transaction must not contain a Drift instruction after FlashLoanEnd"
            )?;
            found_end = true;

            // must be the SwapEnd instruction
            let discriminator = crate::instruction::EndSwap::discriminator();
            validate!(
                ix.data[0..8] == discriminator,
                DexError::InvalidSwap,
                "last drift ix must be end of swap"
            )?;

            validate!(
                ctx.accounts.user.key() == ix.accounts[1].pubkey,
                DexError::InvalidSwap,
                "the user passed to SwapBegin and End must match"
            )?;

            validate!(
                ctx.accounts.authority.key() == ix.accounts[3].pubkey,
                DexError::InvalidSwap,
                "the authority passed to SwapBegin and End must match"
            )?;

            validate!(
                ctx.accounts.out_spot_market_vault.key() == ix.accounts[4].pubkey,
                DexError::InvalidSwap,
                "the out_spot_market_vault passed to SwapBegin and End must match"
            )?;

            validate!(
                ctx.accounts.in_spot_market_vault.key() == ix.accounts[5].pubkey,
                DexError::InvalidSwap,
                "the in_spot_market_vault passed to SwapBegin and End must match"
            )?;

            validate!(
                ctx.accounts.out_token_account.key() == ix.accounts[6].pubkey,
                DexError::InvalidSwap,
                "the out_token_account passed to SwapBegin and End must match"
            )?;

            validate!(
                ctx.accounts.in_token_account.key() == ix.accounts[7].pubkey,
                DexError::InvalidSwap,
                "the in_token_account passed to SwapBegin and End must match"
            )?;

            validate!(
                ctx.remaining_accounts.len() == ix.accounts.len() - 11,
                DexError::InvalidSwap,
                "begin and end ix must have the same number of accounts"
            )?;

            for i in 11..ix.accounts.len() {
                validate!(
                    *ctx.remaining_accounts[i - 11].key == ix.accounts[i].pubkey,
                    DexError::InvalidSwap,
                    "begin and end ix must have the same accounts. {}th account mismatch. begin: {}, end: {}",
                    i,
                    ctx.remaining_accounts[i - 11].key,
                    ix.accounts[i].pubkey
                )?;
            }
        } else {
            let mut whitelisted_programs = vec![
                serum_program::id(),
                jupiter_mainnet_4::ID,
                jupiter_mainnet_6::ID,
            ];
            if !delegate_is_signer {
                whitelisted_programs.push(Token::id());
                whitelisted_programs.push(Token2022::id());
                whitelisted_programs.push(marinade_mainnet::ID);
            }
            validate!(
                whitelisted_programs.contains(&ix.program_id),
                DexError::InvalidSwap,
                "only allowed to pass in ixs to token, openbook, and Jupiter v3/v4/v6 programs"
            )?;

            for meta in ix.accounts.iter() {
                validate!(
                    meta.pubkey != crate::id(),
                    DexError::InvalidSwap,
                    "instructions between begin and end must not be drift instructions"
                )?;
            }
        }

        index += 1;
    }

    validate!(
        found_end,
        DexError::InvalidSwap,
        "found no SwapEnd instruction in transaction"
    )?;

    Ok(())
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum SwapReduceOnly {
    In,
    Out,
}

#[access_control(
    fill_not_paused(&ctx.accounts.state)
)]
pub fn handle_end_swap<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, Swap<'info>>,
    in_market_index: u16,
    out_market_index: u16,
    limit_price: Option<u64>,
    reduce_only: Option<SwapReduceOnly>,
) -> Result<()> {
    let state = &ctx.accounts.state;
    let clock = Clock::get()?;
    let slot = clock.slot;
    let now = clock.unix_timestamp;

    let remaining_accounts = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
    } = load_maps(
        remaining_accounts,
        &get_writable_spot_market_set_from_many(vec![in_market_index, out_market_index]),
        clock.slot,
        Some(state.oracle_guard_rails),
    )?;
    let out_token_program = get_token_interface(remaining_accounts)?;

    let in_mint = get_token_mint(remaining_accounts)?;
    let out_mint = get_token_mint(remaining_accounts)?;

    let user_key = ctx.accounts.user.key();
    let mut user = load_mut!(&ctx.accounts.user)?;

    let mut user_stats = load_mut!(&ctx.accounts.user_stats)?;

    let exchange_status = state.get_exchange_status()?;

    validate!(
        !exchange_status.contains(ExchangeStatus::DepositPaused | ExchangeStatus::WithdrawPaused),
        DexError::ExchangePaused
    )?;

    let mut in_spot_market = spot_market_map.get_ref_mut(&in_market_index)?;

    validate!(
        !in_spot_market.is_operation_paused(SpotOperation::Withdraw),
        DexError::MarketFillOrderPaused,
        "withdraw from market {} paused",
        in_market_index
    )?;

    validate!(
        in_spot_market.flash_loan_amount != 0,
        DexError::InvalidSwap,
        "the in_spot_market must have a flash loan amount set"
    )?;

    let in_oracle_data = oracle_map.get_price_data(&in_spot_market.oracle)?;
    let in_oracle_price = in_oracle_data.price;

    let mut out_spot_market = spot_market_map.get_ref_mut(&out_market_index)?;

    validate!(
        !out_spot_market.is_operation_paused(SpotOperation::Deposit),
        DexError::MarketFillOrderPaused,
        "deposit to market {} paused",
        out_market_index
    )?;

    let out_oracle_data = oracle_map.get_price_data(&out_spot_market.oracle)?;
    let out_oracle_price = out_oracle_data.price;

    let in_vault = &mut ctx.accounts.in_spot_market_vault;
    let in_token_account = &mut ctx.accounts.in_token_account;

    let mut amount_in = in_spot_market.flash_loan_amount;
    if in_token_account.amount > in_spot_market.flash_loan_initial_token_amount {
        let residual = in_token_account
            .amount
            .safe_sub(in_spot_market.flash_loan_initial_token_amount)?;

        controllers::token::receive(
            &ctx.accounts.token_program,
            in_token_account,
            in_vault,
            &ctx.accounts.authority,
            residual,
            &in_mint,
        )?;
        in_token_account.reload()?;
        in_vault.reload()?;

        amount_in = amount_in.safe_sub(residual)?;
    }

    let in_token_amount_before = user
        .force_get_spot_position_mut(in_market_index)?
        .get_signed_token_amount(&in_spot_market)?;

    // checks deposit/borrow limits
    update_spot_balances_and_cumulative_deposits_with_limits(
        amount_in.cast()?,
        &SpotBalanceType::Borrow,
        &mut in_spot_market,
        &mut user,
    )?;

    let in_token_amount_after = user
        .force_get_spot_position_mut(in_market_index)?
        .get_signed_token_amount(&in_spot_market)?;

    let in_position_is_reduced =
        in_token_amount_before > 0 && in_token_amount_before.unsigned_abs() >= amount_in.cast()?;

    if !in_position_is_reduced {
        validate!(
            !in_spot_market.is_reduce_only(),
            DexError::SpotMarketReduceOnly,
            "in spot market is reduce only but token amount before ({}) < amount in ({})",
            in_token_amount_before,
            amount_in
        )?;

        validate!(
            reduce_only != Some(SwapReduceOnly::In),
            DexError::InvalidSwap,
            "reduce only violated. In position before ({}) < amount in ({})",
            in_token_amount_before,
            amount_in
        )?;

        validate!(
            user.is_margin_trading_enabled,
            DexError::MarginTradingDisabled,
            "swap lead to increase in liability for in market {}",
            in_market_index
        )?;

        validate!(
            !user.is_reduce_only(),
            DexError::UserReduceOnly,
            "swap lead to increase in liability for in market {}",
            in_market_index
        )?;
    }

    utils::spot_market_utils::validate_spot_market_vault_amount(&in_spot_market, in_vault.amount)?;

    in_spot_market.flash_loan_initial_token_amount = 0;
    in_spot_market.flash_loan_amount = 0;

    let out_vault = &mut ctx.accounts.out_spot_market_vault;
    let out_token_account = &mut ctx.accounts.out_token_account;

    let mut amount_out = 0_u64;
    if out_token_account.amount > out_spot_market.flash_loan_initial_token_amount {
        amount_out = out_token_account
            .amount
            .safe_sub(out_spot_market.flash_loan_initial_token_amount)?;

        if let Some(token_interface) = out_token_program {
            controllers::token::receive(
                &token_interface,
                out_token_account,
                out_vault,
                &ctx.accounts.authority,
                amount_out,
                &out_mint,
            )?;
        } else {
            controllers::token::receive(
                &ctx.accounts.token_program,
                out_token_account,
                out_vault,
                &ctx.accounts.authority,
                amount_out,
                &out_mint,
            )?;
        }

        out_vault.reload()?;
    }

    if let Some(limit_price) = limit_price {
        let swap_price = calculate_swap_price(
            amount_out.cast()?,
            amount_in.cast()?,
            out_spot_market.decimals,
            in_spot_market.decimals,
        )?;

        validate!(
            swap_price >= limit_price.cast()?,
            DexError::SwapLimitPriceBreached,
            "swap_price ({}) < limit price ({})",
            swap_price,
            limit_price
        )?;
    }

    let fee = 0_u64; // no fee
    let amount_out_after_fee = amount_out.safe_sub(fee)?;

    out_spot_market.total_swap_fee = out_spot_market.total_swap_fee.saturating_add(fee);

    let fee_value = get_token_value(fee.cast()?, out_spot_market.decimals, out_oracle_price)?;

    // update fees
    user.update_cumulative_spot_fees(-fee_value.cast()?)?;
    user_stats.increment_total_fees(fee_value.cast()?)?;

    if fee != 0 {
        // update taker volume
        let amount_out_value = get_token_value(
            amount_out.cast()?,
            out_spot_market.decimals,
            out_oracle_price,
        )?;
        user_stats.update_taker_volume_30d(
            out_spot_market.fuel_boost_taker,
            amount_out_value.cast()?,
            now,
        )?;
    }

    validate!(
        amount_out != 0,
        DexError::InvalidSwap,
        "amount_out must be greater than 0"
    )?;

    let out_token_amount_before = user
        .force_get_spot_position_mut(out_market_index)?
        .get_signed_token_amount(&out_spot_market)?;

    update_spot_balances_and_cumulative_deposits(
        amount_out_after_fee.cast()?,
        &SpotBalanceType::Deposit,
        &mut out_spot_market,
        user.force_get_spot_position_mut(out_market_index)?,
        false,
        Some(amount_out.cast()?),
    )?;

    let out_token_amount_after = user
        .force_get_spot_position_mut(out_market_index)?
        .get_signed_token_amount(&out_spot_market)?;

    // update fees
    update_revenue_pool_balances(fee.cast()?, &SpotBalanceType::Deposit, &mut out_spot_market)?;

    let out_position_is_reduced = out_token_amount_before < 0
        && out_token_amount_before.unsigned_abs() >= amount_out_after_fee.cast()?;

    if !out_position_is_reduced {
        validate!(
            !out_spot_market.is_reduce_only(),
            DexError::SpotMarketReduceOnly,
            "out spot market is reduce only but token amount before ({}) < amount out ({})",
            out_token_amount_before,
            amount_out
        )?;

        validate!(
            reduce_only != Some(SwapReduceOnly::Out),
            DexError::InvalidSwap,
            "reduce only violated. Out position before ({}) < amount out ({})",
            out_token_amount_before,
            amount_out
        )?;

        validate!(
            !user.is_reduce_only(),
            DexError::UserReduceOnly,
            "swap lead to increase in deposit for in market {}, can only pay off borrow",
            out_market_index
        )?;
    }

    utils::spot_market_utils::validate_spot_market_vault_amount(&out_spot_market, out_vault.amount)?;

    out_spot_market.flash_loan_initial_token_amount = 0;
    out_spot_market.flash_loan_amount = 0;

    out_spot_market.validate_max_token_deposits_and_borrows(false)?;

    let in_strict_price = StrictOraclePrice::new(
        in_oracle_price,
        in_spot_market
            .historical_oracle_data
            .last_oracle_price_twap_5min,
        true,
    );

    let out_strict_price = StrictOraclePrice::new(
        out_oracle_price,
        out_spot_market
            .historical_oracle_data
            .last_oracle_price_twap_5min,
        true,
    );

    let margin_type = utils::swap_utils::select_margin_type_for_swap(
        &in_spot_market,
        &out_spot_market,
        &in_strict_price,
        &out_strict_price,
        in_token_amount_before,
        out_token_amount_before,
        in_token_amount_after,
        out_token_amount_after,
        MarginRequirementType::Initial,
    )?;

    drop(out_spot_market);
    drop(in_spot_market);

    meets_withdraw_margin_requirement(
        &user,
        &spot_market_map,
        &mut oracle_map,
        margin_type,
    )?;

    user.update_last_active_slot(slot);

    let swap_record = SwapRecord {
        ts: now,
        amount_in,
        amount_out,
        out_market_index,
        in_market_index,
        in_oracle_price,
        out_oracle_price,
        user: user_key,
        fee,
    };
    emit!(swap_record);

    let out_spot_market = spot_market_map.get_ref_mut(&out_market_index)?;

    validate!(
        out_spot_market.flash_loan_initial_token_amount == 0
            && out_spot_market.flash_loan_amount == 0,
        DexError::InvalidSwap,
        "end_swap ended in invalid state"
    )?;

    let in_spot_market = spot_market_map.get_ref_mut(&in_market_index)?;

    validate!(
        in_spot_market.flash_loan_initial_token_amount == 0
            && in_spot_market.flash_loan_amount == 0,
        DexError::InvalidSwap,
        "end_swap ended in invalid state"
    )?;

    validate_price_bands_for_swap(
        &in_spot_market,
        &out_spot_market,
        amount_in,
        amount_out,
        in_oracle_price,
        out_oracle_price,
        state
            .oracle_guard_rails
            .max_oracle_twap_5min_percent_divergence(),
    )?;

    Ok(())
}


pub fn handle_update_user_name(
    ctx: Context<UpdateUser>,
    _sub_account_id: u16,
    name: [u8; 32],
) -> Result<()> {
    let mut user = load_mut!(ctx.accounts.user)?;
    user.name = name;
    Ok(())
}

pub fn handle_update_user_custom_margin_ratio(
    ctx: Context<UpdateUser>,
    _sub_account_id: u16,
    margin_ratio: u32,
) -> Result<()> {
    let mut user = load_mut!(ctx.accounts.user)?;
    user.max_margin_ratio = margin_ratio;
    Ok(())
}

pub fn handle_update_user_margin_trading_enabled<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, UpdateUser<'info>>,
    _sub_account_id: u16,
    margin_trading_enabled: bool,
) -> Result<()> {
    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let AccountMaps {
        spot_market_map,
        mut oracle_map,
        ..
    } = load_maps(
        remaining_accounts_iter,
        &MarketSet::new(),
        Clock::get()?.slot,
        None,
    )?;

    let mut user = load_mut!(ctx.accounts.user)?;
    user.is_margin_trading_enabled = margin_trading_enabled;

    validate_spot_margin_trading(&user, &spot_market_map, &mut oracle_map)
        .map_err(|_| DexError::MarginOrdersOpen)?;

    Ok(())
}

pub fn handle_update_user_delegate(
    ctx: Context<UpdateUser>,
    _sub_account_id: u16,
    delegate: Pubkey,
) -> Result<()> {
    let mut user = load_mut!(ctx.accounts.user)?;
    user.delegate = delegate;
    Ok(())
}

pub fn handle_update_user_reduce_only(
    ctx: Context<UpdateUser>,
    _sub_account_id: u16,
    reduce_only: bool,
) -> Result<()> {
    let mut user = load_mut!(ctx.accounts.user)?;

    validate!(!user.is_being_liquidated(), DexError::LiquidationsOngoing)?;

    user.update_reduce_only_status(reduce_only)?;
    Ok(())
}

pub fn handle_update_user_advanced_lp(
    ctx: Context<UpdateUser>,
    _sub_account_id: u16,
    advanced_lp: bool,
) -> Result<()> {
    let mut user = load_mut!(ctx.accounts.user)?;

    validate!(!user.is_being_liquidated(), DexError::LiquidationsOngoing)?;

    user.update_advanced_lp_status(advanced_lp)?;
    Ok(())
}

pub fn handle_delete_user(ctx: Context<DeleteUser>) -> Result<()> {
    let user = &load!(ctx.accounts.user)?;
    let user_stats = &mut load_mut!(ctx.accounts.user_stats)?;

    validate_user_deletion(
        user,
        user_stats,
        &ctx.accounts.state,
        Clock::get()?.unix_timestamp,
    )?;

    safe_decrement!(user_stats.number_of_sub_accounts, 1);

    let state = &mut ctx.accounts.state;
    safe_decrement!(state.number_of_sub_accounts, 1);

    Ok(())
}

pub fn handle_reclaim_rent(ctx: Context<ReclaimRent>) -> Result<()> {
    let user_size = ctx.accounts.user.to_account_info().data_len();
    let minimum_lamports = ctx.accounts.rent.minimum_balance(user_size);
    let current_lamports = ctx.accounts.user.to_account_info().try_lamports()?;
    let reclaim_amount = current_lamports.saturating_sub(minimum_lamports);

    validate!(
        reclaim_amount > 0,
        DexError::CantReclaimRent,
        "user account has no excess lamports to reclaim"
    )?;

    **ctx
        .accounts
        .user
        .to_account_info()
        .try_borrow_mut_lamports()? = minimum_lamports;

    **ctx
        .accounts
        .authority
        .to_account_info()
        .try_borrow_mut_lamports()? += reclaim_amount;

    let user_stats = &mut load!(ctx.accounts.user_stats)?;

    // Skip age check if is no max sub accounts
    let max_sub_accounts = ctx.accounts.state.max_number_of_sub_accounts();
    let estimated_user_stats_age = user_stats.get_age_ts(Clock::get()?.unix_timestamp);
    validate!(
        max_sub_accounts == 0 || estimated_user_stats_age >= THIRTEEN_DAY,
        DexError::CantReclaimRent,
        "user stats too young to reclaim rent. age ={} minimum = {}",
        estimated_user_stats_age,
        THIRTEEN_DAY
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


#[derive(Accounts)]
pub struct PlaceOrder<'info> {
    pub state: Box<Account<'info, DexState>>,
    #[account(
        mut,
        constraint = can_sign_for_user(&user, &authority)?
    )]
    pub user: AccountLoader<'info, User>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct PlaceAndTake<'info> {
    pub state: Box<Account<'info, DexState>>,
    #[account(
        mut,
        constraint = can_sign_for_user(&user, &authority)?
    )]
    pub user: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&user, &user_stats)?
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct PlaceAndMake<'info> {
    pub state: Box<Account<'info, DexState>>,
    #[account(
        mut,
        constraint = can_sign_for_user(&user, &authority)?
    )]
    pub user: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&user, &user_stats)?
    )]
    pub user_stats: AccountLoader<'info, UserStats>,
    #[account(mut)]
    pub taker: AccountLoader<'info, User>,
    #[account(
        mut,
        constraint = is_stats_for_user(&taker, &taker_stats)?
    )]
    pub taker_stats: AccountLoader<'info, UserStats>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(in_market_index: u16, out_market_index: u16, )]
pub struct Swap<'info> {
    pub state: Box<Account<'info, DexState>>,
    #[account(
        mut,
        constraint = can_sign_for_user(&user, &authority)?
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
        seeds = [b"spot_market_vault".as_ref(), out_market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub out_spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [b"spot_market_vault".as_ref(), in_market_index.to_le_bytes().as_ref()],
        bump,
    )]
    pub in_spot_market_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = &out_spot_market_vault.mint.eq(&out_token_account.mint),
        token::authority = authority
    )]
    pub out_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = &in_spot_market_vault.mint.eq(&in_token_account.mint),
        token::authority = authority
    )]
    pub in_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
    #[account(
        constraint = state.signer.eq(&drift_signer.key())
    )]
    /// CHECK: forced drift_signer
    pub drift_signer: AccountInfo<'info>,
    /// Instructions Sysvar for instruction introspection
    /// CHECK: fixed instructions sysvar account
    #[account(address = instructions::ID)]
    pub instructions: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(
    sub_account_id: u16,
)]
pub struct UpdateUser<'info> {
    #[account(
        mut,
        seeds = [b"user", authority.key.as_ref(), sub_account_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub user: AccountLoader<'info, User>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct DeleteUser<'info> {
    #[account(
        mut,
        has_one = authority,
        close = authority
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
}

#[derive(Accounts)]
pub struct ReclaimRent<'info> {
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
    pub state: Box<Account<'info, DexState>>,
    pub authority: Signer<'info>,
    pub rent: Sysvar<'info, Rent>,
}