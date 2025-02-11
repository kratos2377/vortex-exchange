use solana_program::native_token::lamports_to_sol;



pub fn calculate_winner_and_bettor_rewards(
    total_money_staked_by_user: u64,
    total_money_staked_on_player: u64,
    total_pot: u64,
) -> u64 {

    let user_ratio_staked =( (total_money_staked_by_user as f64 ) / (total_money_staked_on_player as f64)) as f64;

    (user_ratio_staked * total_pot as f64) as u64

}


pub fn calculate_bettor_money_for_invalid_game(
    total_money_staked_by_user: u64,
    total_money_staked_on_player: u64,
    player_staked_money: u64,
    total_pot: u64,
    is_player: bool
) -> u64{


    if is_player {
        return player_staked_money
    }

    return total_money_staked_by_user

}