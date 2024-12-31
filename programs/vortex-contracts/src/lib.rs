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
use crate::state::{game_stake::* , load_ref::* , pool::* };
use crate::instructions::{liquidity::*, game_stake::* , user::*};


#[cfg(feature = "devnet")]
declare_id!("4TKQybhrJ5oHwrBiwe4jRT3mtQFxpxfgoHBoxHj4KUc4");
#[cfg(not(feature = "devnet"))]
declare_id!("4TKQybhrJ5oHwrBiwe4jRT3mtQFxpxfgoHBoxHj4KUc4");


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

    // pub fn initialize_pool(
    //     ctx: Context<InitializePool>, 
    //     fee_numerator: u64,
    //     fee_denominator: u64,
    // ) -> Result<()> {
    //     handle_initialize(ctx, fee_numerator, fee_denominator)
    // }

    pub fn remove_liquidity(
        ctx: Context<LiquidityOperation>, 
        burn_amount: u64,
    ) -> Result<()> {
        handle_remove_liquidity(ctx, burn_amount)
    }

    pub fn add_liquidity(
        ctx: Context<LiquidityOperation>, 
        amount_liq0: u64, 
        amount_liq1: u64, 
    ) -> Result<()> {
        handle_add_liquidity(ctx, amount_liq0, amount_liq1)
    }

    pub fn swap(
        ctx: Context<Swap>, 
        amount_in: u64, 
        min_amount_out: u64,
    ) -> Result<()> {
        handle_swap(ctx, amount_in, min_amount_out)
    }

    // Methods to stake in game
    pub fn initialize_game(
        ctx: Context<InitGame>,
        game_id: [u8; 16],
        total_money_staked: u64,
    
    ) -> Result<()> {
        handle_init_game(ctx, game_id, total_money_staked)
    }


    pub fn initialize_player_bet(
        ctx: Context<InitPlayerBet>,
        game_id: [u8; 16],
        total_money_staked: u64,
        user_betting_on_id: [u8;16]
    
    ) -> Result<()> {
        handle_init_player_bet(ctx, game_id, total_money_staked, user_betting_on_id)
    }


    
    pub fn user_bet(
        ctx: Context<MakeUserGameBet>,
        game_id: [u8; 16],
        user_betting_on_id: [u8;16],
        money_staked: u64,
        bet_type: BetType
    
    ) -> Result<()> {
        handle_user_bet(ctx, game_id, user_betting_on_id, money_staked, bet_type)
    }


        
    pub fn update_user_bet(
        ctx: Context<UpdateUserGameBet>,
        bet_type: BetType,
        game_id: [u8;16],
        user_betting_on_id: [u8;16],
        money_staked: u64,
    
    ) -> Result<()> {
        handle_update_bet(ctx, bet_type,  game_id, user_betting_on_id, money_staked)
    }


    //THis will be done by executors

    pub fn settle_all_bets(
        ctx: Context<SettleAllBetsForGame>,
        bet_type: BetType,
        game_id: [u8;16],
        user_betting_on_id: [u8;16],
        winner_id: [u8;16],
    
    ) -> Result<()> {
        handle_settle_all_bets_for_game(ctx, bet_type,  game_id, user_betting_on_id, winner_id)
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
