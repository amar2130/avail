use crate::types::{Mode, State};

use super::common::types::{
	ClientResponse, ConfidenceResponse, ExtrinsicsDataResponse, FfiSafeAppDataQuery,
	FfiSafeConfidenceResponse, FfiSafeStatus, LatestBlockResponse,
};
use crate::api::common;
use crate::api::common::types::AppDataQuery;
use rocksdb::DB;
use std::{
	ffi::CString,
	ptr::{self},
	sync::{Arc, Mutex},
};
pub struct EmbedState {
	db: Arc<DB>,
	state: Arc<Mutex<State>>,
}
impl EmbedState {
	pub fn new(state: Arc<Mutex<State>>, db: Arc<DB>) -> Self {
		return Self { state, db };
	}
	pub fn from_ptr(embed_state: *const EmbedState) -> &'static mut EmbedState {
		let r = unsafe {
			let mut p = ptr::NonNull::new(embed_state as *mut EmbedState).unwrap();
			p.as_mut()
		};
		return r;
	}
	fn get_state(&self) -> Arc<Mutex<State>> {
		return self.state.clone();
	}
	fn get_db(&self) -> Arc<DB> {
		return self.db.clone();
	}
}
fn get_state(embed_state_ref: *const EmbedState) -> Arc<Mutex<State>> {
	let embed_sate: &'static mut EmbedState = EmbedState::from_ptr(embed_state_ref);
	let state: Arc<Mutex<State>> = EmbedState::get_state(embed_sate);
	return state;
}

fn get_db(embed_state_ref: *const EmbedState) -> Arc<DB> {
	let embed_sate: &'static mut EmbedState = EmbedState::from_ptr(embed_state_ref);
	let db: Arc<DB> = EmbedState::get_db(embed_sate);
	return db;
}

#[no_mangle]
pub extern "C" fn c_mode(app_id: u32) -> ClientResponse<Mode> {
	return common::mode(Some(app_id));
}
#[allow(improper_ctypes_definitions)]
#[no_mangle]
pub extern "C" fn c_confidence(
	block_number: u32,
	embed_state: *const EmbedState,
) -> ClientResponse<common::types::FfiSafeConfidenceResponse> {
	let db: Arc<DB> = get_db(embed_state);
	let state: Arc<Mutex<State>> = get_state(embed_state);

	let client_response: ClientResponse<ConfidenceResponse> =
		common::confidence(block_number, db, state);

	match client_response {
		ClientResponse::Normal(res) => {
			return ClientResponse::Normal(FfiSafeConfidenceResponse {
				block: res.block,
				confidence: res.confidence,
				serialised_confidence: CString::new(res.serialised_confidence.unwrap()).unwrap(),
			});
		},

		ClientResponse::Error(e) => {
			return ClientResponse::Error(e);
		},
		ClientResponse::InProcess => {
			return ClientResponse::InProcess;
		},
		ClientResponse::NotFound => {
			return ClientResponse::NotFound;
		},
		ClientResponse::NotFinalized => {
			return ClientResponse::NotFinalized;
		},
	}
}

#[no_mangle]
pub extern "C" fn c_status(
	app_id: u32,
	embed_state: *const EmbedState,
) -> ClientResponse<FfiSafeStatus> {
	let db: Arc<DB> = get_db(embed_state);
	let state: Arc<Mutex<State>> = get_state(embed_state);
	let client_response = common::status(Some(app_id), state, db);
	match client_response {
		ClientResponse::Normal(res) => {
			return ClientResponse::Normal(FfiSafeStatus {
				block_num: res.block_num,
				app_id: res.app_id.unwrap(),
				confidence: res.confidence,
			});
		},

		ClientResponse::Error(e) => {
			return ClientResponse::Error(e);
		},
		ClientResponse::InProcess => {
			return ClientResponse::InProcess;
		},
		ClientResponse::NotFound => {
			return ClientResponse::NotFound;
		},
		ClientResponse::NotFinalized => {
			return ClientResponse::NotFinalized;
		},
	}
}

#[no_mangle]
pub extern "C" fn c_latest_block(
	embed_state: *const EmbedState,
) -> ClientResponse<LatestBlockResponse> {
	let state: Arc<Mutex<State>> = get_state(embed_state);
	return common::latest_block(state);
}
#[allow(improper_ctypes_definitions)]
#[no_mangle]
pub extern "C" fn c_appdata(
	block_num: u32,
	query: FfiSafeAppDataQuery,
	app_id: u32,
	embed_state: *const EmbedState,
) -> ClientResponse<ExtrinsicsDataResponse> {
	let db: Arc<DB> = get_db(embed_state);
	let state: Arc<Mutex<State>> = get_state(embed_state);
	return common::appdata(
		block_num,
		AppDataQuery {
			decode: Some(query.decode),
		},
		db,
		Some(app_id),
		state,
	);
}
