#![allow(clippy::too_many_arguments)]
#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::comparison_chain)]
use anchor_lang::prelude::*;
pub mod errors;
pub mod state;
pub mod utils;
pub mod instructions;
pub mod macros;
pub mod safe_methods;
pub mod controllers;
use crate::state::game_stake::*;
use crate::instructions::game_stake::*;


#[cfg(feature = "devnet")]
declare_id!("mzB1EFq8qsGR9m3U2YFivNPfubsmfKUebLY43AjGcHt");
#[cfg(not(feature = "devnet"))]
declare_id!("mzB1EFq8qsGR9m3U2YFivNPfubsmfKUebLY43AjGcHt");


pub mod admin {
    use anchor_lang::prelude::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("4SxWFybWqHYYpMXaf1uByp1AgBV8vi8vn7mp7yLAXsH3");
    #[cfg(not(feature = "devnet"))]
    declare_id!("4SxWFybWqHYYpMXaf1uByp1AgBV8vi8vn7mp7yLAXsH3");
}


#[program]
pub mod vortex_contracts {


    use super::*;

    // Methods to stake in game
    pub fn initialize_game(
        ctx: Context<InitGame>,
        game_id: [u8; 16],
        session_id: [u8;21],
        total_money_staked: f64,
    ) -> Result<()> {
        handle_init_game(ctx, game_id,session_id, total_money_staked )
    }


    pub fn update_game_status(
        ctx: Context<UpdateGameStatus>,
        game_id: [u8; 16],
        session_id: [u8;21]
    ) -> Result<()> {
        handle_update_game_status(ctx, game_id , session_id)
    }


    pub fn update_game_is_settled_status(
        ctx: Context<UpdateGameSettleStatus>,
        game_id: [u8; 16],
        session_id: [u8;21]
    ) -> Result<()> {
        handle_update_game_is_settled_status(ctx, game_id , session_id)
    }



    pub fn initialize_player_bet(
        ctx: Context<InitPlayerBet>,
        game_id: [u8; 16],
        user_betting_on_id: [u8;16],
        session_id: [u8;21],
        total_money_staked: f64
    
    ) -> Result<()> {
        handle_init_player_bet(ctx, game_id, user_betting_on_id, session_id, total_money_staked)
    }


    
    pub fn user_bet(
        ctx: Context<MakeUserGameBet>,
        game_id: [u8; 16],
        user_betting_on_id: [u8;16],
        session_id: [u8;21],
        money_staked: f64
    ) -> Result<()> {
        handle_user_bet(ctx, game_id, user_betting_on_id, session_id , money_staked )
    }


        
    pub fn update_user_bet(
        ctx: Context<UpdateUserGameBet>,
        game_id: [u8;16],
        user_betting_on_id: [u8;16],
        session_id: [u8;21],
        money_staked: f64
    
    ) -> Result<()> {
        handle_update_bet(ctx,   game_id, user_betting_on_id, session_id, money_staked)
    }


    //THis will be done by executors

    pub fn settle_all_bets(
        ctx: Context<SettleAllBetsForGame>,
        game_id: [u8;16],
        user_betting_on_id: [u8;16],
        session_id: [u8;21],
        winner_id: [u8;16],
    
    ) -> Result<()> {
        handle_settle_all_bets_for_game(ctx,   game_id, user_betting_on_id, session_id , winner_id)
    }



}

#[cfg(not(feature = "no-entrypoint"))]
use solana_security_txt::security_txt;
#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "Vortex",
    project_url: "none",
    contacts: "none",
    policy: "https://github.com/kratos2377/vortex-contracts/blob/main/SECURITY.md",
    preferred_languages: "en",
    source_code: "https://github.com/kratos2377/vortex-contracts"
}
