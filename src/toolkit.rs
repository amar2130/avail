use anyhow::{anyhow, Context, Result};
use avail_subxt::{
	api::runtime_types::{da_control::pallet::Call, da_runtime::RuntimeCall},
	primitives::AppUncheckedExtrinsic,
};
use codec::Decode;
use kate_recovery::com::AppData;
use serde::Deserialize;

#[allow(unused)]
fn decode_json_app_data<T: for<'a> Deserialize<'a>>(data: AppData) -> Result<Vec<T>> {
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
