use anchor_lang::prelude::*;
use crate::game_stake::Game;



pub fn handle_init_game(
    ctx: Context<InitGame>,
    game_id: [u8; 16],
    total_money_staked: f64,

) -> Result<()> {

}


#[derive(Accounts)]
pub struct InitGame<'info> {

    #[account(
        init,
        space = Game::SIZE,
        payer = payer
    )]
    pub game: AccountLoader<'info, Game>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}
