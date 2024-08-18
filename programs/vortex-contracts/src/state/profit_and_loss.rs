use anchor_lang::prelude::*;
use borsh::{BorshSerialize , BorshDeserialize};
use crate::errors::{DexError, VortexDexResult};
use std::panic::Location;

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum SettlePnlMode {
    MustSettle,
    TrySettle,
}

impl SettlePnlMode {
    #[track_caller]
    #[inline(always)]
    pub fn result(self, error_code: DexError, market_index: u16, msg: &str) -> VortexDexResult {
        let caller = Location::caller();
        msg!(msg);
        msg!(
            "Error {:?} for market {} at {}:{}",
            error_code,
            market_index,
            caller.file(),
            caller.line()
        );
        match self {
            SettlePnlMode::MustSettle => Err(error_code),
            SettlePnlMode::TrySettle => Ok(()),
        }
    }
}
