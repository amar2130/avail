use self::config::CustomClientConfig;
use crate::toolkit;
use anyhow::{anyhow, Context, Result};
use bip39::Mnemonic;
use cosmrs::{
	bip32,
	crypto::{secp256k1::SigningKey, PublicKey},
	proto::{
		cosmos::{
			auth::v1beta1::{
				query_client::QueryClient, BaseAccount, QueryAccountRequest, QueryAccountResponse,
			},
			tx::v1beta1::{service_client::ServiceClient, BroadcastMode, BroadcastTxRequest},
		},
		cosmwasm::wasm::v1::MsgExecuteContract,
		traits::Message,
	},
	tx::{Body, Fee, MessageExt, SignDoc, SignerInfo},
	Coin,
};
use kate_recovery::com::AppData;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tokio::sync::mpsc::{Receiver, Sender};
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

impl CustomClient {
	pub async fn new(cfg: CustomClientConfig) -> Result<Self> {
		let Ok(mut query_client) = QueryClient::connect(cfg.node_host.clone()).await else {
		    return Err(anyhow!("Cannot connect to the cosmos node"));
		};

		let sender_private_key = private_key(&cfg.sender_mnemonic, &cfg.sender_password)?;
		let address = account_id(sender_private_key.public_key())?;

		let request = QueryAccountRequest { address };
		let response = query_client.account(request).await?;
		let sequence = sequence(response.into_inner())?;

		info!("Sequence number: {sequence}");

		Ok(CustomClient { cfg, sequence })
	}

	async fn process_block(&self, data: AppData) -> Result<Vec<u8>> {
		let CustomClientConfig {
			chain_id,
			contract,
			sender_mnemonic,
			sender_password,
			sender_account_number,
			..
		} = &self.cfg;

		let transfers = toolkit::decode_json_app_data(data)?;

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

		tx_signed.to_bytes().map_err(|error| anyhow!("{error}"))
	}

	pub async fn run(
		&mut self,
		mut app_rx: Receiver<AppData>,
		error_sender: Sender<anyhow::Error>,
	) {
		let Ok(mut client) = ServiceClient::connect(self.cfg.node_host.clone()).await else {
		    let message = "Cannot connect to the cosmos node";
		    if let Err(error) = error_sender.send(anyhow!(message)).await {
			error!("Cannot send error message: {error}");
		    }
		    return;
		};

		while let Some(block) = app_rx.recv().await {
			match self.process_block(block).await {
				Ok(tx_bytes) => {
					let mode = BroadcastMode::Block.into();
					let request = BroadcastTxRequest { tx_bytes, mode };

					if let Err(error) = &client.broadcast_tx(request).await {
						error!("{error}");
					}
				},
				Err(error) => error!("{error}"),
			}
			self.sequence += 1;
		}
	}
}
