use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    StdError(#[from] StdError),
    #[error("{addr} is not found")]
    NotFound { addr: String },
    #[error("Insufficient balance")]
    InsufficientBalance,
}

pub fn not_found(addr: String) -> ContractError {
    ContractError::NotFound { addr }
}
