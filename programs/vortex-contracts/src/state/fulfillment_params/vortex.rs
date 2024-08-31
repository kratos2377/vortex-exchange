use crate::errors::{VortexDexResult, DexError};
use anchor_lang::prelude::*;
use anchor_spl::token_interface::TokenAccount;
use crate::state::events::OrderActionExplanation;

use crate::state::position::PositionDirection;
use crate::state::spot_fulfillment_params::{ExternalSpotFill, SpotFulfillmentParams};
use crate::state::spot_market::SpotMarket;
use crate::utils::spot_market_utils::validate_spot_market_vault_amount;
use crate::validate;


use anchor_lang::prelude::InterfaceAccount;
use arrayref::array_ref;

use solana_program::account_info::AccountInfo;
use solana_program::msg;
use std::cell::Ref;

pub struct MatchFulfillmentParams<'a> {
    pub base_market_vault: Box<InterfaceAccount<'a, TokenAccount>>,
    pub quote_market_vault: Box<InterfaceAccount<'a, TokenAccount>>,
}

impl<'a> MatchFulfillmentParams<'a> {
    pub fn new<'b, 'c: 'a>(
        account_info_iter: &'b mut std::iter::Peekable<std::slice::Iter<'c, AccountInfo<'a>>>,
        base_market: &SpotMarket,
        quote_market: &SpotMarket,
    ) -> VortexDexResult<MatchFulfillmentParams<'a>> {
        let account_info_vec = account_info_iter.collect::<Vec<_>>();
        let account_infos = array_ref![account_info_vec, 0, 2];
        let [base_market_vault, quote_market_vault] = account_infos;

        validate!(
            &base_market.vault == base_market_vault.key,
            DexError::InvalidFulfillmentConfig
        )?;

        validate!(
            &quote_market.vault == quote_market_vault.key,
            DexError::InvalidFulfillmentConfig
        )?;

        let base_market_vault: Box<InterfaceAccount<TokenAccount>> =
            Box::new(InterfaceAccount::try_from(base_market_vault).map_err(|e| {
                msg!("{:?}", e);
                DexError::InvalidFulfillmentConfig
            })?);
        let quote_market_vault: Box<InterfaceAccount<TokenAccount>> =
            Box::new(InterfaceAccount::try_from(quote_market_vault).map_err(|e| {
                msg!("{:?}", e);
                DexError::InvalidFulfillmentConfig
            })?);

        Ok(MatchFulfillmentParams {
            base_market_vault,
            quote_market_vault,
        })
    }
}

impl<'a> SpotFulfillmentParams for MatchFulfillmentParams<'a> {
    fn is_external(&self) -> bool {
        false
    }

    fn get_best_bid_and_ask(&self) -> VortexDexResult<(Option<u64>, Option<u64>)> {
        Err(DexError::InvalidSpotFulfillmentParams)
    }

    fn fulfill_order(
        &mut self,
        _taker_direction: PositionDirection,
        _taker_price: u64,
        _taker_base_asset_amount: u64,
        _taker_max_quote_asset_amount: u64,
    ) -> VortexDexResult<ExternalSpotFill> {
        Err(DexError::InvalidSpotFulfillmentParams)
    }

    fn get_order_action_explanation(&self) -> VortexDexResult<OrderActionExplanation> {
        Err(DexError::InvalidSpotFulfillmentParams)
    }

    fn validate_vault_amounts(
        &self,
        base_market: &Ref<SpotMarket>,
        quote_market: &Ref<SpotMarket>,
    ) -> VortexDexResult<()> {
        validate_spot_market_vault_amount(base_market, self.base_market_vault.amount)?;

        validate_spot_market_vault_amount(quote_market, self.quote_market_vault.amount)?;

        Ok(())
    }
}
