use anchor_lang::prelude::*;
use num_traits::ToBytes;
use solana_program::native_token::LAMPORTS_PER_SOL;
use crate::{controllers, errors::DexError,  load_mut, state::game_stake::{BetType, Game, PlayerTotalBet, UserGameBet}, validate};



pub fn handle_init_game(
    ctx: Context<InitGame>,
    game_id: [u8; 16],
    total_money_staked: u64,
) -> Result<()> {
    let game_key = ctx.accounts.game.key();

    // Add logic to add money to so that users can recalim the money they won without paying some transaction fee

    let game = &mut ctx.accounts.game.load_init()?;
    // let clock = Clock::get()?;
    // let now = clock
    //     .unix_timestamp
    //     .cast()
    //     .or(Err(DexError::UnableToCastUnixTime))?;

    let total_money_staked_u128 = total_money_staked as u128;

    let fee = total_money_staked_u128
    .checked_mul(1 as u128).unwrap()
    .checked_div(10000 as u128).unwrap(); 

    **game = Game {
        game_id: game_id,
        pubkey: game_key,
        total_pot: total_money_staked,
        is_game_active: true,
        is_settled: false,
    };

    let total_lamports_to_be_transferred = (fee as u64 * LAMPORTS_PER_SOL) as u64;
    &solana_program::system_instruction::transfer(
        &ctx.accounts.admin.key(),
        &crate::admin::id(),
        total_lamports_to_be_transferred,
    );

    Ok(())

}

pub fn handle_init_player_bet(
    ctx: Context<InitPlayerBet>,
    game_id: [u8; 16],
    total_money_staked: u64,
    user_betting_on_id: [u8;16]

) -> Result<()> {
    let game_key = ctx.accounts.game.key();
    let admin_user_key = ctx.accounts.admin.key();

    // Add logic to add money to so that users can recalim the money they won without paying some transaction fee

    let mut game = load_mut!(ctx.accounts.game)?;
    let player_total_bet = &mut ctx.accounts.player_total_bet.load_init()?;
    // let clock = Clock::get()?;
    // let now = clock
    //     .unix_timestamp
    //     .cast()
    //     .or(Err(DexError::UnableToCastUnixTime))?;

    game.total_pot += total_money_staked;

    let total_money_staked_u128 = total_money_staked as u128;

    let fee = total_money_staked_u128
    .checked_mul(1 as u128).unwrap()
    .checked_div(10000 as u128).unwrap(); 


    **player_total_bet = PlayerTotalBet {
        game_id: game_id,
        user_betting_on_id,
        total_money_staked_on_player: total_money_staked,
    };

    
    let total_lamports_to_be_transferred = ( (fee as u64 + total_money_staked) * LAMPORTS_PER_SOL) as u64;
    &solana_program::system_instruction::transfer(
        &admin_user_key,
        &crate::admin::id(),
        total_lamports_to_be_transferred,
    );


    Ok(())

}


pub fn handle_user_bet(
    ctx: Context<MakeUserGameBet>,
    game_id: [u8; 16],
    user_betting_on_id: [u8;16],
    money_staked: u64,
    bet_type: BetType
) -> Result<()> {
    let user_bet_account_model = load_mut!(ctx.accounts.user_bet)?;
    let user_bet_wallet_key = ctx.accounts.user_bet_wallet_key.key();
    let mut game = load_mut!(ctx.accounts.game)?;
    let mut player_bet = load_mut!(ctx.accounts.player_total_bet)?;

    validate!(user_bet_account_model.game_id == game_id, DexError::AlreadyMadeABetOnGame );

    let user_bet = &mut ctx.accounts.user_bet.load_init()?;
    // let clock = Clock::get()?;
    // let now = clock
    //     .unix_timestamp
    //     .cast()
    //     .or(Err(DexError::UnableToCastUnixTime))?;

    game.total_pot += money_staked;
    player_bet.total_money_staked_on_player += money_staked;
    **user_bet = UserGameBet {
        game_id,
        bet_type: bet_type,
        user_bet_wallet_key: user_bet_wallet_key,
        user_betting_on_id,
        money_staked,
        is_settled: false,
    };


    let total_money_staked_u128 = money_staked as u128;

    let fee = total_money_staked_u128
    .checked_mul(1 as u128).unwrap()
    .checked_div(10000 as u128).unwrap(); 

    
    let total_lamports_to_be_transferred = ((fee as u64 + money_staked) * LAMPORTS_PER_SOL) as u64;
    &solana_program::system_instruction::transfer(
        &user_bet_wallet_key.key(),
        &crate::admin::id(),
        total_lamports_to_be_transferred,
    );



    Ok(())
}

pub fn handle_update_bet(
    ctx: Context<UpdateUserGameBet>,
    bet_type: BetType,
    game_id: [u8;16],
    user_betting_on_id: [u8;16],
    money_staked: u64,
) -> Result<()> {
    let mut user_bet = load_mut!(ctx.accounts.user_bet)?;
    let mut player_total_bet = load_mut!(ctx.accounts.player_total_bet)?;
    let mut game = load_mut!(ctx.accounts.game)?;
    let user_bet_wallet_key = ctx.accounts.user_bet_wallet_key.key();
    validate!(user_bet.user_betting_on_id == user_betting_on_id && user_bet.bet_type != bet_type, DexError::UserHasDifferentBetType);

    let total_staked = user_bet.money_staked + money_staked;
    player_total_bet.total_money_staked_on_player += money_staked;
    game.total_pot += money_staked;
    user_bet.money_staked = total_staked;

    let total_money_staked_u128 = money_staked as u128;

    let fee = total_money_staked_u128
    .checked_mul(1 as u128).unwrap()
    .checked_div(10000 as u128).unwrap(); 

    let total_lamports_to_be_transferred = ((fee as u64 + money_staked) * LAMPORTS_PER_SOL) as u64;
    &solana_program::system_instruction::transfer(
        &user_bet_wallet_key,
        &crate::admin::id(),
        total_lamports_to_be_transferred,
    );


    Ok(())
}


pub fn handle_settle_all_bets_for_game(
    ctx: Context<SettleAllBetsForGame>,
    bet_type: BetType,
    game_id: [u8;16],
    user_betting_on_id: [u8;16],
    winner_id: [u8;16],
) -> Result<()> {
    let user_bet_wallet_key = &ctx.accounts.user_bet_wallet_key.key();
    let game = load_mut!(ctx.accounts.game)?;
    let user_bet = load_mut!(ctx.accounts.user_bet)?;
    let player_total_bet = load_mut!(ctx.accounts.player_bet)?;

    validate!(winner_id != user_bet.user_betting_on_id, DexError::UserLostTheBet);

    let money_to_be_rewarded_to_user = controllers::game_stake::calculate_winner_and_bettor_rewards(
        user_bet.money_staked,
        player_total_bet.total_money_staked_on_player,
        game.total_pot
    );

    let total_lamports_to_be_transferred = (money_to_be_rewarded_to_user * LAMPORTS_PER_SOL) as u64;
        &solana_program::system_instruction::transfer(
            &crate::admin::id(),
            user_bet_wallet_key,
            total_lamports_to_be_transferred,
        );

    Ok(())
}

#[derive(Accounts)]
#[instruction(game_id: [u8;16])]
pub struct InitGame<'info> {
// in this case payer should be some global admin
    #[account(
        init,
        seeds = [b"game", game_id.as_ref()],
        bump,
        space = Game::SIZE,
        payer = admin
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16])]
pub struct InitPlayerBet<'info> {
// in this case payer should be some global admin
    #[account(
        init,
        seeds = [b"player_bet", game_id.as_ref() , user_betting_on_id.as_ref()],
        bump,
        space = PlayerTotalBet::SIZE,
        payer = admin
    )]
    pub player_total_bet: AccountLoader<'info, PlayerTotalBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref()],
        bump,
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16])]
pub struct MakeUserGameBet<'info> {

    #[account(
        init,  seeds = [b"user_game_bet", game_id.as_ref(), user_betting_on_id.as_ref(), user_bet_wallet_key.key.as_ref()],
        bump,
        space = UserGameBet::SIZE,
        payer = user_bet_wallet_key
    )]
    pub user_bet: AccountLoader<'info, UserGameBet>,
    #[account(
        mut,
        seeds = [b"player_bet", game_id.as_ref() , user_betting_on_id.as_ref()],
        bump
    )]
    pub player_total_bet: AccountLoader<'info, PlayerTotalBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref()],
        bump
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(mut)]
    pub user_bet_wallet_key: Signer<'info>,
    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16])]
pub struct UpdateUserGameBet<'info> {

    #[account(
        mut,
        seeds = [b"user_game_bet",  game_id.as_ref(), user_betting_on_id.as_ref(), user_bet_wallet_key.key.as_ref()],
        bump
    )]
    pub user_bet: AccountLoader<'info, UserGameBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref()],
        bump
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(
        mut,
        seeds = [b"player_bet", game_id.as_ref() , user_betting_on_id.as_ref()],
        bump
    )]
    pub player_total_bet: AccountLoader<'info, PlayerTotalBet>, 
    pub user_bet_wallet_key: Signer<'info>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16])]
pub struct SettleAllBetsForGame<'info> {

    #[account(
        mut,
        seeds = [b"user_game_bet", game_id.as_ref(), user_betting_on_id.as_ref(), user_bet_wallet_key.key.as_ref()],
        bump
    )]
    pub user_bet: AccountLoader<'info, UserGameBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref()],
        bump
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(
        mut,
        seeds = [b"player_bet", game_id.as_ref(), user_betting_on_id.as_ref()],
        bump
    )]
    pub player_bet: AccountLoader<'info, PlayerTotalBet>,
    pub user_bet_wallet_key: Signer<'info>,
}
