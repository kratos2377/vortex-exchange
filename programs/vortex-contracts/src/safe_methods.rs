use crate::errors::{DexError, VortexDexResult};
use solana_program::msg;
use std::panic::Location;
use num_traits::{One, Zero};


// Safe Unwrap
pub trait SafeUnwrap {
    type Item;

    fn safe_unwrap(self) -> VortexDexResult<Self::Item>;
}

impl<T> SafeUnwrap for Option<T> {
    type Item = T;

    #[track_caller]
    #[inline(always)]
    fn safe_unwrap(self) -> VortexDexResult<T> {
        match self {
            Some(v) => Ok(v),
            None => {
                let caller = Location::caller();
                msg!("Unwrap error thrown at {}:{}", caller.file(), caller.line());
                Err(DexError::FailedUnwrap)
            }
        }
    }
}

impl<T, U> SafeUnwrap for Result<T, U> {
    type Item = T;

    #[track_caller]
    #[inline(always)]
    fn safe_unwrap(self) -> VortexDexResult<T> {
        match self {
            Ok(v) => Ok(v),
            Err(_) => {
                let caller = Location::caller();
                msg!("Unwrap error thrown at {}:{}", caller.file(), caller.line());
                Err(DexError::FailedUnwrap)
            }
        }
    }
}


// Safe math operations

pub trait SafeMath: Sized {
    fn safe_add(self, rhs: Self) -> VortexDexResult<Self>;
    fn safe_sub(self, rhs: Self) -> VortexDexResult<Self>;
    fn safe_mul(self, rhs: Self) -> VortexDexResult<Self>;
    fn safe_div(self, rhs: Self) -> VortexDexResult<Self>;
    fn safe_div_ceil(self, rhs: Self) -> VortexDexResult<Self>;
}

macro_rules! checked_impl {
    ($t:ty) => {
        impl SafeMath for $t {
            #[track_caller]
            #[inline(always)]
            fn safe_add(self, v: $t) -> VortexDexResult<$t> {
                match self.checked_add(v) {
                    Some(result) => Ok(result),
                    None => {
                        let caller = Location::caller();
                        msg!("Math error thrown at {}:{}", caller.file(), caller.line());
                        Err(DexError::MathError)
                    }
                }
            }

            #[track_caller]
            #[inline(always)]
            fn safe_sub(self, v: $t) -> VortexDexResult<$t> {
                match self.checked_sub(v) {
                    Some(result) => Ok(result),
                    None => {
                        let caller = Location::caller();
                        msg!("Math error thrown at {}:{}", caller.file(), caller.line());
                        Err(DexError::MathError)
                    }
                }
            }

            #[track_caller]
            #[inline(always)]
            fn safe_mul(self, v: $t) -> VortexDexResult<$t> {
                match self.checked_mul(v) {
                    Some(result) => Ok(result),
                    None => {
                        let caller = Location::caller();
                        msg!("Math error thrown at {}:{}", caller.file(), caller.line());
                        Err(DexError::MathError)
                    }
                }
            }

            #[track_caller]
            #[inline(always)]
            fn safe_div(self, v: $t) -> VortexDexResult<$t> {
                match self.checked_div(v) {
                    Some(result) => Ok(result),
                    None => {
                        let caller = Location::caller();
                        msg!("Math error thrown at {}:{}", caller.file(), caller.line());
                        Err(DexError::MathError)
                    }
                }
            }

            #[track_caller]
            #[inline(always)]
            fn safe_div_ceil(self, v: $t) -> VortexDexResult<$t> {
                match self.checked_ceil_div(v) {
                    Some(result) => Ok(result),
                    None => {
                        let caller = Location::caller();
                        msg!("Math error thrown at {}:{}", caller.file(), caller.line());
                        Err(DexError::MathError)
                    }
                }
            }
        }
    };
}

// checked_impl!(U256);
// checked_impl!(U192);
checked_impl!(u128);
checked_impl!(u64);
checked_impl!(u32);
checked_impl!(u16);
checked_impl!(u8);
checked_impl!(i128);
checked_impl!(i64);
checked_impl!(i32);
checked_impl!(i16);
checked_impl!(i8);

pub trait SafeDivFloor: Sized {
    /// Perform floor division
    fn safe_div_floor(self, rhs: Self) -> VortexDexResult<Self>;
}

macro_rules! div_floor_impl {
    ($t:ty) => {
        impl SafeDivFloor for $t {
            #[track_caller]
            #[inline(always)]
            fn safe_div_floor(self, v: $t) -> VortexDexResult<$t> {
                match self.checked_floor_div(v) {
                    Some(result) => Ok(result),
                    None => {
                        let caller = Location::caller();
                        msg!("Math error thrown at {}:{}", caller.file(), caller.line());
                        Err(DexError::MathError)
                    }
                }
            }
        }
    };
}

div_floor_impl!(i128);
div_floor_impl!(i64);
div_floor_impl!(i32);
div_floor_impl!(i16);
div_floor_impl!(i8);



// Defining Floor and Ceil fns

pub trait CheckedFloorDiv: Sized {
    /// Perform floor division
    fn checked_floor_div(&self, rhs: Self) -> Option<Self>;
}

macro_rules! checked_impl {
    ($t:ty) => {
        impl CheckedFloorDiv for $t {
            #[track_caller]
            #[inline]
            fn checked_floor_div(&self, rhs: $t) -> Option<$t> {
                let quotient = self.checked_div(rhs)?;

                let remainder = self.checked_rem(rhs)?;

                if remainder != <$t>::zero() {
                    quotient.checked_sub(<$t>::one())
                } else {
                    Some(quotient)
                }
            }
        }
    };
}

checked_impl!(i128);
checked_impl!(i64);
checked_impl!(i32);
checked_impl!(i16);
checked_impl!(i8);

pub trait CheckedCeilDiv: Sized {
    /// Perform ceiling division
    fn checked_ceil_div(&self, rhs: Self) -> Option<Self>;
}

macro_rules! checked_impl {
    ($t:ty) => {
        impl CheckedCeilDiv for $t {
            #[track_caller]
            #[inline]
            fn checked_ceil_div(&self, rhs: $t) -> Option<$t> {
                let quotient = self.checked_div(rhs)?;

                let remainder = self.checked_rem(rhs)?;

                if remainder > <$t>::zero() {
                    quotient.checked_add(<$t>::one())
                } else {
                    Some(quotient)
                }
            }
        }
    };
}

// checked_impl!(U256);
// checked_impl!(U192);
checked_impl!(u128);
checked_impl!(u64);
checked_impl!(u32);
checked_impl!(u16);
checked_impl!(u8);
checked_impl!(i128);
checked_impl!(i64);
checked_impl!(i32);
checked_impl!(i16);
checked_impl!(i8);