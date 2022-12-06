use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Decimal, Uint128};

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Option<String>,
    pub required_native_denom: String,
    pub fee: Decimal,
}

#[cw_serde]
pub enum ExecuteMsg {
    Bid {},
    Close {},
    Retract { friend_rec: Option<String> },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Uint128)]
    BidderTotalBid { address: String },
    #[returns(BidEventInfoResponse)]
    HighestBidInfo {},
    #[returns(Uint128)]
    TotalNumberOfParticipants {},
}

#[cw_serde]
pub struct BidEventInfoResponse {
    pub addr: Option<Addr>,
    pub bid_amount: Option<Uint128>,
    pub event_closed: bool,
}
