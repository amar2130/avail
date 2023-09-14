use anyhow::{anyhow, Result};
use rocksdb::DB;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::channel;

use crate::api::v1::common::types::{ClientResponse, ExtrinsicsDataResponse, LatestBlockResponse};
use crate::api::v1::ffi::types::{FfiSafeAppDataQuery, FfiSafeConfidenceResponse, FfiSafeStatus};
use crate::api::v1::ffi::{c_appdata, c_confidence, c_latest_block, c_mode, c_status};
use crate::light_client_commons::run;
use crate::types::{Mode, RuntimeConfig, State};
use tracing::error;

use crate::api::v1::ffi::EmbedState;
use crate::light_client_commons::{DB, STATE};

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub async unsafe extern "C" fn start_light_node(cfg: RuntimeConfig) -> Result<bool> {
	let (error_sender, mut error_receiver) = channel::<anyhow::Error>(1);
	let res = run(error_sender, cfg, false).await;
	if let Err(error) = res {
		error!("{error}");
		return Err(error);
	} else {
		let (state, db): (Arc<Mutex<State>>, Arc<DB>) = res.unwrap();
		STATE = Some(state);
		DB = Some(db);
	};

	let error = match error_receiver.recv().await {
		Some(error) => error,
		None => anyhow!("Failed to receive error message"),
	};
	Err(error)
}

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub unsafe extern "C" fn android_block_confidence(
	block_number: u32,
) -> ClientResponse<FfiSafeConfidenceResponse> {
	if STATE.is_some() && DB.is_some() {
		let embed_state: EmbedState = EmbedState::new(STATE.clone().unwrap(), DB.clone().unwrap());
		return c_confidence(block_number, &embed_state);
	} else {
		return ClientResponse::NotFound;
	}
}

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub unsafe extern "C" fn android_status(app_id: u32) -> ClientResponse<FfiSafeStatus> {
	if STATE.is_some() && DB.is_some() {
		let embed_state: EmbedState = EmbedState::new(STATE.clone().unwrap(), DB.clone().unwrap());
		return c_status(app_id, &embed_state);
	} else {
		return ClientResponse::NotFound;
	}
}
#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub unsafe extern "C" fn android_latest_block() -> ClientResponse<LatestBlockResponse> {
	if STATE.is_some() && DB.is_some() {
		let embed_state: EmbedState = EmbedState::new(STATE.clone().unwrap(), DB.clone().unwrap());
		return c_latest_block(&embed_state);
	} else {
		return ClientResponse::NotFound;
	}
}

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub unsafe extern "C" fn android_appdata(
	block_num: u32,
	query: FfiSafeAppDataQuery,
	app_id: u32,
) -> ClientResponse<ExtrinsicsDataResponse> {
	if STATE.is_some() && DB.is_some() {
		let embed_state: EmbedState = EmbedState::new(STATE.clone().unwrap(), DB.clone().unwrap());
		return c_appdata(block_num, query, app_id, &embed_state);
	} else {
		return ClientResponse::NotFound;
	}
}

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub unsafe extern "C" fn android_mode(app_id: u32) -> ClientResponse<Mode> {
	if STATE.is_some() && DB.is_some() {
		return c_mode(app_id);
	} else {
		return ClientResponse::NotFound;
	}
}
