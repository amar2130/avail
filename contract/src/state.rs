use cosmwasm_std::Uint128;
use cw_storage_plus::{Item, Map};

pub const ACCOUNTS: Item<Vec<String>> = Item::new("accounts");
pub const BALANCES: Map<String, Uint128> = Map::new("balances");
