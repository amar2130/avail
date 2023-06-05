use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{ACCOUNTS, BALANCES};
use cosmwasm_std::{to_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult};

pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    let accounts = msg.balances.iter().map(|b| b.0.clone()).collect::<Vec<_>>();
    ACCOUNTS.save(deps.storage, &accounts)?;
    for (key, value) in msg.balances {
        BALANCES.save(deps.storage, key, &value.into())?;
    }

    Ok(Response::new())
}

pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    use QueryMsg::*;

    match msg {
        Balances {} => to_binary(&query::balances(deps)?),
    }
}

pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    use ExecuteMsg::*;

    match msg {
        TransferBalance { transfers } => exec::transfer(deps, info, transfers),
    }
}

mod exec {
    use super::*;
    use crate::{error, msg::Transfer};
    use cosmwasm_std::{DepsMut, MessageInfo, Response};

    pub fn transfer(
        deps: DepsMut,
        _info: MessageInfo,
        transfers: Vec<Transfer>,
    ) -> Result<Response, ContractError> {
        let accounts = ACCOUNTS.load(deps.storage)?;

        deps.api.debug(&format!("ACCOUNTS: {accounts:?}"));

        for transfer in transfers {
            if !accounts.contains(&transfer.from) {
                return Err(error::not_found(transfer.from));
            }
            if !accounts.contains(&transfer.to) {
                return Err(error::not_found(transfer.to));
            }
            let mut balance_from = BALANCES.load(deps.storage, transfer.from.clone())?;
            let mut balance_to = BALANCES.load(deps.storage, transfer.to.clone())?;

            deps.api.debug(&format!(
                "BALANCE from: {balance_from}, BALANCE to: {balance_to}"
            ));
            if balance_from < transfer.amount {
                return Err(ContractError::InsufficientBalance);
            }

            balance_from = balance_from.saturating_sub(transfer.amount);
            balance_to = balance_to.saturating_add(transfer.amount);

            BALANCES.save(deps.storage, transfer.from, &balance_from)?;
            BALANCES.save(deps.storage, transfer.to, &balance_to)?;
        }

	let balances = query::balances(deps.as_ref())?;
        Ok(Response::new().set_data(to_binary(&balances)?))
    }
}

mod query {
    use crate::msg::BalancesResp;

    use super::*;

    pub fn balances(deps: Deps) -> StdResult<BalancesResp> {
        let accounts = ACCOUNTS.load(deps.storage)?;
        let mut balances = Vec::new();
        for account in accounts {
            let balance = BALANCES.load(deps.storage, account.clone())?;
            balances.push((account, balance.into()));
        }
        Ok(BalancesResp { balances })
    }
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{from_binary, Addr, Uint128};
    use cw_multi_test::{App, ContractWrapper, Executor};

    use crate::msg::{BalancesResp, Transfer};

    use super::*;

    fn mock_app() -> (App, u64) {
        let mut app = App::default();
        let code = ContractWrapper::new(execute, instantiate, query);
        let code_id = app.store_code(Box::new(code));
        (app, code_id)
    }

    fn mock_contract(app: &mut App, code_id: u64, balances: &Vec<(String, u128)>) -> Addr {
        let owner = Addr::unchecked("owner");
        let balances = balances.to_owned();
        let msg = InstantiateMsg { balances };
        app.instantiate_contract(code_id, owner, &msg, &[], "contract", None)
            .unwrap()
    }

    fn query_balances(app: &App, addr: Addr) -> BalancesResp {
        app.wrap()
            .query_wasm_smart(addr, &QueryMsg::Balances {})
            .unwrap()
    }

    #[test]
    fn instantiation() {
        let (mut app, code_id) = mock_app();

        let balances = vec![];
        let addr = mock_contract(&mut app, code_id, &balances);
        assert_eq!(query_balances(&app, addr), BalancesResp { balances });

        let balances = vec![("a".to_string(), 1), ("b".to_string(), 10)];
        let addr = mock_contract(&mut app, code_id, &balances);
        assert_eq!(query_balances(&app, addr), BalancesResp { balances });
    }

    #[test]
    fn no_transfers() {
        let (mut app, code_id) = mock_app();

        let balances = vec![];
        let user = Addr::unchecked("user");
        let addr = mock_contract(&mut app, code_id, &balances);
        let msg = &ExecuteMsg::TransferBalance { transfers: vec![] };

        let response = app.execute_contract(user, addr, msg, &[]).unwrap();
        let data: BalancesResp = from_binary(&response.data.unwrap()).unwrap();
        assert_eq!(data, BalancesResp { balances: vec![] });
    }

    #[test]
    fn transfers_not_found() {
        let (mut app, code_id) = mock_app();

        let balances = vec![("a".to_string(), 1), ("b".to_string(), 10)];
        let user = Addr::unchecked("user");
        let addr = mock_contract(&mut app, code_id, &balances);
        let transfer = Transfer {
            from: "a".to_string(),
            to: "c".to_string(),
            amount: Uint128::new(1),
        };
        let msg = &ExecuteMsg::TransferBalance {
            transfers: vec![transfer],
        };

        let error = app.execute_contract(user, addr, msg, &[]).unwrap_err();

        assert_eq!(
            ContractError::NotFound {
                addr: "c".to_string()
            },
            error.downcast().unwrap(),
        );
    }

    #[test]
    fn transfers_insufficient_balance() {
        let (mut app, code_id) = mock_app();

        let balances = vec![("a".to_string(), 1), ("b".to_string(), 10)];
        let user = Addr::unchecked("user");
        let addr = mock_contract(&mut app, code_id, &balances);
        let transfer = Transfer {
            from: "a".to_string(),
            to: "b".to_string(),
            amount: Uint128::new(1),
        };
        let msg = &ExecuteMsg::TransferBalance {
            transfers: vec![transfer],
        };

        app.execute_contract(user.clone(), addr.clone(), msg, &[])
            .unwrap();
        let error = app.execute_contract(user, addr, msg, &[]).unwrap_err();

        assert_eq!(
            ContractError::InsufficientBalance,
            error.downcast().unwrap(),
        );
    }

    #[test]
    fn transfers() {
        let (mut app, code_id) = mock_app();

        let balances = vec![("a".to_string(), 9), ("b".to_string(), 1)];
        let user = Addr::unchecked("user");
        let addr = mock_contract(&mut app, code_id, &balances);
        let transfer = Transfer {
            from: "a".to_string(),
            to: "b".to_string(),
            amount: Uint128::new(4),
        };
        let msg = &ExecuteMsg::TransferBalance {
            transfers: vec![transfer],
        };

        let response = app.execute_contract(user, addr.clone(), msg, &[]).unwrap();
        let balances_resp: BalancesResp = from_binary(&response.data.unwrap()).unwrap();

        let expected = BalancesResp {
            balances: vec![("a".to_string(), 5), ("b".to_string(), 5)],
        };

        assert_eq!(balances_resp, expected);
        assert_eq!(query_balances(&app, addr), expected);
    }
}
