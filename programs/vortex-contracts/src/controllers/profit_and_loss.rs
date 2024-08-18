use anchor_lang::prelude::*;
use crate::{controllers::{orders::cancel_orders, spot_balance::{update_spot_balances, update_spot_market_cumulative_interest}}, dex_state::DexState, errors::{DexError, VortexDexResult}, events::OrderActionExplanation, oracle_map::OracleMap, profit_and_loss::SettlePnlMode, spot_market::SpotBalanceType, spot_market_map::SpotMarketMap, user::User, utils::{margin_utils::meets_maintenance_margin_requirement, spot_market_utils::get_token_amount}, validate};



pub fn settle_pnl(
    market_index: u16,
    user: &mut User,
    authority: &Pubkey,
    user_key: &Pubkey,
    spot_market_map: &SpotMarketMap,
    oracle_map: &mut OracleMap,
    clock: &Clock,
    state: &DexState,
    meets_margin_requirement: Option<bool>,
    mode: SettlePnlMode,
) -> VortexDexResult {
    validate!(!user.is_bankrupt(), ErrorCode::UserBankrupt)?;
    let now = clock.unix_timestamp;
    {
        let spot_market = &mut spot_market_map.get_quote_spot_market_mut()?;
        update_spot_market_cumulative_interest(spot_market, None, now)?;
    }

 

    let oracle_price = oracle_map.get_price_data(&market.amm.oracle)?.price;

   

    crate::controllers::lp::settle_funding_payment_then_lp(user, user_key, &mut market, now)?;


    
    let spot_market = &mut spot_market_map.get_quote_spot_market_mut()?;
   


    // let pnl_pool_token_amount = get_token_amount(
    //     user.spot_positions[0].scaled_balance
    //     spot_market,
    //     &SpotBalanceType::Deposit
    // )?;

    // let fraction_of_fee_pool_token_amount = get_token_amount(
    //     spot_market,
    //     &SpotBalanceType::Deposit
    // )?
    // .safe_div(5)?;

    // // add a buffer from fee pool for pnl pool balance
    // let pnl_tokens_available: i128 = pnl_pool_token_amount
    //     .safe_add(fraction_of_fee_pool_token_amount)?
    //     .cast()?;


    //     let user_unsettled_pnl: i128 =
    //     user.spot_positions[0].get_claimable_pnl(oracle_price, max_pnl_pool_excess)?;
    
    // let pnl_to_settle_with_user = update_pool_balances(
    //         spot_market,
    //         user.get_quote_spot_position(),
    //         user_unsettled_pnl,
    //         now,
    //     )?;



    // if user_unsettled_pnl == 0 {
    //     let msg = format!("User has no unsettled pnl for market {}", market_index);
    //     return mode.result(ErrorCode::NoUnsettledPnl, market_index, &msg);
    // } else if pnl_to_settle_with_user == 0 {
    //     let msg = format!(
    //         "Pnl Pool cannot currently settle with user for market {}",
    //         market_index
    //     );
    //     return mode.result(ErrorCode::PnlPoolCantSettleUser, market_index, &msg);
    // }

    // let user_must_settle_themself = pnl_to_settle_with_user >= 0
    //     && max_pnl_pool_excess <= 0
    //     && !(pnl_to_settle_with_user > 0 && base_asset_amount == 0 && user.is_being_liquidated())
    //     && !(user.authority.eq(authority) || user.delegate.eq(authority));

    // if user_must_settle_themself {
    //     let msg = format!(
    //         "Market = {} user must settle their own unsettled pnl when its positive and pnl pool not in excess",
    //         market_index
    //     );
    //     return mode.result(
    //         ErrorCode::UserMustSettleTheirOwnPositiveUnsettledPNL,
    //         market_index,
    //         &msg,
    //     );
    // }

    update_spot_balances(
        pnl_to_settle_with_user.unsigned_abs(),
        if pnl_to_settle_with_user > 0 {
            &SpotBalanceType::Deposit
        } else {
            &SpotBalanceType::Borrow
        },
        spot_market,
        user.get_quote_spot_position_mut(),
        false,
    )?;



    update_settled_pnl(user, position_index, pnl_to_settle_with_user.cast()?)?;

    let quote_asset_amount_after = user.perp_positions[position_index].quote_asset_amount;
    let quote_entry_amount = user.perp_positions[position_index].quote_entry_amount;

   

    emit!(SettlePnlRecord {
        ts: now,
        user: *user_key,
        market_index,
        pnl: pnl_to_settle_with_user,
        base_asset_amount,
        quote_asset_amount_after,
        quote_entry_amount,
        settle_price: oracle_price,
        explanation: SettlePnlExplanation::None,
    });

    Ok(())
}