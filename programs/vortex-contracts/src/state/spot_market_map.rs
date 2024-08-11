use std::{cell::{Ref, RefMut}, collections::BTreeMap, iter::Peekable, panic::Location, slice::Iter};

use anchor_lang::prelude::{AccountInfo, AccountLoader};
use arrayref::array_ref;
use solana_program::msg;

use crate::{errors::{DexError, VortexDexResult}, state::constants::QUOTE_SPOT_MARKET_INDEX};

use super::spot_market::SpotMarket;




pub struct SpotMarketMap<'a>(pub BTreeMap<u16, AccountLoader<'a, SpotMarket>>);

impl<'a> SpotMarketMap<'a> {
    #[track_caller]
    #[inline(always)]
    pub fn get_ref(&self, market_index: &u16) -> VortexDexResult<Ref<SpotMarket>> {
        let loader = match self.0.get(market_index) {
            Some(loader) => loader,
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not find spot market {} at {}:{}",
                    market_index,
                    caller.file(),
                    caller.line()
                );
                return Err(DexError::SpotMarketNotFound);
            }
        };

        match loader.load() {
            Ok(spot_market) => Ok(spot_market),
            Err(e) => {
                let caller = Location::caller();
                msg!("{:?}", e);
                msg!(
                    "Could not load spot market {} at {}:{}",
                    market_index,
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToLoadSpotMarketAccount)
            }
        }
    }

    #[track_caller]
    #[inline(always)]
    pub fn get_ref_mut(&self, market_index: &u16) -> VortexDexResult<RefMut<SpotMarket>> {
        let loader = match self.0.get(market_index) {
            Some(loader) => loader,
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not find spot market {} at {}:{}",
                    market_index,
                    caller.file(),
                    caller.line()
                );
                return Err(DexError::SpotMarketNotFound);
            }
        };

        match loader.load_mut() {
            Ok(spot_market) => Ok(spot_market),
            Err(e) => {
                let caller = Location::caller();
                msg!("{:?}", e);
                msg!(
                    "Could not load spot market {} at {}:{}",
                    market_index,
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToLoadSpotMarketAccount)
            }
        }
    }

    #[track_caller]
    #[inline(always)]
    pub fn get_quote_spot_market(&self) -> VortexDexResult<Ref<SpotMarket>> {
        let loader = match self.0.get(&QUOTE_SPOT_MARKET_INDEX) {
            Some(loader) => loader,
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not find spot market {} at {}:{}",
                    QUOTE_SPOT_MARKET_INDEX,
                    caller.file(),
                    caller.line()
                );
                return Err(DexError::SpotMarketNotFound);
            }
        };

        match loader.load() {
            Ok(spot_market) => Ok(spot_market),
            Err(e) => {
                let caller = Location::caller();
                msg!("{:?}", e);
                msg!(
                    "Could not load spot market {} at {}:{}",
                    QUOTE_SPOT_MARKET_INDEX,
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToLoadSpotMarketAccount)
            }
        }
    }

    #[track_caller]
    #[inline(always)]
    pub fn get_quote_spot_market_mut(&self) -> VortexDexResult<RefMut<SpotMarket>> {
        let loader = match self.0.get(&QUOTE_SPOT_MARKET_INDEX) {
            Some(loader) => loader,
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not find spot market {} at {}:{}",
                    QUOTE_SPOT_MARKET_INDEX,
                    caller.file(),
                    caller.line()
                );
                return Err(DexError::SpotMarketNotFound);
            }
        };

        match loader.load_mut() {
            Ok(spot_market) => Ok(spot_market),
            Err(e) => {
                let caller = Location::caller();
                msg!("{:?}", e);
                msg!(
                    "Could not load spot market {} at {}:{}",
                    QUOTE_SPOT_MARKET_INDEX,
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToLoadSpotMarketAccount)
            }
        }
    }

    pub fn load<'b, 'c>(
        writable_spot_markets: &'b SpotMarketMap,
        account_info_iter: &'c mut Peekable<Iter<'a, AccountInfo<'a>>>,
    ) -> VortexDexResult<SpotMarketMap<'a>> {
        let mut spot_market_map: SpotMarketMap = SpotMarketMap(BTreeMap::new());

        let spot_market_discriminator: [u8; 8] = SpotMarket::discriminator();
        while let Some(account_info) = account_info_iter.peek() {
            let data = account_info
                .try_borrow_data()
                .or(Err(DexError::CouldNotLoadSpotMarketData))?;

            let expected_data_len = SpotMarket::SIZE;
            if data.len() < expected_data_len {
                break;
            }

            let account_discriminator = array_ref![data, 0, 8];
            if account_discriminator != &spot_market_discriminator {
                break;
            }

            let market_index = u16::from_le_bytes(*array_ref![data, 684, 2]);

            if spot_market_map.0.contains_key(&market_index) {
                msg!("Can not include same market index twice {}", market_index);
                return Err(DexError::InvalidSpotMarketAccount);
            }

            let account_info = account_info_iter.next().safe_unwrap()?;
            let is_writable = account_info.is_writable;
            let account_loader: AccountLoader<SpotMarket> =
                AccountLoader::try_from(account_info)
                    .or(Err(DexError::InvalidSpotMarketAccount))?;

            if writable_spot_markets.contains(&market_index) && !is_writable {
                return Err(DexError::SpotMarketWrongMutability);
            }

            spot_market_map.0.insert(market_index, account_loader);
        }

        Ok(spot_market_map)
    }
}