use serde::{Deserialize, Serialize};
use std::ffi::CString;

#[repr(C)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FfiSafeConfidenceResponse {
	pub block: u32,
	pub confidence: f64,
	pub serialised_confidence: CString,
}

#[repr(C)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FfiSafeStatus {
	pub block_num: u32,
	pub confidence: f64,
	pub app_id: u32,
}

#[repr(C)]
#[derive(Deserialize, Serialize)]
pub struct FfiSafeAppDataQuery {
	pub decode: bool,
}
