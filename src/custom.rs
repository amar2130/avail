use self::{config::CustomClientConfig, types::Transfer};
use anyhow::{anyhow, Context, Result};
use bip39::Mnemonic;
use cosmrs::{
	bip32,
	crypto::{secp256k1::SigningKey, PublicKey},
	proto::{
		cosmos::{
			auth::v1beta1::{
				query_client::QueryClient as AuthQueryClient, BaseAccount, QueryAccountRequest,
				QueryAccountResponse,
			},
			tx::v1beta1::{
				service_client::ServiceClient, BroadcastMode, BroadcastTxRequest, SimulateRequest,
			},
		},
		cosmwasm::wasm::v1::{
			query_client::QueryClient, MsgExecuteContract, QuerySmartContractStateRequest,
		},
		traits::Message,
	},
	tx::{Body, Fee, MessageExt, SignDoc, SignerInfo},
	Coin,
};
use kate_recovery::com::AppData;
use serde::{Deserialize, Serialize};
use sp_core::hashing::sha2_256;
use sp_keyring::AccountKeyring;
use std::{str::FromStr, sync::Arc, time::Duration};
use subxt::OnlineClient;
use tokio::{
	sync::{mpsc::Receiver, Mutex as AsyncMutex},
	time,
};
use tonic::transport::Channel;
use tracing::{error, info};

pub mod types {
	use serde::{Deserialize, Serialize};

	#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
	pub struct Transfer {
		pub from: String,
		pub to: String,
		pub amount: String,
	}

	#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
	pub enum ExecuteTransfers {
		TransferBalance { transfers: Vec<Transfer> },
	}
}

pub mod config {
	use crate::types::RuntimeConfig;

	pub struct CustomClientConfig {
		pub node_host: String,
		pub chain_id: String,
		pub contract: String,
		pub sender_mnemonic: String,
		pub sender_password: String,
		pub sender_account_number: u64,
	}

	impl From<&RuntimeConfig> for CustomClientConfig {
		fn from(value: &RuntimeConfig) -> Self {
			Self {
				node_host: value.node_host.clone(),
				chain_id: value.chain_id.clone(),
				contract: value.contract.clone(),
				sender_mnemonic: value.sender_mnemonic.clone(),
				sender_password: value.sender_password.clone(),
				sender_account_number: value.sender_account_number,
			}
		}
	}
}

// TODO:
// - check balance on smart contract in cosmos

fn private_key(mnemonic: &str, password: &str) -> Result<SigningKey> {
	let mnemonic = Mnemonic::parse(mnemonic)?;
	let seed = mnemonic.to_seed(password);
	let derivation_path = bip32::DerivationPath::from_str("m/44'/118'/0'/0/0")?;
	let signing_key = SigningKey::derive_from_path(seed, &derivation_path)?;
	Ok(signing_key)
}

fn account_id(public_key: PublicKey) -> Result<String> {
	public_key
		.account_id("wasm")
		.map(|account| account.to_string())
		.map_err(|error| anyhow!("{error}"))
}

fn sequence(response: QueryAccountResponse) -> Result<u64> {
	let value = &response.account.context("Account not found")?.value[..];
	let account = BaseAccount::decode(value)?;
	Ok(account.sequence)
}

pub struct CustomClient {
	cfg: CustomClientConfig,
	sequence: u64,
	service_client: ServiceClient<Channel>,
	query_client: QueryClient<Channel>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum QueryMsg {
	Balances {},
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum PostAppData {
	Balances(Balances),
	Error(String),
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Balances {
	pub balances: Vec<(String, String)>,
}

pub async fn run(mut app_receiver: Receiver<AppData>) {
	while let Some(_app_data) = app_receiver.recv().await {
		info!("App data received");
	}
}

impl CustomClient {
	pub async fn new(cfg: CustomClientConfig) -> Result<Self> {
		let Ok(service_client) = ServiceClient::connect(cfg.node_host.clone()).await else {
		    return Err(anyhow!("Cannot connect to the cosmos node"));
		};

		let Ok(query_client) = QueryClient::connect(cfg.node_host.clone()).await else {
		    return Err(anyhow!("Cannot connect to the cosmos node"));
		};

		let Ok(mut account_query_client) = AuthQueryClient::connect(cfg.node_host.clone()).await else {
		    return Err(anyhow!("Cannot connect to the cosmos node"));
		};
		let sender_private_key = private_key(&cfg.sender_mnemonic, &cfg.sender_password)?;
		let address = account_id(sender_private_key.public_key())?;

		let request = QueryAccountRequest { address };
		let response = account_query_client.account(request).await?;
		let sequence = sequence(response.into_inner())?;

		info!("Current sequence number: {sequence}");

		Ok(CustomClient {
			cfg,
			sequence,
			service_client,
			query_client,
		})
	}

	pub async fn query_state(&mut self) -> Result<Balances> {
		let query_data = serde_json::to_vec(&QueryMsg::Balances {}).unwrap();
		let request = QuerySmartContractStateRequest {
			address: self.cfg.contract.clone(),
			query_data,
		};

		let query_response = self.query_client.smart_contract_state(request);
		let response = query_response.await?.into_inner();
		serde_json::from_slice(&response.data).map_err(|error| anyhow!("{error}"))
	}

	fn execute_transfers_tx(&self, transfers: Vec<Transfer>) -> Result<Vec<u8>> {
		let CustomClientConfig {
			chain_id,
			contract,
			sender_mnemonic,
			sender_password,
			sender_account_number,
			..
		} = &self.cfg;

		// NOTE: We cannot pass SigningKey to async fn due to missing Send marker
		let sender_private_key = private_key(sender_mnemonic, sender_password)?;
		let sender_public_key = sender_private_key.public_key();
		let sender_account_id = account_id(sender_public_key)?;

		let msg = serde_json::to_vec(&types::ExecuteTransfers::TransferBalance { transfers })?;

		let execute_msg = MsgExecuteContract {
			sender: sender_account_id,
			contract: contract.clone(),
			msg,
			funds: vec![],
		};

		let memo = "";
		let timeout_height = 9001u16;
		let tx_body = Body::new(vec![execute_msg.to_any()?], memo, timeout_height);

		// Signing
		let gas = 500_000u64;
		let amount = Coin::new(0, "atom").map_err(|error| anyhow!("{error}"))?;
		let signer_info = SignerInfo::single_direct(Some(sender_public_key), self.sequence);
		let auth_info = signer_info.auth_info(Fee::from_amount_and_gas(amount, gas));
		let sign_doc = SignDoc::new(
			&tx_body,
			&auth_info,
			&chain_id.parse()?,
			*sender_account_number,
		)
		.map_err(|error| anyhow!("{error}"))?;

		let tx_signed = sign_doc
			.sign(&sender_private_key)
			.map_err(|error| anyhow!("{error}"))?;

		tx_signed
			.to_bytes()
			.map(|bytes| {
				info!("TXHASH: {}", hex::encode(sha2_256(&bytes)));
				bytes
			})
			.map_err(|error| anyhow!("{error}"))
	}

	pub async fn simulate(&mut self, transfers: Vec<Transfer>) -> Result<Balances> {
		let tx_bytes = self.execute_transfers_tx(transfers)?;
		#[allow(deprecated)]
		let simulate_request = SimulateRequest { tx: None, tx_bytes };
		let response = self.service_client.simulate(simulate_request).await?;

		// TODO: Decode data properly (proto doesn't work)
		let data = response.into_inner().result.context("No data found")?.data;
		let data = String::from_utf8(data)?;
		let data = if data.contains('-') {
			*data.split('-').collect::<Vec<_>>().last().unwrap()
		} else {
			*data.split('.').collect::<Vec<_>>().last().unwrap()
		};
		let balances: Balances = serde_json::from_str(data)?;
		Ok(balances)
	}

	pub async fn broadcast(&mut self, transfers: Vec<Transfer>) -> Result<()> {
		let tx_bytes = self.execute_transfers_tx(transfers)?;
		let mode = BroadcastMode::Block.into();
		let request = BroadcastTxRequest { tx_bytes, mode };
		self.service_client.broadcast_tx(request).await?;
		self.sequence += 1;
		Ok(())
	}
}

pub struct CustomSequencer {
	pub state: Arc<AsyncMutex<Vec<Transfer>>>,
	pub custom_client: Arc<AsyncMutex<CustomClient>>,
	pub da_client: OnlineClient<AvailConfig>,
}

impl CustomSequencer {
	async fn broadcast(&self, transfers: Vec<Transfer>) -> Result<()> {
		let mut custom_client = self.custom_client.lock().await;
		custom_client.broadcast(transfers).await
	}

	async fn da_submit(&self, transfers: Vec<Transfer>) -> Result<()> {
		let signer = PairSigner::new(AccountKeyring::Alice.pair());
		let app_id = 1;

		let da_client = self.da_client.clone();

		let msg = serde_json::to_vec(&types::ExecuteTransfers::TransferBalance { transfers })?;

		let data_transfer = api::tx()
			.data_availability()
			.submit_data(BoundedVec(msg.clone()));
		let extrinsic_params = AvailExtrinsicParams::new_with_app_id(app_id.into());

		let _ = da_client
			.tx()
			.sign_and_submit_then_watch(&data_transfer, &signer, extrinsic_params)
			.await?
			.wait_for_finalized_success()
			.await?;

		Ok(())
	}

	pub async fn run(&self) -> ! {
		let mut interval = time::interval(Duration::from_secs(20));
		loop {
			interval.tick().await;

			let transfers = {
				let mut state = self.state.lock().await;
				let transfers = state.drain(0..).collect::<Vec<_>>();
				if let Err(error) = self.broadcast(transfers.clone()).await {
					error!("{error}");
					continue;
				};
				info!("Transfers submitted to the node");
				transfers
			};

			if let Err(error) = self.da_submit(transfers.clone()).await {
				error!("{error}");
				continue;
			}

			info!("Transfers submitted to DA");
		}
	}
}
