use anchor_lang::prelude::Pubkey;

#[derive(Debug)]
pub enum SpotFulfillmentMethod {
    ExternalMarket,
    Match(Pubkey, u16),
}
