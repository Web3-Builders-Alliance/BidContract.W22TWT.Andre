use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Bid event closed")]
    BidEventClosed {},

    #[error("You never participated...")]
    NoFundsToRetract {},

    #[error("Wrong token to bid")]
    WrongToken {},

    #[error("Bid amount is insufficient")]
    BidAmountInsuf {},

    #[error("Amount already retracted once")]
    AlreadyRetracted {},
}
