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


declare_id!("8Wp69Tg8PZw7Acr6DAtroYrkUt4yuvvPd8nWqYD2sJJn");


pub mod admin {
    use anchor_lang::prelude::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("4SxWFybWqHYYpMXaf1uByp1AgBV8vi8vn7mp7yLAXsH3");
    #[cfg(not(feature = "devnet"))]
    declare_id!("4SxWFybWqHYYpMXaf1uByp1AgBV8vi8vn7mp7yLAXsH3");
}


pub mod vortex_wallet {
    use anchor_lang::prelude::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("4SeSw6t5H8xFn8rLXLyo2rVAJeDXBimU5Nybw2zm95LJ");
    #[cfg(not(feature = "devnet"))]
    declare_id!("4SeSw6t5H8xFn8rLXLyo2rVAJeDXBimU5Nybw2zm95LJ");
}

#[program]
pub mod vortex_contracts {


    use super::*;

    // Methods to stake in game

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        handle_initialize(ctx)
    }

    pub fn initialize_game<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, InitGame<'info>>,
        game_id: [u8; 16],
        session_id: [u8;21],
        total_money_staked: u64,
    ) -> Result<()> {
        handle_init_game(ctx, game_id,session_id, total_money_staked )
    }


    pub fn initialize_player_bet<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, InitPlayerBet<'info>>,
        game_id: [u8; 16],
        user_betting_on_id: [u8;16],
        session_id: [u8;21],
        total_money_staked: u64
    
    ) -> Result<()> {
        handle_init_player_bet(ctx, game_id, user_betting_on_id, session_id, total_money_staked)
    }


    
    pub fn user_bet<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, MakeUserGameBet<'info>>,
        game_id: [u8; 16],
        user_betting_on_id: [u8;16],
        session_id: [u8;21],
        money_staked: u64
    ) -> Result<()> {
        handle_user_bet(ctx, game_id, user_betting_on_id, session_id , money_staked )
    }


        
    pub fn update_user_bet<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, UpdateUserGameBet<'info>>,
        game_id: [u8;16],
        user_betting_on_id: [u8;16],
        session_id: [u8;21],
        money_staked: u64
    
    ) -> Result<()> {
        handle_update_bet(ctx,   game_id, user_betting_on_id, session_id, money_staked)
    }


    

    //THis will be done by executors

    pub fn update_game_stake_status<'c: 'info, 'info>(
        ctx: Context<'_ , '_ , 'c , 'info , UpdateGameStatus<'info>>,
        game_id: [u8; 16],
        session_id: [u8;21]
    ) -> Result<()> {
        handle_update_game_stake_status(ctx, game_id , session_id)
    }


    //fn to settle bets if the game ended abruptly
    pub fn settle_all_bets_for_invalid_game<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, SettleAllBetsForInvalidGame<'info>>,
        game_id: [u8;16],
        user_betting_on_id: [u8;16],
        session_id: [u8;21],
        is_player: bool
    
    ) -> Result<()> {
        handle_settle_all_bets_for_invalid_game(ctx,   game_id, user_betting_on_id, session_id , is_player)
    }


    // fn to settle bets
    pub fn settle_all_bets<'c: 'info, 'info>(
        ctx: Context<'_, '_, 'c, 'info, SettleAllBetsForGame<'info>>,
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
