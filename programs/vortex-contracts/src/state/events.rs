use anchor_lang::prelude::*;

#[event]
pub struct NewUserAccountRecord {
    pub ts: i64,
    pub user_authority: Pubkey,
    pub user: Pubkey,
    pub name: [u8; 32],
}