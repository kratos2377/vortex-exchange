
use std::collections::HashMap;

use anchor_lang::prelude::*;


#[account(zero_copy(unsafe))]
#[derive(PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Game {
    pub game_id: [u8;16],
    pub pubkey: Pubkey,
    pub total_money_staked: f64,
    pub is_game_active: bool
}

impl Game {
    pub const SIZE: usize = 60;
}