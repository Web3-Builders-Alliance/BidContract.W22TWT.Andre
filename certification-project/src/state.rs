use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub required_native_denom: String,
    pub fee: Decimal,
    pub open_sale: bool,
}

pub const OWNER: Item<Addr> = Item::new("owner");

pub const CONFIG: Item<Config> = Item::new("config");

pub const ALL_BIDS_PER_BIDDER: Map<Addr, Uint128> = Map::new("all_bids");

pub const HIGHEST_CURRENT_BID: Item<(Addr, Uint128)> = Item::new("highest_current_bid");
