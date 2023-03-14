use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use avail_subxt::{
	api::runtime_types::{da_control::pallet::Call, da_runtime::RuntimeCall},
	primitives::AppUncheckedExtrinsic,
};
use bip39::Mnemonic;
use codec::Decode;
use cosmrs::{
	bip32,
	crypto::secp256k1::SigningKey,
	proto::{
		cosmos::tx::v1beta1::{service_client::ServiceClient, BroadcastMode, BroadcastTxRequest},
		cosmwasm::wasm::v1::MsgExecuteContract,
	},
	tx::{Body, Fee, MessageExt, SignDoc, SignerInfo},
	Coin,
};
use kate_recovery::com::AppData;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::error;

// mod custom
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Transfer {
	pub from: String,
	pub to: String,
	pub amount: String,
}

// mod custom
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum ExecuteTransfers {
	TransferBalance { transfers: Vec<Transfer> },
}

// mod toolkit
fn decode_app_data<T: for<'a> Deserialize<'a>>(data: AppData) -> Result<Vec<T>> {
	let xts: Vec<AppUncheckedExtrinsic> = data
		.iter()
		.enumerate()
		.map(|(i, raw)| {
			<_ as Decode>::decode(&mut &raw[..])
				.context(format!("Couldn't decode AvailExtrinsic num {i}"))
		})
		.collect::<Result<Vec<_>>>()?;

	xts.iter()
		.flat_map(|xt| match &xt.function {
			RuntimeCall::DataAvailability(Call::submit_data { data, .. }) => Some(data),
			_ => None,
		})
		.map(|data| serde_json::from_slice::<T>(&data.0).map_err(|error| anyhow!("{error}")))
		.collect::<Result<Vec<T>>>()
}

// TODO
// - remove menmonic and contract from commited code
// - get the account sequence
// - check balance on smart contract in cosmos
// - and submit tx to avail through avail-light client

// mod custom
pub struct CustomClient {
	pub node_host: String,
	pub chain_id: String,
	pub contract: String,
	pub sender_mnemonic: String,
	pub sender_password: String,
	pub sender_account_number: u64,
	pub sequence: u64,
}

// mod custom
impl CustomClient {
	fn sender_private_key(&self) -> Result<SigningKey> {
		let mnemonic = Mnemonic::parse(self.sender_mnemonic.clone())?;
		let seed = mnemonic.to_seed(self.sender_password.clone());
		let derivation_path = bip32::DerivationPath::from_str("m/44'/118'/0'/0/0")?;
		let signing_key = SigningKey::derive_from_path(seed, &derivation_path)?;
		Ok(signing_key)
	}

	async fn process_block(&self, data: AppData) -> Result<BroadcastTxRequest> {
		let transfers = decode_app_data(data)?;

		let msg = serde_json::to_vec(&ExecuteTransfers::TransferBalance { transfers })?;

		// NOTE: We cannot pass SigningKey to async fn due to missing Send marker
		let sender_private_key = self.sender_private_key()?;
		let sender_public_key = sender_private_key.public_key();
		let sender_account_id = sender_public_key
			.account_id("wasm")
			.map_err(|error| anyhow!("{error}"))?
			.to_string();

		let execute_msg = MsgExecuteContract {
			sender: sender_account_id,
			contract: self.contract.clone(),
			msg,
			funds: vec![],
		};

		let chain_id = self.chain_id.parse()?;

		let memo = "";
		let timeout_height = 9001u16;
		let tx_body = Body::new(vec![execute_msg.to_any()?], memo, timeout_height);

		// Signing
		let gas = 500_000u64;
		let amount = Coin::new(0, "atom").map_err(|error| anyhow!("{error}"))?;
		let signer_info = SignerInfo::single_direct(Some(sender_public_key), self.sequence);
		let auth_info = signer_info.auth_info(Fee::from_amount_and_gas(amount, gas));
		let sign_doc = SignDoc::new(&tx_body, &auth_info, &chain_id, self.sender_account_number)
			.map_err(|error| anyhow!("{error}"))?;
		let tx_signed = sign_doc
			.sign(&sender_private_key)
			.map_err(|error| anyhow!("{error}"))?;
		let tx_bytes = tx_signed.to_bytes().map_err(|error| anyhow!("{error}"))?;

		let mode = BroadcastMode::Block.into();
		let request = BroadcastTxRequest { tx_bytes, mode };

		Ok(request)
	}

	pub async fn run(
		&mut self,
		mut app_rx: Receiver<AppData>,
		error_sender: Sender<anyhow::Error>,
	) {
		let Ok(mut client) = ServiceClient::connect(self.node_host.clone()).await else {
		    let message = "Cannot connect to the cosmos node";
		    if let Err(error) = error_sender.send(anyhow!(message)).await {
			error!("Cannot send error message: {error}");
		    }
		    return;
		};
		// mod toolkit
		while let Some(block) = app_rx.recv().await {
			// mod custom
			match self.process_block(block).await {
				Ok(request) => {
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
