use std::io::Write;

use anchor_lang::{prelude::*, Discriminator};
use borsh::{BorshDeserialize, BorshSerialize};

use crate::{casting::Cast, errors::{DexError, VortexDexResult}, safe_methods::SafeUnwrap};

use super::{position::PositionDirection, user::{MarketType, Order}};

#[event]
pub struct NewUserAccountRecord {
    pub ts: i64,
    pub user_authority: Pubkey,
    pub user: Pubkey,
    pub name: [u8; 32],
}


#[event]
pub struct SpotInterestRecord {
    pub ts: i64,
    pub market_index: u16,
    /// precision: SPOT_BALANCE_PRECISION
    pub deposit_balance: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub cumulative_deposit_interest: u128,
    /// precision: SPOT_BALANCE_PRECISION
    pub borrow_balance: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub cumulative_borrow_interest: u128,
    /// precision: PERCENTAGE_PRECISION
    pub optimal_utilization: u32,
    /// precision: PERCENTAGE_PRECISION
    pub optimal_borrow_rate: u32,
    /// precision: PERCENTAGE_PRECISION
    pub max_borrow_rate: u32,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
pub enum DepositDirection {
    #[default]
    Deposit,
    Withdraw,
}

#[event]
pub struct DepositRecord {
    /// unix_timestamp of action
    pub ts: i64,
    pub user_authority: Pubkey,
    /// user account public key
    pub user: Pubkey,
    pub direction: DepositDirection,
    pub deposit_record_id: u64,
    /// precision: token mint precision
    pub amount: u64,
    /// spot market index
    pub market_index: u16,
    /// precision: PRICE_PRECISION
    pub oracle_price: i64,
    /// precision: SPOT_BALANCE_PRECISION
    pub market_deposit_balance: u128,
    /// precision: SPOT_BALANCE_PRECISION
    pub market_withdraw_balance: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub market_cumulative_deposit_interest: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub market_cumulative_borrow_interest: u128,
    /// precision: QUOTE_PRECISION
    pub total_deposits_after: u64,
    /// precision: QUOTE_PRECISION
    pub total_withdraws_after: u64,
    pub explanation: DepositExplanation,
    pub transfer_user: Option<Pubkey>,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
pub enum DepositExplanation {
    #[default]
    None,
    Transfer,
    Borrow,
    RepayBorrow,
}


#[event]
#[derive(Default)]
pub struct InsuranceFundStakeRecord {
    pub ts: i64,
    pub user_authority: Pubkey,
    pub action: StakeAction,
    /// precision: token mint precision
    pub amount: u64,
    pub market_index: u16,

    /// precision: token mint precision
    pub insurance_vault_amount_before: u64,
    pub if_shares_before: u128,
    pub user_if_shares_before: u128,
    pub total_if_shares_before: u128,
    pub if_shares_after: u128,
    pub user_if_shares_after: u128,
    pub total_if_shares_after: u128,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
pub enum StakeAction {
    #[default]
    Stake,
    UnstakeRequest,
    UnstakeCancelRequest,
    Unstake,
    UnstakeTransfer,
    StakeTransfer,
}


#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
pub enum OrderAction {
    #[default]
    Place,
    Cancel,
    Fill,
    Trigger,
    Expire,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub enum OrderActionExplanation {
    None,
    InsufficientFreeCollateral,
    OraclePriceBreachedLimitPrice,
    MarketOrderFilledToLimitPrice,
    OrderExpired,
    Liquidation,
    OrderFilledWithAMM,
    OrderFilledWithAMMJit,
    OrderFilledWithMatch,
    OrderFilledWithMatchJit,
    MarketExpired,
    RiskingIncreasingOrder,
    ReduceOnlyOrderIncreasedPosition,
    OrderFillWithSerum,
    NoBorrowLiquidity,
    OrderFillWithPhoenix,
    OrderFilledWithAMMJitLPSplit,
    OrderFilledWithLPJit,
    DeriskLp,
    OrderFilledWithOpenbookV2,
}

#[event]
#[derive(Default)]
pub struct SwapRecord {
    pub ts: i64,
    pub user: Pubkey,
    /// precision: out market mint precision
    pub amount_out: u64,
    /// precision: in market mint precision
    pub amount_in: u64,
    pub out_market_index: u16,
    pub in_market_index: u16,
    /// precision: PRICE_PRECISION
    pub out_oracle_price: i64,
    /// precision: PRICE_PRECISION
    pub in_oracle_price: i64,
    pub fee: u64,
}

#[event]
pub struct SpotMarketVaultDepositRecord {
    pub ts: i64,
    pub market_index: u16,
    /// precision: SPOT_BALANCE_PRECISION
    pub deposit_balance: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub cumulative_deposit_interest_before: u128,
    /// precision: SPOT_CUMULATIVE_INTEREST_PRECISION
    pub cumulative_deposit_interest_after: u128,
    pub deposit_token_amount_before: u64,
    pub amount: u64,
}


#[event]
#[derive(Default)]
pub struct LiquidationRecord {
    pub ts: i64,
    pub liquidation_type: LiquidationType,
    pub user: Pubkey,
    pub liquidator: Pubkey,
    pub margin_requirement: u128,
    pub total_collateral: i128,
    pub margin_freed: u64,
    pub liquidation_id: u16,
    pub bankrupt: bool,
    pub canceled_order_ids: Vec<u32>,
    pub liquidate_spot: LiquidateSpotRecord,
    pub spot_bankruptcy: SpotBankruptcyRecord,
}


#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct LiquidateSpotRecord {
    pub asset_market_index: u16,
    pub asset_price: i64,
    pub asset_transfer: u128,
    pub liability_market_index: u16,
    pub liability_price: i64,
    /// precision: token mint precision
    pub liability_transfer: u128,
    /// precision: token mint precision
    pub if_fee: u64,
}


#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct SpotBankruptcyRecord {
    pub market_index: u16,
    pub borrow_amount: u128,
    pub if_payment: u128,
    pub cumulative_deposit_interest_delta: u128,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
pub enum LiquidationType {
    #[default]
    LiquidateSpot,
    SpotBankruptcy,
}

#[event]
pub struct OrderRecord {
    pub ts: i64,
    pub user: Pubkey,
    pub order: Order,
}

impl OrderRecord {
    pub const SIZE: usize = 192;
}

#[event]
pub struct OrderActionRecord {
    pub ts: i64,
    pub action: OrderAction,
    pub action_explanation: OrderActionExplanation,
    pub market_index: u16,
    pub market_type: MarketType,

    pub filler: Option<Pubkey>,
    /// precision: QUOTE_PRECISION
    pub filler_reward: Option<u64>,
    pub fill_record_id: Option<u64>,

    /// precision: BASE_PRECISION (perp) or MINT_PRECISION (spot)
    pub base_asset_amount_filled: Option<u64>,
    /// precision: QUOTE_PRECISION
    pub quote_asset_amount_filled: Option<u64>,
    /// precision: QUOTE_PRECISION
    pub taker_fee: Option<u64>,
    /// precision: QUOTE_PRECISION
    pub maker_fee: Option<i64>,
    /// precision: QUOTE_PRECISION
    pub referrer_reward: Option<u32>,
    /// precision: QUOTE_PRECISION
    pub quote_asset_amount_surplus: Option<i64>,
    /// precision: QUOTE_PRECISION
    pub spot_fulfillment_method_fee: Option<u64>,

    pub taker: Option<Pubkey>,
    pub taker_order_id: Option<u32>,
    pub taker_order_direction: Option<PositionDirection>,
    /// precision: BASE_PRECISION (perp) or MINT_PRECISION (spot)
    pub taker_order_base_asset_amount: Option<u64>,
    /// precision: BASE_PRECISION (perp) or MINT_PRECISION (spot)
    pub taker_order_cumulative_base_asset_amount_filled: Option<u64>,
    /// precision: QUOTE_PRECISION
    pub taker_order_cumulative_quote_asset_amount_filled: Option<u64>,

    pub maker: Option<Pubkey>,
    pub maker_order_id: Option<u32>,
    pub maker_order_direction: Option<PositionDirection>,
    /// precision: BASE_PRECISION (perp) or MINT_PRECISION (spot)
    pub maker_order_base_asset_amount: Option<u64>,
    /// precision: BASE_PRECISION (perp) or MINT_PRECISION (spot)
    pub maker_order_cumulative_base_asset_amount_filled: Option<u64>,
    /// precision: QUOTE_PRECISION
    pub maker_order_cumulative_quote_asset_amount_filled: Option<u64>,

    /// precision: PRICE_PRECISION
    pub oracle_price: i64,
}

impl OrderActionRecord {
    pub const SIZE: usize = 384;
}

pub fn get_order_action_record(
    ts: i64,
    action: OrderAction,
    action_explanation: OrderActionExplanation,
    market_index: u16,
    filler: Option<Pubkey>,
    fill_record_id: Option<u64>,
    filler_reward: Option<u64>,
    base_asset_amount_filled: Option<u64>,
    quote_asset_amount_filled: Option<u64>,
    taker_fee: Option<u64>,
    maker_rebate: Option<u64>,
    referrer_reward: Option<u64>,
    quote_asset_amount_surplus: Option<i64>,
    spot_fulfillment_method_fee: Option<u64>,
    taker: Option<Pubkey>,
    taker_order: Option<Order>,
    maker: Option<Pubkey>,
    maker_order: Option<Order>,
    oracle_price: i64,
) -> VortexDexResult<OrderActionRecord> {
    Ok(OrderActionRecord {
        ts,
        action,
        action_explanation,
        market_index,
        market_type: if let Some(taker_order) = taker_order {
            taker_order.market_type
        } else if let Some(maker_order) = maker_order {
            maker_order.market_type
        } else {
            return Err(DexError::InvalidOrder);
        },
        filler,
        filler_reward,
        fill_record_id,
        base_asset_amount_filled,
        quote_asset_amount_filled,
        taker_fee,
        maker_fee: match maker_rebate {
            Some(maker_rebate) => Some(-maker_rebate.cast()?),
            None => None,
        },
        referrer_reward: match referrer_reward {
            Some(referrer_reward) if referrer_reward > 0 => Some(referrer_reward.cast()?),
            _ => None,
        },
        quote_asset_amount_surplus,
        spot_fulfillment_method_fee,
        taker,
        taker_order_id: taker_order.map(|order| order.order_id),
        taker_order_direction: taker_order.map(|order| order.direction),
        taker_order_base_asset_amount: taker_order.map(|order| order.base_asset_amount),
        taker_order_cumulative_base_asset_amount_filled: taker_order
            .map(|order| order.base_asset_amount_filled),
        taker_order_cumulative_quote_asset_amount_filled: taker_order
            .as_ref()
            .map(|order| order.quote_asset_amount_filled),
        maker,
        maker_order_id: maker_order.map(|order| order.order_id),
        maker_order_direction: maker_order.map(|order| order.direction),
        maker_order_base_asset_amount: maker_order.map(|order| order.base_asset_amount),
        maker_order_cumulative_base_asset_amount_filled: maker_order
            .map(|order| order.base_asset_amount_filled),
        maker_order_cumulative_quote_asset_amount_filled: maker_order
            .map(|order| order.quote_asset_amount_filled),
        oracle_price,
    })
}

pub fn emit_stack<T: AnchorSerialize + Discriminator, const N: usize>(event: T) -> VortexDexResult {
    let mut data_buf = [0u8; N];
    let mut out_buf = [0u8; N];

    emit_buffers(event, &mut data_buf[..], &mut out_buf[..])
}

pub fn emit_buffers<T: AnchorSerialize + Discriminator>(
    event: T,
    data_buf: &mut [u8],
    out_buf: &mut [u8],
) -> VortexDexResult {
    let mut data_writer = std::io::Cursor::new(data_buf);
    data_writer
        .write_all(&<T as Discriminator>::discriminator())
        .safe_unwrap()?;
    borsh::to_writer(&mut data_writer, &event).safe_unwrap()?;
    let data_len = data_writer.position() as usize;

    let out_len = base64::encode_config_slice(
        &data_writer.into_inner()[0..data_len],
        base64::STANDARD,
        out_buf,
    );

    let msg_bytes = &out_buf[0..out_len];
    let msg_str = unsafe { std::str::from_utf8_unchecked(msg_bytes) };

    msg!(msg_str);

    Ok(())
}


