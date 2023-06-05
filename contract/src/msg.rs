use cosmwasm_std::Uint128;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct InstantiateMsg {
    pub balances: Vec<(String, u128)>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Transfer {
    pub from: String,
    pub to: String,
    pub amount: Uint128,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum ExecuteMsg {
    TransferBalance { transfers: Vec<Transfer> },
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct BalancesResp {
    pub balances: Vec<(String, u128)>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum QueryMsg {
    Balances {},
}
