//! Parallelized proof verification

use dusk_plonk::commitment_scheme::kzg10::PublicParameters;
use itertools::{Either, Itertools};
use kate_recovery::{
	data::{Cell, DataCell},
	matrix::{Dimensions, Position},
	proof,
};
use std::sync::{mpsc::channel, Arc};
use tracing::error;

/// Verifies proofs for given block, cells and commitments
pub fn verify(
	block_num: u32,
	dimensions: Dimensions,
	cells: &[Cell],
	commitments: &[[u8; 48]],
	public_parameters: Arc<PublicParameters>,
) -> Result<(Vec<Position>, Vec<Position>), proof::Error> {
	let cpus = num_cpus::get();
	let pool = threadpool::ThreadPool::new(cpus);
	let (tx, rx) = channel::<(Position, Result<bool, proof::Error>)>();

	for cell in cells {
		let commitment = commitments[cell.position.row as usize];

		let tx = tx.clone();
		let cell = cell.clone();
		let public_parameters = public_parameters.clone();

		pool.execute(move || {
			let result = proof::verify(&public_parameters, dimensions, &commitment, &cell);
			if let Err(error) = tx.clone().send((cell.position, result)) {
				error!(block_num, "Failed to send proof verified message: {error}");
			}
		});
	}

	let (verified, unverified): (Vec<_>, Vec<_>) = rx
		.iter()
		.take(cells.len())
		.map(|(position, result)| result.map(|is_verified| (position, is_verified)))
		.collect::<Result<Vec<(Position, bool)>, _>>()?
		.into_iter()
		.partition_map(|(position, is_verified)| match is_verified {
			true => Either::Left(position),
			false => Either::Right(position),
		});

	Ok((verified, unverified))
}

fn find_overlap<'a>(left: &'a [u8], right: &'a [u8]) -> &'a [u8] {
	(0..=right.len())
		.rev()
		.find_map(|overlap_size| {
			let right = &right[..overlap_size];
			left.ends_with(right).then_some(right)
		})
		.unwrap_or(&[])
}

fn data_chunks(first_cell: &DataCell, data: Vec<u8>) -> Vec<Vec<u8>> {
	let first = find_overlap(&first_cell.data[..31], &data).to_vec();
	let rest = data[first.len()..].chunks(31).map(|chunk| chunk.to_vec());
	vec![first].into_iter().chain(rest).collect::<Vec<Vec<_>>>()
}

pub fn data_positions(cells: Vec<DataCell>, data: Vec<u8>) -> Option<Vec<Position>> {
	let chunks = data_chunks(&cells[0], data);
	let positions = chunks
		.iter()
		.zip(cells)
		.enumerate()
		.filter_map(|(i, (data, cell))| {
			let cell_data = cell.data[..31].to_vec();
			match i {
				0 => cell_data.ends_with(data),
				_ if i == chunks.len() - 1 => cell_data.starts_with(data),
				_ => &cell_data.to_vec() == data,
			}
			.then_some(cell.position)
		})
		.collect::<Vec<_>>();
	(positions.len() == chunks.len()).then_some(positions)
}
