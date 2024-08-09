use anchor_lang::prelude::Pubkey;



#[account]
#[repr(C)]
pub struct UserStats {
    pub authority: Pubkey,
    pub name: [u8; 32],
    pub total_deposits: u64,
    pub total_withdraws: u64,
    pub status: u8,

    pub cumulative_spot_fees: i64,
    pub cumulative_perp_funding: i64,
    pub liquidation_margin_freed: u64,
    pub last_active_slot: u64,
    pub next_order_id: u32,
    pub max_margin_ratio: u32,
    pub next_liquidation_id: u16,
}