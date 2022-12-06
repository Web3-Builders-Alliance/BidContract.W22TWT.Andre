#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_binary, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult, Uint128, Uint256,
};
use cw_utils::must_pay;
// use cw2::set_contract_version;

use crate::error::ContractError;
use crate::msg::{BidEventInfoResponse, ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{Config, ALL_BIDS_PER_BIDDER, CONFIG, HIGHEST_CURRENT_BID, OWNER};

/*
// version info for migration info
const CONTRACT_NAME: &str = "crates.io:certification-project";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
*/

const FEE_SCALE_FACTOR: Uint128 = Uint128::new(10_000);
const _MAX_FEE_PERCENT: &str = "1";
const FEE_DECIMAL_PRECISION: Uint128 = Uint128::new(10u128.pow(20));

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    // set owner add, checking Option in the msg, or default to sender
    let owner = msg
        .owner
        .map(|str_addr| deps.api.addr_validate(&str_addr))
        .transpose()?
        .unwrap_or_else(|| env.contract.address.clone());
    OWNER.save(deps.storage, &owner)?;

    // creates and saves config
    let config = Config {
        required_native_denom: msg.required_native_denom,
        fee: msg.fee,
        open_sale: true,
    };
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("sender", info.sender)
        .add_attribute("owner", owner))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Bid {} => do_bid(deps, info),
        ExecuteMsg::Close {} => close_bid_event(deps, info),
        ExecuteMsg::Retract { friend_rec } => retract(deps, info, friend_rec),
    }
}

pub fn retract(
    deps: DepsMut,
    info: MessageInfo,
    friend_rec: Option<String>,
) -> Result<Response, ContractError> {
    // validate bid event is closed
    let config = CONFIG.load(deps.storage)?;
    if config.open_sale {
        return Err(ContractError::BidEventClosed {});
    }

    // validate requester is not the winning addr
    let (ad, _) = HIGHEST_CURRENT_BID.load(deps.storage)?;
    if ad == info.sender {
        return Err(ContractError::Unauthorized {});
    }
    // check if friend_rec is filled
    let receiver_addr = friend_rec
        .map(|add| deps.api.addr_validate(&add))
        .transpose()?
        .unwrap_or_else(|| info.sender.clone());

    // validate if requester have founds to withdraw

    let mut amount_to_send = Uint128::zero();

    ALL_BIDS_PER_BIDDER.update(deps.storage, info.sender, |x| -> Result<_, ContractError> {
        match x {
            Some(mut amount) => {
                if amount > Uint128::zero() {
                    amount_to_send = amount;
                    amount = Uint128::zero();
                    Ok(amount)
                } else {
                    Err(ContractError::AlreadyRetracted {})
                }
            }
            None => Err(ContractError::NoFundsToRetract {}),
        }
    })?;

    let paid_fee = get_owner_fee_amount(amount_to_send, config.fee)?;

    amount_to_send -= paid_fee;

    // message to retract funds
    let withdraw_msg: CosmosMsg = CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
        to_address: receiver_addr.into_string(),
        amount: vec![Coin {
            denom: config.required_native_denom,
            amount: amount_to_send,
        }],
    });

    Ok(Response::new().add_message(withdraw_msg))
}

pub fn close_bid_event(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    // validate that it's the owner trying to close the bid event
    let owner = OWNER.load(deps.storage)?;
    if owner != info.sender {
        return Err(ContractError::Unauthorized {});
    }

    // validate that bid event is still open
    let config = CONFIG.load(deps.storage)?;
    if !config.open_sale {
        return Err(ContractError::BidEventClosed {});
    }

    // close bid event in the state config
    CONFIG.update(deps.storage, |mut con| -> Result<Config, ContractError> {
        con.open_sale = false;
        Ok(con)
    })?;

    // calculate amount to send from highest bidder
    let (_, am) = HIGHEST_CURRENT_BID.load(deps.storage)?;
    let fee_amount = get_owner_fee_amount(am, config.fee)?;
    let amount_wo_fee = am - fee_amount;

    // msg move funds to owner
    let winner_bid_to_owner_msg: CosmosMsg = CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
        to_address: owner.into_string(),
        amount: vec![Coin {
            denom: config.required_native_denom,
            amount: amount_wo_fee,
        }],
    });

    Ok(Response::new().add_message(winner_bid_to_owner_msg))
}

pub fn do_bid(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    // check if bid event is still open
    let config = CONFIG.load(deps.storage)?;
    if !config.open_sale {
        return Err(ContractError::BidEventClosed {});
    }

    // find and check sent funds
    let paid = must_pay(&info, config.required_native_denom.as_str())
        .map_err(|_| ContractError::WrongToken {})?;

    // get highest bid
    let highest_bid: Uint128 = get_highest_bid(&deps)?;

    // find and check sender existing bid in state
    let total_user_bid = match ALL_BIDS_PER_BIDDER.may_load(deps.storage, info.sender.clone()) {
        Ok(amount) => match amount {
            Some(a) => a,
            None => Uint128::zero(),
        },
        Err(_) => {
            return Err(ContractError::Std(StdError::GenericErr {
                msg: "std error".to_string(),
            }))
        }
    };

    // check if sender bid is inferior to the current highest bid
    if highest_bid >= total_user_bid + paid {
        return Err(ContractError::BidAmountInsuf {});
    }

    // save/update bid
    ALL_BIDS_PER_BIDDER.update(
        deps.storage,
        info.sender.clone(),
        |bid| -> Result<_, ContractError> {
            match bid {
                Some(mut amount) => {
                    amount += paid;

                    Ok(amount)
                }
                None => Ok(paid),
            }
        },
    )?;

    // set new current winning bid
    HIGHEST_CURRENT_BID.save(deps.storage, &(info.sender, total_user_bid + paid))?;

    // TOOK THIS FROM WASMSWAP REPO! I DON'T HAVE A CLUE HOW MATH WORKS IN RUST! SORRY!
    let fee_amount = get_owner_fee_amount(paid, config.fee)?;

    // if fee is 0 return here
    if fee_amount == Uint128::zero() {
        return Ok(Response::new().add_attribute("fee", "0"));
    }

    // msg to send fee to owner that is greater than zero
    let owner = OWNER.load(deps.storage)?;
    let fee_to_owner_msg: CosmosMsg = CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
        to_address: owner.into_string(),
        amount: vec![Coin {
            denom: config.required_native_denom,
            amount: fee_amount,
        }],
    });

    Ok(Response::new()
        .add_message(fee_to_owner_msg)
        .add_attribute("fee", fee_amount.to_string()))
}

// provide the stored highest bid amount, or zero if it is first bid
fn get_highest_bid(deps: &DepsMut) -> Result<Uint128, ContractError> {
    let hb = HIGHEST_CURRENT_BID.load(deps.storage);

    match hb {
        Ok(highest_bid) => Ok(highest_bid.1),
        Err(_) => Ok(Uint128::zero()),
    }
}

fn get_owner_fee_amount(input_amount: Uint128, fee_percent: Decimal) -> StdResult<Uint128> {
    if fee_percent.is_zero() {
        return Ok(Uint128::zero());
    }

    let fee_percent = fee_decimal_to_uint128(fee_percent)?;
    Ok(input_amount
        .full_mul(fee_percent)
        .checked_div(Uint256::from(FEE_SCALE_FACTOR))
        .map_err(StdError::divide_by_zero)?
        .try_into()?)
}

fn fee_decimal_to_uint128(decimal: Decimal) -> StdResult<Uint128> {
    let result: Uint128 = decimal
        .atomics()
        .checked_mul(FEE_SCALE_FACTOR)
        .map_err(StdError::overflow)?;

    Ok(result / FEE_DECIMAL_PRECISION)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::BidderTotalBid { address } => query_bidder_total_bid(deps, address),
        QueryMsg::HighestBidInfo {} => query_bid_event_info(deps),
        QueryMsg::TotalNumberOfParticipants {} => query_total_number_participants(deps),
    }
}

fn query_total_number_participants(deps: Deps) -> StdResult<Binary> {
    let number_of_participants = ALL_BIDS_PER_BIDDER
        .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending)
        .collect::<StdResult<Vec<_>>>()?
        .len();

    to_binary(&number_of_participants)
}

fn query_bid_event_info(deps: Deps) -> StdResult<Binary> {
    let config = CONFIG.load(deps.storage)?;
    let (hi_bid_add, hi_bid_amount) = HIGHEST_CURRENT_BID.load(deps.storage)?;

    let resp = BidEventInfoResponse {
        addr: Some(hi_bid_add),
        bid_amount: Some(hi_bid_amount),
        event_closed: !config.open_sale,
    };

    to_binary(&resp)
}

fn query_bidder_total_bid(deps: Deps, address: String) -> StdResult<Binary> {
    let valide_addr = match deps.api.addr_validate(&address) {
        Ok(addr) => addr,
        Err(_) => return to_binary(&Uint128::zero()),
    };

    let bid_amount = match ALL_BIDS_PER_BIDDER.may_load(deps.storage, valide_addr)? {
        Some(amount) => amount,
        None => Uint128::zero(),
    };

    to_binary(&bid_amount)
}

#[cfg(test)]
mod tests {

    use std::{borrow::BorrowMut, str::FromStr};

    use cosmwasm_std::{coins, Addr, Coin, Decimal, Empty, Uint128};
    use cw_multi_test::{App, Contract, ContractWrapper, Executor};

    use crate::msg::{BidEventInfoResponse, ExecuteMsg, InstantiateMsg};

    use super::{execute, instantiate, query};

    const OWNER: &str = "owner1";
    const BIDDER1: &str = "bidder1";
    const BIDDER2: &str = "bidder2";
    const USED_DENOM: &str = "Juno";
    fn bid_festival_contract() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(execute, instantiate, query);
        Box::new(contract)
    }

    fn bank_balance(router: &mut App, addr: &Addr, denom: String) -> Coin {
        router
            .wrap()
            .query_balance(addr.to_string(), denom)
            .unwrap()
    }

    #[test]
    fn test_instantiate() {
        let mut app = App::default();

        // set initial balances for owner and check balance
        let funds = coins(20000, USED_DENOM);
        app.borrow_mut().init_modules(|router, _, storage| {
            router
                .bank
                .init_balance(storage, &Addr::unchecked(OWNER), funds.clone())
                .unwrap()
        });
        let balance: Coin = bank_balance(&mut app, &Addr::unchecked(OWNER), USED_DENOM.to_string());
        assert_eq!(balance.amount, Uint128::new(20000));

        // set initial balances for Bidder1 and check balance
        app.borrow_mut().init_modules(|router, _, storage| {
            router
                .bank
                .init_balance(storage, &Addr::unchecked(BIDDER1), funds.clone())
                .unwrap()
        });
        let balance: Coin =
            bank_balance(&mut app, &Addr::unchecked(BIDDER1), USED_DENOM.to_string());
        assert_eq!(balance.amount, Uint128::new(20000));

        // set initial balances for Bidder2 and check balance
        app.borrow_mut().init_modules(|router, _, storage| {
            router
                .bank
                .init_balance(storage, &Addr::unchecked(BIDDER2), funds)
                .unwrap()
        });
        let balance: Coin =
            bank_balance(&mut app, &Addr::unchecked(BIDDER2), USED_DENOM.to_string());
        assert_eq!(balance.amount, Uint128::new(20000));

        // store and instantiate contract
        let festival_code = app.store_code(bid_festival_contract());
        let festival = app
            .instantiate_contract(
                festival_code,
                Addr::unchecked(OWNER),
                &InstantiateMsg {
                    owner: Some(OWNER.to_string()),
                    required_native_denom: USED_DENOM.into(),
                    fee: Decimal::from_str("3").unwrap(),
                },
                &[],
                "biddromedo",
                None,
            )
            .unwrap();

        // execute and check bid BIDDER1
        app.execute_contract(
            Addr::unchecked(BIDDER1),
            festival.clone(),
            &ExecuteMsg::Bid {},
            &[Coin {
                denom: USED_DENOM.to_string(),
                amount: Uint128::new(1000),
            }],
        )
        .unwrap();

        let highest_bid: BidEventInfoResponse = app
            .wrap()
            .query_wasm_smart(festival.clone(), &crate::msg::QueryMsg::HighestBidInfo {})
            .unwrap();
        assert_eq!(highest_bid.bid_amount, Some(Uint128::new(1000)));
        assert_eq!(highest_bid.addr, Some(Addr::unchecked(BIDDER1)));
        assert!(!highest_bid.event_closed);

        // check if owner is receiving fees (1000*0.03) = 30
        let balance: Coin = bank_balance(&mut app, &Addr::unchecked(OWNER), USED_DENOM.to_string());
        assert_eq!(balance.amount, Uint128::new(20030));

        // execute and check bid BIDDER2
        app.execute_contract(
            Addr::unchecked(BIDDER2),
            festival.clone(),
            &ExecuteMsg::Bid {},
            &[Coin {
                denom: USED_DENOM.to_string(),
                amount: Uint128::new(1500),
            }],
        )
        .unwrap();

        let highest_bid: BidEventInfoResponse = app
            .wrap()
            .query_wasm_smart(festival.clone(), &crate::msg::QueryMsg::HighestBidInfo {})
            .unwrap();
        assert_eq!(highest_bid.bid_amount, Some(Uint128::new(1500)));
        assert_eq!(highest_bid.addr, Some(Addr::unchecked(BIDDER2)));
        assert!(!highest_bid.event_closed);

        // check if owner is receiving fees (1500*0.03) = 45
        let balance: Coin = bank_balance(&mut app, &Addr::unchecked(OWNER), USED_DENOM.to_string());
        assert_eq!(balance.amount, Uint128::new(20075));

        // ececute close event and check owner receive fees + winning bid (30 from 1st bid + 2nd bid)
        app.execute_contract(
            Addr::unchecked(OWNER),
            festival.clone(),
            &ExecuteMsg::Close {},
            &[],
        )
        .unwrap();
        let balance: Coin = bank_balance(&mut app, &Addr::unchecked(OWNER), USED_DENOM.to_string());
        assert_eq!(balance.amount, Uint128::new(21530));

        // check contract balance. It should be 1st bid - 1st bid fee (already sent to owner)
        let balance: Coin = bank_balance(
            &mut app,
            &Addr::unchecked(festival.clone()),
            USED_DENOM.to_string(),
        );
        assert_eq!(balance.amount, Uint128::new(970));

        // Bidder 1 retracts amount. Expects initial balance - paid fee
        app.execute_contract(
            Addr::unchecked(BIDDER1),
            festival,
            &ExecuteMsg::Retract { friend_rec: None },
            &[],
        )
        .unwrap();
        let balance: Coin =
            bank_balance(&mut app, &Addr::unchecked(BIDDER1), USED_DENOM.to_string());
        assert_eq!(balance.amount, Uint128::new(19970));
    }
}
