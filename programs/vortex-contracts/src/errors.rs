use anchor_lang::prelude::*;

pub type VortexDexResult<T = ()> = std::result::Result<T, DexError>;

#[error_code]
#[derive(PartialEq, Eq)]
pub enum DexError {
    #[msg("Src Balance < LP Deposit Amount.")]
    NotEnoughBalance,
    #[msg("Pool Mint Amount < 0 on LP Deposit")]
    NoPoolMintOutput,
    #[msg("Trying to burn too much")]
    BurnTooMuch,
    #[msg("Not enough out")]
    NotEnoughOut,
    #[msg("Already Made a Bet on the game")]
    AlreadyMadeABetOnGame,
    #[msg("Bet Type cannot be changed")]
    UserHasDifferentBetType,
    #[msg("You lost the bet. No amount will be rewarded")]
    UserLostTheBet,
    #[msg("Failed Unwrap")]
    FailedUnwrap,
    #[msg("Unable to load AccountLoader")]
    UnableToLoadAccountLoader,
    #[msg("DefaultError")]
    DefaultError,
    #[msg("InvalidPDASigner")]
    InvalidPDASigner,
    #[msg("InvalidPDA")]
    InvalidPDA,
    #[msg("Error During Math Computation")]
    MathError,

    #[msg("Game is still going on")]
    GameIsStillGoingOn,

    #[msg("Game has ended")]
    GameHasEnded,

    #[msg("Stake Period Over")]
    StakePeriodOver,

    #[msg("Only Admin can settle bets")]
    OnlyAdminCanSettleBets,

    #[msg("Only Admin can change game states")]
    OnlyAdminCanChangeGameStates,

    #[msg("Only Admin can initialize")]
    OnlyAdminCanInitialize,

    #[msg("Non zero transfer fee")]
    NonZeroTransferFee,
}

#[macro_export]
macro_rules! print_error {
    ($err:expr) => {{
        || {
            let error_code: DexError = $err;
            msg!("{:?} thrown at {}:{}", error_code, file!(), line!());
            $err
        }
    }};
}