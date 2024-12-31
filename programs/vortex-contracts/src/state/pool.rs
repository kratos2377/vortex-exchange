use anchor_lang::prelude::*;

#[account(zero_copy(unsafe))]
#[derive( PartialEq, Eq, Debug , Default)]
#[repr(C)]
pub struct PoolState {
    pub total_amount_minted: u64, 
    pub fee_numerator: u64, 
    pub fee_denominator: u64,
}
