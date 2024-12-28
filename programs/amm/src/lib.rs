#![cfg(not(feature = "no-entrypoint"))]

use solana_program::declare_id;
use crate::{error::AmmError, processor::Processor};
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult,
    program_error::PrintProgramError, pubkey::Pubkey,
};

pub mod error;
pub mod instruction;
pub mod invoker;
pub mod log;
pub mod math;
pub mod processor;
pub mod state;

declare_id!("2VN7F63kL5N7AXHqFW7VYdr7b7CANQA5iM1GbTNZVbXE");

entrypoint!(process_instruction);
fn process_instruction<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    if let Err(error) = Processor::process(program_id, accounts, instruction_data) {
        // catch the error so we can print it
        error.print::<AmmError>();
        return Err(error);
    }
    Ok(())
}
