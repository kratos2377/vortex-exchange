use ahash::HashMap;
use anchor_lang::prelude::*;



#[account]
#[repr(C)]
pub struct Game {
    pub game_id: [u8;32],
    pub total_money_stake: f64,
    pub users_staked: HashMap<Pubkey , f64>,
}