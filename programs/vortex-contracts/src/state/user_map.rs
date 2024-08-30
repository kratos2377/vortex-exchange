use std::{cell::{Ref, RefMut}, collections::BTreeMap, iter::Peekable, panic::Location, slice::Iter};
use anchor_lang::{prelude::*, Discriminator};
use arrayref::array_ref;
use crate::{errors::{DexError, VortexDexResult}, safe_methods::SafeUnwrap, validate};

use super::{user::User, user_stats::UserStats};

pub struct UserMap<'a>(pub BTreeMap<Pubkey, AccountLoader<'a, User>>);

impl<'a> UserMap<'a> {
    #[track_caller]
    #[inline(always)]
    pub fn get_ref(&self, user: &Pubkey) -> VortexDexResult<Ref<User>> {
        let loader = match self.0.get(user) {
            Some(loader) => loader,
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not find user {} at {}:{}",
                    user,
                    caller.file(),
                    caller.line()
                );
                return Err(DexError::UserNotFound);
            }
        };

        match loader.load() {
            Ok(user) => Ok(user),
            Err(e) => {
                let caller = Location::caller();
                msg!("{:?}", e);
                msg!(
                    "Could not load user {} at {}:{}",
                    user,
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToLoadUserAccount)
            }
        }
    }

    #[track_caller]
    #[inline(always)]
    pub fn get_ref_mut(&self, user: &Pubkey) -> VortexDexResult<RefMut<User>> {
        let loader = match self.0.get(user) {
            Some(loader) => loader,
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not find user {} at {}:{}",
                    user,
                    caller.file(),
                    caller.line()
                );
                return Err(DexError::UserNotFound);
            }
        };

        match loader.load_mut() {
            Ok(user) => Ok(user),
            Err(e) => {
                let caller = Location::caller();
                msg!("{:?}", e);
                msg!(
                    "Could not load user {} at {}:{}",
                    user,
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToLoadUserAccount)
            }
        }
    }

    pub fn insert(&mut self, user: Pubkey, account_loader: AccountLoader<'a, User>) -> VortexDexResult {
        validate!(
            !self.0.contains_key(&user),
            DexError::InvalidUserAccount,
            "User already exists in map {:?}",
            user
        )?;

        self.0.insert(user, account_loader);

        Ok(())
    }

    pub fn empty() -> UserMap<'a> {
        UserMap(BTreeMap::new())
    }
}


pub struct UserStatsMap<'a>(pub BTreeMap<Pubkey, AccountLoader<'a, UserStats>>);

impl<'a> UserStatsMap<'a> {
    #[track_caller]
    #[inline(always)]
    pub fn get_ref(&self, authority: &Pubkey) -> VortexDexResult<Ref<UserStats>> {
        let loader = match self.0.get(authority) {
            Some(loader) => loader,
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not find user stats {} at {}:{}",
                    authority,
                    caller.file(),
                    caller.line()
                );
                return Err(DexError::UserStatsNotFound);
            }
        };

        match loader.load() {
            Ok(user_stats) => Ok(user_stats),
            Err(e) => {
                let caller = Location::caller();
                msg!("{:?}", e);
                msg!(
                    "Could not user stats {} at {}:{}",
                    authority,
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToLoadUserStatsAccount)
            }
        }
    }

    #[track_caller]
    #[inline(always)]
    pub fn get_ref_mut(&self, authority: &Pubkey) -> VortexDexResult<RefMut<UserStats>> {
        let loader = match self.0.get(authority) {
            Some(loader) => loader,
            None => {
                let caller = Location::caller();
                msg!(
                    "Could not find user stats {} at {}:{}",
                    authority,
                    caller.file(),
                    caller.line()
                );
                return Err(DexError::UserStatsNotFound);
            }
        };

        match loader.load_mut() {
            Ok(perp_market) => Ok(perp_market),
            Err(e) => {
                let caller = Location::caller();
                msg!("{:?}", e);
                msg!(
                    "Could not user stats {} at {}:{}",
                    authority,
                    caller.file(),
                    caller.line()
                );
                Err(DexError::UnableToLoadUserStatsAccount)
            }
        }
    }

    pub fn insert(
        &mut self,
        authority: Pubkey,
        account_loader: AccountLoader<'a, UserStats>,
    ) -> VortexDexResult {
        validate!(
            !self.0.contains_key(&authority),
            DexError::InvalidUserStatsAccount,
            "User stats already exists in map {:?}",
            authority
        )?;

        self.0.insert(authority, account_loader);

        Ok(())
    }

    pub fn empty() -> UserStatsMap<'a> {
        UserStatsMap(BTreeMap::new())
    }
}


pub fn load_user_maps<'a: 'b, 'b>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'b>>>,
    must_be_writable: bool,
) -> VortexDexResult<(UserMap<'b>, UserStatsMap<'b>)> {
    let mut user_map = UserMap::empty();
    let mut user_stats_map = UserStatsMap::empty();

    let user_discriminator: [u8; 8] = User::discriminator();
    let user_stats_discriminator: [u8; 8] = UserStats::discriminator();
    while let Some(user_account_info) = account_info_iter.peek() {
        let user_key = user_account_info.key;

        let data = user_account_info
            .try_borrow_data()
            .or(Err(DexError::CouldNotLoadUserData))?;

        let expected_data_len = User::SIZE;
        if data.len() < expected_data_len {
            break;
        }

        let account_discriminator = array_ref![data, 0, 8];
        if account_discriminator != &user_discriminator {
            break;
        }

        let user_account_info = account_info_iter.next().safe_unwrap()?;

        let is_writable = user_account_info.is_writable;
        if !is_writable && must_be_writable {
            return Err(DexError::UserWrongMutability);
        }

        let user_account_loader: AccountLoader<User> =
            AccountLoader::try_from(user_account_info).or(Err(DexError::InvalidUserAccount))?;

        user_map.0.insert(*user_key, user_account_loader);

        validate!(
            account_info_iter.peek().is_some(),
            DexError::UserStatsNotFound
        )?;

        let user_stats_account_info = account_info_iter.peek().safe_unwrap()?;

        let data = user_stats_account_info
            .try_borrow_data()
            .or(Err(DexError::CouldNotLoadUserStatsData))?;

        let expected_data_len = UserStats::SIZE;
        if data.len() < expected_data_len {
            return Err(DexError::InvalidUserStatsAccount);
        }

        let account_discriminator = array_ref![data, 0, 8];
        if account_discriminator != &user_stats_discriminator {
            return Err(DexError::InvalidUserStatsAccount);
        }

        let authority_slice = array_ref![data, 8, 32];
        let authority = Pubkey::try_from(*authority_slice).safe_unwrap()?;

        let user_stats_account_info = account_info_iter.next().safe_unwrap()?;

        if user_stats_map.0.contains_key(&authority) {
            continue;
        }

        let is_writable = user_stats_account_info.is_writable;
        if !is_writable && must_be_writable {
            return Err(DexError::UserStatsWrongMutability);
        }

        let user_stats_account_loader: AccountLoader<UserStats> =
            AccountLoader::try_from(user_stats_account_info)
                .or(Err(DexError::InvalidUserStatsAccount))?;

        user_stats_map.insert(authority, user_stats_account_loader)?;
    }

    Ok((user_map, user_stats_map))
}

pub fn load_user_map<'a: 'b, 'b>(
    account_info_iter: &mut Peekable<Iter<'a, AccountInfo<'b>>>,
    must_be_writable: bool,
) -> VortexDexResult<UserMap<'b>> {
    let mut user_map = UserMap::empty();

    let user_discriminator: [u8; 8] = User::discriminator();
    let user_stats_discriminator: [u8; 8] = UserStats::discriminator();
    while let Some(user_account_info) = account_info_iter.peek() {
        let user_key = user_account_info.key;

        let data = user_account_info
            .try_borrow_data()
            .or(Err(DexError::CouldNotLoadUserData))?;

        let expected_user_data_len = User::SIZE;
        let expected_user_stats_len = UserStats::SIZE;
        if data.len() < expected_user_data_len && data.len() < expected_user_stats_len {
            break;
        }

        let account_discriminator = array_ref![data, 0, 8];

        // if it is user stats, for backwards compatability, just move iter forward
        if account_discriminator == &user_stats_discriminator {
            account_info_iter.next().safe_unwrap()?;
            continue;
        }

        if account_discriminator != &user_discriminator {
            break;
        }

        let user_account_info = account_info_iter.next().safe_unwrap()?;

        let is_writable = user_account_info.is_writable;
        if !is_writable && must_be_writable {
            return Err(DexError::UserWrongMutability);
        }

        let user_account_loader: AccountLoader<User> =
            AccountLoader::try_from(user_account_info).or(Err(DexError::InvalidUserAccount))?;

        user_map.0.insert(*user_key, user_account_loader);
    }

    Ok(user_map)
}
