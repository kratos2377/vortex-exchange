

pub fn calculate_winner_and_bettor_rewards(
    total_money_staked_by_user: f64,
    total_money_staked_on_player: f64,
    total_pot: f64,
) -> f64{

    let user_ratio_staked =( (total_money_staked_by_user) / (total_money_staked_on_player)) as f64;

    user_ratio_staked * total_pot

}