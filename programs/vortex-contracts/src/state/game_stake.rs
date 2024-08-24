use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};


#[account(zero_copy(unsafe))]
#[derive(PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Game {
    pub game_id: [u8;16],
    pub pubkey: Pubkey,
    pub total_pot: f64,
    pub is_game_active: bool,
    pub is_settled: bool,
}

impl Game {
    pub const SIZE: usize = 65;
}

#[account(zero_copy(unsafe))]
#[derive( PartialEq, Eq, Debug)]
#[repr(C)]
pub struct UserGameBet {
    pub game_id: [u8;16],
    pub user_bet_wallet_key: Pubkey,
    pub user_betting_on_id: [u8;16],
    pub bet_type: BetType,
    pub money_staked: f64,
    pub is_settled: bool,
}

impl UserGameBet {
    pub const SIZE: usize = 80;
}

#[account(zero_copy(unsafe))]
#[derive( PartialEq, Eq, Debug)]
#[repr(C)]
pub struct PlayerTotalBet {
    pub game_id: [u8;16],
    pub user_betting_on_id: [u8;16],
    pub total_money_staked_on_player: f64,
}

impl PlayerTotalBet {
    pub const SIZE: usize = 50;
}


#[derive(Clone, Copy, PartialEq, Debug, Eq, Default)]
pub enum BetType {
    #[default]
    WIN,
    LOSE
}

#[derive(Clone, Copy, PartialEq, Debug, Eq)]
pub enum GameSettleType {
    HostDisconnected,
    GameOver
}