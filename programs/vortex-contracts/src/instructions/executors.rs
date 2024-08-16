use anchor_lang::prelude::*;

#[derive(Clone, Copy, AnchorSerialize, AnchorDeserialize, PartialEq, Debug, Eq, Default)]
pub enum SpotFulfillmentType {
    #[default]
    SerumV3,
    Match,
}