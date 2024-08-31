

pub fn calculate_winner_and_bettor_rewards(
    total_money_staked_by_user: u64,
    total_money_staked_on_player: u64,
    total_pot: u64,
) -> u64{

    let user_ratio_staked =( (total_money_staked_by_user) / (total_money_staked_on_player)) as u64;

    user_ratio_staked * total_pot

}