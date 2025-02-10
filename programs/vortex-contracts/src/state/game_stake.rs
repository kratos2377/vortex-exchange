use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};

// All these instruction should store staked amount in lamports 

#[account(zero_copy(unsafe))]
#[derive(Default , PartialEq)]
#[repr(C)]
pub struct VortexState {
    pub admin: Pubkey,
    pub signer: Pubkey,
    pub signer_nonce: u8,
}

impl anchor_lang::AccountSerialize for VortexState {}
impl VortexState {
    pub const SIZE: usize = 8 + 32 + 32 + 1 + 8;
}

#[account(zero_copy(unsafe))]
#[derive(PartialEq, Debug)]
#[repr(C)]
pub struct Game {
    pub game_id: [u8;16],
    pub pubkey: Pubkey,
    pub total_pot: u64,
    pub is_game_active: bool,
    pub game_vault_key: Pubkey,
    pub is_settled: bool,
    pub session_id: [u8;21]
}

impl Game {
    pub const SIZE: usize = 8 + 16 + 32 + 8  + 1  + 1 + 21 + 8 + 32;
}

#[account(zero_copy(unsafe))]
#[derive( PartialEq, Debug)]
#[repr(C)]
pub struct UserGameBet {
    pub game_id: [u8;16],
    pub user_bet_wallet_key: Pubkey,
    pub user_betting_on_id: [u8;16],
    pub bet_type: BetType,
    pub money_staked: u64 ,
    pub is_settled: bool,
    pub session_id: [u8;21],
}

impl UserGameBet {
    pub const SIZE: usize = 8 + 16 + 32 + 16 + 4 + 8  + 1 + 21 + 8;
}

#[account(zero_copy(unsafe))]
#[derive( PartialEq, Debug)]
#[repr(C)]
pub struct PlayerTotalBet {
    pub game_id: [u8;16],
    pub user_betting_on_id: [u8;16],
    pub player_staked_money: u64,
    pub total_money_staked_on_player: u64,
    pub session_id: [u8;21],
}

impl PlayerTotalBet {
    pub const SIZE: usize =  8 + 16 + 16 + 8 +  8 + 21 + 8;
}


#[derive(Clone, Copy, BorshDeserialize , BorshSerialize , PartialEq, Debug, Eq, Default)]
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