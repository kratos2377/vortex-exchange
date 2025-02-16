use anchor_lang::prelude::*;
use num_traits::ToBytes;
use solana_program::native_token::{sol_to_lamports, LAMPORTS_PER_SOL};
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};
use crate::{controllers, errors::DexError, load_mut, state::game_stake::{BetType, Game, PlayerTotalBet, UserGameBet}, utils::token::{self, get_token_mint}, validate, VortexState};

const SETTLE_FEE_LAMPORTS: u64 = 100_000;


pub fn handle_initialize(
    ctx: Context<Initialize>
) -> Result<()> {
    validate!(ctx.accounts.admin.key() == crate::admin::id() , DexError::OnlyAdminCanChangeGameStates);
    let (vortex_signer, vortex_signer_nonce) =
    Pubkey::find_program_address(&[b"vortex_signer".as_ref()], ctx.program_id);

    let state = &mut ctx.accounts.state.load_init()?;
    **state = VortexState{
        admin: *ctx.accounts.admin.key,
        signer: vortex_signer,
        signer_nonce: vortex_signer_nonce,
    };

    Ok(())
}


pub fn handle_init_game<'c: 'info, 'info>(
    ctx: Context<'_ , '_ , 'c , 'info , InitGame<'info>>,
    game_id: [u8; 16],
    session_id: [u8;21],
    total_money_staked: u64
) -> Result<()> {
    let game_key = ctx.accounts.game.key();

    // Add logic to add money to so that users can recalim the money they won without paying some transaction fee

    let game = &mut ctx.accounts.game.load_init()?;
    // let clock = Clock::get()?;
    // let now = clock
    //     .unix_timestamp
    //     .cast()
    //     .or(Err(DexError::UnableToCastUnixTime))?;

    // let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    // let mint = get_token_mint(remaining_accounts_iter)?;


    **game = Game {
        game_id: game_id,
        pubkey: game_key,
        total_pot: total_money_staked,
        can_stake: true,
        session_id: session_id,
        game_vault_key: ctx.accounts.game_vault.key()
       };

    
    Ok(())

}



pub fn handle_update_game_stake_status<'c: 'info, 'info>(
    ctx: Context<'_ , '_ , 'c , 'info , UpdateGameStatus<'info>>,
    game_id: [u8; 16],
    session_id: [u8;21]
) -> Result<()> {

    let mut game = load_mut!(ctx.accounts.game)?;

    game.can_stake = false;

    Ok(())

}



pub fn handle_init_player_bet<'c: 'info, 'info>(
    ctx: Context<'_ , '_ , 'c , 'info , InitPlayerBet<'info>>,
    game_id: [u8; 16],
    user_betting_on_id: [u8;16],
    session_id: [u8;21],
    total_money_staked: u64
) -> Result<()> {
    let game_key = ctx.accounts.game.key();
    let admin_user_key = ctx.accounts.admin.key();

    // Add logic to add money to so that users can recalim the money they won without paying some transaction fee

    let mut game = load_mut!(ctx.accounts.game)?;

    require!(game.can_stake , DexError::StakePeriodOver);


    //0.002% fee this can be used to transfer amount during settling
    let fee = total_money_staked
    .checked_mul(2).unwrap()
    .checked_div(10000).unwrap();

    // We will be using USDC Token hence better to send the token value with decimal for eg if we want to stake 1.32$
    //Send anchor.BN(1.32 * 1e6)
    validate!(ctx.accounts.user_token_account.amount > total_money_staked  + fee , DexError::NotEnoughBalance);

    let player_total_bet = &mut ctx.accounts.player_total_bet.load_init()?;

    //Player total bet acts as the total bet on 1 player in 1 game in 1 particular session
    //The UserGameBet we are initializing here represents the bet the player bet on himself
    let user_game_bet = &mut ctx.accounts.user_bet.load_init()?;
    // let clock = Clock::get()?;
    // let now = clock
    //     .unix_timestamp
    //     .cast()
    //     .or(Err(DexError::UnableToCastUnixTime))?;


    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let mint = get_token_mint(remaining_accounts_iter)?;



    **player_total_bet = PlayerTotalBet {
        game_id: game_id,
        user_betting_on_id,
        total_money_staked_on_player: total_money_staked ,
        session_id: session_id,
        player_staked_money: total_money_staked,
    };

    **user_game_bet = UserGameBet{
        game_id,
        user_bet_wallet_key: admin_user_key,
        user_betting_on_id,
        bet_type: BetType::WIN,
        money_staked: total_money_staked,
        is_settled: false,
        session_id,
    };
    game.total_pot += total_money_staked;
    
    let total_usdc_to_be_transferred = fee  + total_money_staked ;

    
    token::receive(
        &ctx.accounts.token_program,
        &ctx.accounts.user_token_account,
        &ctx.accounts.game_vault,
        &ctx.accounts.admin,
        total_usdc_to_be_transferred,
        &mint,
    );


    //Transfer sols to vortex_wallet so that wallet can settle fees later
    let settle_fee = 3 * SETTLE_FEE_LAMPORTS;
    let transfer_ix = solana_program::system_instruction::transfer(
        &ctx.accounts.admin.key(),
        &crate::vortex_wallet::id(),
        settle_fee
    );
    solana_program::program::invoke(
        &transfer_ix,
        &[
            ctx.accounts.admin.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;



    Ok(())

}


pub fn handle_user_bet<'c: 'info, 'info>(
    ctx: Context<'_ , '_ , 'c , 'info , MakeUserGameBet<'info>>,
    game_id: [u8; 16],
    user_betting_on_id: [u8;16],
    session_id: [u8;21],
    money_staked: u64,
) -> Result<()> {
    let user_bet_wallet_key = ctx.accounts.user_bet_wallet_key.key();
    let mut game = load_mut!(ctx.accounts.game)?;

    require!(game.can_stake  , DexError::StakePeriodOver);

    

    let fee = money_staked
    .checked_mul(1).unwrap()
    .checked_div(10000).unwrap(); 


    validate!(ctx.accounts.user_token_account.amount > money_staked  + fee  , DexError::NotEnoughBalance);

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let mint = get_token_mint(remaining_accounts_iter)?;




    let mut player_bet = load_mut!(ctx.accounts.player_total_bet)?;

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
        bet_type: BetType::WIN,
        user_bet_wallet_key: user_bet_wallet_key,
        user_betting_on_id,
        money_staked: money_staked,
        is_settled: false,
        session_id: session_id
    };


  


    
    let total_usdc_to_be_transferred = fee + money_staked ;


    token::receive(
        &ctx.accounts.token_program,
        &ctx.accounts.user_token_account,
        &ctx.accounts.game_vault,
        &ctx.accounts.user_bet_wallet_key.to_account_info(),
        total_usdc_to_be_transferred,
        &mint,
    );

 //Transfer sols to vortex_wallet so that wallet can settle fees later
    let settle_fee = 3 * SETTLE_FEE_LAMPORTS;
    let transfer_ix = solana_program::system_instruction::transfer(
        &ctx.accounts.user_bet_wallet_key.key(),
        &crate::vortex_wallet::id(),
        settle_fee
    );
    solana_program::program::invoke(
        &transfer_ix,
        &[
            ctx.accounts.user_bet_wallet_key.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
    )?;




    Ok(())
}

pub fn handle_update_bet<'c: 'info, 'info>(
    ctx: Context<'_ , '_ , 'c , 'info , UpdateUserGameBet<'info>>,
    game_id: [u8;16],
    user_betting_on_id: [u8;16],
    session_id: [u8;21],
    money_staked: u64
) -> Result<()> {
    let mut user_bet = load_mut!(ctx.accounts.user_bet)?;
    let mut player_total_bet = load_mut!(ctx.accounts.player_total_bet)?;
    let mut game = load_mut!(ctx.accounts.game)?;

    require!(game.can_stake, DexError::StakePeriodOver);



    let fee = money_staked
    .checked_mul(1).unwrap()
    .checked_div(10000).unwrap(); 
    
    validate!(ctx.accounts.user_token_account.amount >  money_staked + fee, DexError::NotEnoughBalance);

    
  
 


    validate!(ctx.accounts.user_bet_wallet_key.lamports() > money_staked as u64 + fee as u64 , DexError::NotEnoughBalance);

    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let mint = get_token_mint(remaining_accounts_iter)?;



    let total_staked = user_bet.money_staked + money_staked;
    player_total_bet.total_money_staked_on_player += money_staked;
    game.total_pot += money_staked;
    user_bet.money_staked = total_staked;


    let total_usdc_to_be_transferred = fee + money_staked;

    token::receive(
        &ctx.accounts.token_program,
        &ctx.accounts.user_token_account,
        &ctx.accounts.game_vault,
        &ctx.accounts.user_bet_wallet_key.to_account_info(),
        total_usdc_to_be_transferred,
        &mint,
    );

    Ok(())
}



pub fn handle_settle_all_bets_for_invalid_game<'c: 'info, 'info>(
    ctx: Context<'_ , '_ , 'c , 'info , SettleAllBetsForInvalidGame<'info>>,
    game_id: [u8;16],
    user_betting_on_id: [u8;16],
    session_id: [u8;21],
    is_player: bool
) -> Result<()> {
    let game = load_mut!(ctx.accounts.game)?;
    let user_bet = load_mut!(ctx.accounts.user_bet)?;
    let player_total_bet = load_mut!(ctx.accounts.player_bet)?;



    let vortex_state = &ctx.accounts.vortex_state.load()?;
    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let mint = get_token_mint(remaining_accounts_iter)?;


    let money_to_be_rewarded_to_user = controllers::game_stake::calculate_bettor_money_for_invalid_game(
        user_bet.money_staked,
        player_total_bet.total_money_staked_on_player,
        player_total_bet.player_staked_money,
        game.total_pot,
        is_player
    );



    let _ = token::send_from_program_vault(
        &ctx.accounts.token_program,
        &ctx.accounts.game_vault,
        &ctx.accounts.to,
        &ctx.accounts.to.to_account_info(),
        vortex_state.signer_nonce,
        money_to_be_rewarded_to_user,
        &mint,
    );

    Ok(())
}


pub fn handle_settle_all_bets_for_game<'c: 'info, 'info>(
    ctx: Context<'_ , '_ , 'c , 'info , SettleAllBetsForGame<'info>>,
    game_id: [u8;16],
    user_betting_on_id: [u8;16],
    session_id: [u8;21],
    winner_id: [u8;16]
) -> Result<()> {
    let game = load_mut!(ctx.accounts.game)?;


    let user_bet = load_mut!(ctx.accounts.user_bet)?;
    let player_total_bet = load_mut!(ctx.accounts.player_bet)?;

    let vortex_state = &ctx.accounts.vortex_state.load()?;
    let remaining_accounts_iter = &mut ctx.remaining_accounts.iter().peekable();
    let mint = get_token_mint(remaining_accounts_iter)?;


    // this money that we get is in actual sol . this is not lamports we need to convert this to lamports to be transferred
    let money_to_be_rewarded_to_user = controllers::game_stake::calculate_winner_and_bettor_rewards(
        user_bet.money_staked,
        player_total_bet.total_money_staked_on_player,
        game.total_pot
    );


    let _ = token::send_from_program_vault(
        &ctx.accounts.token_program,
        &ctx.accounts.game_vault,
        &ctx.accounts.to,
        &ctx.accounts.vortex_signer,
        vortex_state.signer_nonce,
        money_to_be_rewarded_to_user,
        &mint,
    );

    Ok(())
}


#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        init,
        seeds = [b"vortex_state".as_ref()],
        space = VortexState::SIZE,
        bump,
        payer = admin
    )]
    pub state: AccountLoader<'info, VortexState>,
    pub quote_asset_mint: Box<InterfaceAccount<'info, Mint>>,
    /// CHECK: checked in `initialize`
    pub vortex_signer: AccountInfo<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16] , session_id: [u8;21])]
pub struct InitGame<'info> {
// in this case payer should be some global admin
    #[account(
        init,
        seeds = [b"game", game_id.as_ref() , session_id.as_ref()],
        bump,
        space = Game::SIZE,
        payer = admin
    )]
    pub game: AccountLoader<'info, Game>,
    pub game_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        seeds = [b"game_vault".as_ref(), game_id.as_ref() , session_id.as_ref()],
        bump,
        payer = admin,
        token::mint = game_mint,
        token::authority = vortex_signer
    )]
    pub game_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = &game_vault.mint.eq(&user_token_account.mint)
    )]
    pub user_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account()]
    /// CHECK: program signer
    pub vortex_signer: AccountInfo<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16] , session_id: [u8;21])]
pub struct UpdateGameStatus<'info> {
// in this case payer should be some global admin
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref() , session_id.as_ref()],
        bump,
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(
        mut,
        seeds = [b"game_vault".as_ref(), game_id.as_ref() , session_id.as_ref()],
        bump,
    )]
    pub game_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: program signer
    pub vortex_signer: AccountInfo<'info>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16] , session_id: [u8;21])]
pub struct InitPlayerBet<'info> {
    #[account(
        init,
        seeds = [b"player_bet", game_id.as_ref() , user_betting_on_id.as_ref() , session_id.as_ref()],
        bump,
        space = PlayerTotalBet::SIZE,
        payer = admin
    )]
    pub player_total_bet: AccountLoader<'info, PlayerTotalBet>,
    #[account(
        init,  seeds = [b"user_game_bet", game_id.as_ref(), user_betting_on_id.as_ref(), admin.key.as_ref(), session_id.as_ref()],
        bump,
        space = UserGameBet::SIZE,
        payer = admin
    )]
    pub user_bet: AccountLoader<'info, UserGameBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref() , session_id.as_ref()],
        bump,
    )]
    pub game: AccountLoader<'info, Game>,
    pub game_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        mut,
        seeds = [b"game_vault".as_ref(), game_id.as_ref() , session_id.as_ref()],
        bump,
    )]
    pub game_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = &game_vault.mint.eq(&user_token_account.mint)
    )]
    pub user_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16], session_id: [u8;21])]
pub struct MakeUserGameBet<'info> {

    #[account(
        init,  seeds = [b"user_game_bet", game_id.as_ref(), user_betting_on_id.as_ref(), user_bet_wallet_key.key.as_ref(), session_id.as_ref()],
        bump,
        space = UserGameBet::SIZE,
        payer = user_bet_wallet_key
    )]
    pub user_bet: AccountLoader<'info, UserGameBet>,
    #[account(
        mut,
        seeds = [b"player_bet", game_id.as_ref() , user_betting_on_id.as_ref() , session_id.as_ref()],
        bump
    )]
    pub player_total_bet: AccountLoader<'info, PlayerTotalBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref() , session_id.as_ref()],
        bump
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(
        mut,
        seeds = [b"game_vault".as_ref(), game_id.as_ref() , session_id.as_ref()],
        bump,
    )]
    pub game_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = &game_vault.mint.eq(&user_token_account.mint)
    )]
    pub user_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub user_bet_wallet_key: Signer<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16] , session_id: [u8;21])]
pub struct UpdateUserGameBet<'info> {

    #[account(
        mut,
        seeds = [b"user_game_bet",  game_id.as_ref(), user_betting_on_id.as_ref(), user_bet_wallet_key.key.as_ref() , session_id.as_ref()],
        bump
    )]
    pub user_bet: AccountLoader<'info, UserGameBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref() , session_id.as_ref()],
        bump
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(
        mut,
        seeds = [b"player_bet", game_id.as_ref() , user_betting_on_id.as_ref(), session_id.as_ref()],
        bump
    )]
    pub player_total_bet: AccountLoader<'info, PlayerTotalBet>, 
    #[account(
        mut,
        seeds = [b"game_vault".as_ref(), game_id.as_ref() , session_id.as_ref()],
        bump,
    )]
    pub game_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = &game_vault.mint.eq(&user_token_account.mint)
    )]
    pub user_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub user_bet_wallet_key: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}



#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16], session_id: [u8;21])]
pub struct SettleAllBetsForInvalidGame<'info> {

    #[account(
        mut,
        seeds = [b"user_game_bet", game_id.as_ref(), user_betting_on_id.as_ref(), receiver_public_key.key.as_ref() , session_id.as_ref()],
        bump
    )]
    pub user_bet: AccountLoader<'info, UserGameBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref(), session_id.as_ref()],
        bump
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(
        mut,
        seeds = [b"player_bet", game_id.as_ref(), user_betting_on_id.as_ref(), session_id.as_ref()],
        bump
    )]
    pub player_bet: AccountLoader<'info, PlayerTotalBet>,
    #[account(
        mut,
        seeds = [b"game_vault".as_ref(), game_id.as_ref() , session_id.as_ref()],
        bump,
        token::authority = vortex_signer
    )]
    pub game_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        seeds = [b"vortex_state"],
        bump,
    )]
    pub vortex_state: AccountLoader<'info, VortexState>,
    #[account(
        mut,
        constraint = &game_vault.mint.eq(&to.mint)
    )]
    pub to: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: reciever info
    pub receiver_public_key: AccountInfo<'info>,
    /// CHECK: program signer
    pub vortex_signer: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}


#[derive(Accounts)]
#[instruction(game_id: [u8;16], user_betting_on_id: [u8;16], session_id: [u8;21])]
pub struct SettleAllBetsForGame<'info> {

    #[account(
        mut,
        seeds = [b"user_game_bet", game_id.as_ref(), user_betting_on_id.as_ref(), receiver_public_key.key.as_ref() , session_id.as_ref()],
        bump
    )]
    pub user_bet: AccountLoader<'info, UserGameBet>,
    #[account(
        mut,
        seeds = [b"game", game_id.as_ref(), session_id.as_ref()],
        bump
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(
        mut,
        seeds = [b"player_bet", game_id.as_ref(), user_betting_on_id.as_ref(), session_id.as_ref()],
        bump
    )]
    pub player_bet: AccountLoader<'info, PlayerTotalBet>,
    #[account(
        mut,
        seeds = [b"game_vault".as_ref(), game_id.as_ref() , session_id.as_ref()],
        bump
    )]
    pub game_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        seeds = [b"vortex_state"],
        bump,
    )]
    pub vortex_state: AccountLoader<'info, VortexState>,
    #[account(
        mut,
        constraint = &game_vault.mint.eq(&to.mint)
    )]
    pub to: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: reciever info
    pub receiver_public_key: AccountInfo<'info>,
    /// CHECK: program signer
    pub vortex_signer: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
}
