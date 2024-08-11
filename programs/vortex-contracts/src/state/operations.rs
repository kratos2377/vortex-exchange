use solana_program::msg;

#[derive(Clone, Copy, PartialEq, Debug, Eq)]
pub enum SpotOperation {
    UpdateCumulativeInterest = 0b00000001,
    Fill = 0b00000010,
    Deposit = 0b00000100,
    Withdraw = 0b00001000,
    Liquidation = 0b00010000,
}

const ALL_SPOT_OPERATIONS: [SpotOperation; 5] = [
    SpotOperation::UpdateCumulativeInterest,
    SpotOperation::Fill,
    SpotOperation::Deposit,
    SpotOperation::Withdraw,
    SpotOperation::Liquidation,
];

impl SpotOperation {
    pub fn is_operation_paused(current: u8, operation: SpotOperation) -> bool {
        current & operation as u8 != 0
    }

    pub fn log_all_operations_paused(current: u8) {
        for operation in ALL_SPOT_OPERATIONS.iter() {
            if Self::is_operation_paused(current, *operation) {
                msg!("{:?} is paused", operation);
            }
        }
    }
}