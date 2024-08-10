use anchor_lang::prelude::*;

pub mod errors;
pub mod state;
pub mod utils;
pub mod instructions;
pub mod macros;
pub mod safe_methods;

declare_id!("HkApQpEsdzdfHsedkuZvNEbmcQXfabobbb9Yf8wdz7AZ");

#[program]
pub mod vortex_contracts {
    use super::*;

    pub fn initialize_user_account(ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
