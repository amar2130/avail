use crate::data::{Database, Key};
use codec::{Decode, Encode};
use libp2p::identity::PeerId;
use libp2p::kad::store::{Error, RecordStore, Result};
use libp2p::kad::{self, KBucketKey, ProviderRecord, K_VALUE};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::{hash_map, hash_set, HashMap, HashSet};
use std::iter;
use std::time::{Duration, Instant};
use tracing::error;

// use super::RecordsIter;

/// Flexible implementation of a `RecordStore`.
pub struct Store<T>
where
	T: Database + Iter,
{
	/// The identity of the peer owning the store.
	local_key: KBucketKey<PeerId>,
	/// The configuration of the store.
	config: StoreConfig,
	/// The stored (regular) records.
	records: T,
	/// The stored provider records.
	providers: HashMap<kad::RecordKey, SmallVec<[ProviderRecord; K_VALUE.get()]>>,
	/// The set of all provider records for the node identified by `local_key`.
	///
	/// Must be kept in sync with `providers`.
	provided: HashSet<ProviderRecord>,
}

/// Configuration for a `Store`.
#[derive(Debug, Clone)]
pub struct StoreConfig {
	/// The maximum number of records.
	pub max_records: usize,
	/// The maximum size of record values, in bytes.
	pub max_value_bytes: usize,
	/// The maximum number of providers stored for a key.
	///
	/// This should match up with the chosen replication factor.
	pub max_providers_per_key: usize,
	/// The maximum number of provider records for which the
	/// local node is the provider.
	pub max_provided_keys: usize,
}

impl Default for StoreConfig {
	// Default values kept in line with libp2p
	fn default() -> Self {
		Self {
			max_records: 1024,
			max_value_bytes: 65 * 1024,
			max_provided_keys: 1024,
			max_providers_per_key: K_VALUE.get(),
		}
	}
}

impl<T: Database + Iter> Store<T> {
	/// Creates a new `MemoryRecordStore` with the given configuration.
	pub fn with_config(local_id: PeerId, config: StoreConfig, records: T) -> Self {
		Store {
			local_key: KBucketKey::from(local_id),
			config,
			records,
			provided: HashSet::default(),
			providers: HashMap::default(),
		}
	}
}

#[derive(Serialize, Deserialize, Encode, Decode, Clone)]
pub struct Entry(pub Vec<u8>, pub Record);

#[derive(Serialize, Deserialize, Encode, Decode, Clone)]
pub struct Record {
	value: Vec<u8>,
	publisher: Vec<u8>,
	ttl: u32,
}

impl From<kad::Record> for Entry {
	fn from(record: kad::Record) -> Self {
		// 1 is minimum value if `expires` is set because 0 means "does not expire"
		let ttl = record
			.expires
			.map(|t| (t - Instant::now()).max(Duration::from_secs(1)).as_secs())
			.unwrap_or(0) as u32;
		let key = record.key.to_vec();
		let record = Record {
			value: record.value,
			publisher: record.publisher.map(PeerId::to_bytes).unwrap_or_default(),
			ttl,
		};
		Entry(key, record)
	}
}

impl From<Entry> for kad::Record {
	fn from(entry: Entry) -> Self {
		let Entry(key, record) = entry;

		kad::Record {
			key: kad::RecordKey::from(key),
			value: record.value,
			publisher: (!record.publisher.is_empty())
				.then(|| PeerId::from_bytes(&record.publisher).expect("Invalid peer ID")),
			expires: (record.ttl > 0)
				.then(|| Instant::now() + Duration::from_secs(record.ttl.into())),
		}
	}
}

pub struct DatabaseIter<T: Iterator<Item = kad::Record>> {
	inner: T,
}

impl<T: Iterator<Item = kad::Record>> Iterator for DatabaseIter<T> {
	type Item = T::Item;

	fn next(&mut self) -> Option<Self::Item> {
		self.inner.next()
	}
}

pub trait Iter {
	type Iterator: Iterator<Item = kad::Record>;

	fn iter(&self) -> Self::Iterator;
}

impl<T: Database + Iter> RecordStore for Store<T> {
	type RecordsIter<'a> = iter::Map<T::Iterator, fn(kad::Record) -> Cow<'a, kad::Record>> where T: 'a;

	type ProvidedIter<'a> = iter::Map<
		hash_set::Iter<'a, ProviderRecord>,
		fn(&'a ProviderRecord) -> Cow<'a, ProviderRecord>,
	> where T: 'a;

	fn get(&self, k: &kad::RecordKey) -> Option<Cow<'_, kad::Record>> {
		let record = self.records.get::<Entry>(Key::KademliaRecord(k.to_vec()));
		match record {
			Ok(record) => record.map(|entry| Cow::Owned(entry.into())),
			Err(error) => {
				error!("Cannot get record from store: {error}");
				None
			},
		}
	}

	fn put(&mut self, record: kad::Record) -> Result<()> {
		if record.value.len() >= self.config.max_value_bytes {
			return Err(Error::ValueTooLarge);
		}

		let Entry(key, record) = record.into();

		self.records
			.put(Key::KademliaRecord(key), record)
			.map_err(|_| Error::ValueTooLarge) // TODO error?
	}

	fn remove(&mut self, k: &kad::RecordKey) {
		// TODO: error?
		let _ = self.records.delete(Key::KademliaRecord(k.to_vec()));
	}

	fn records(&self) -> Self::RecordsIter<'_> {
		self.records.iter().map(Cow::Owned)
	}

	fn add_provider(&mut self, record: ProviderRecord) -> Result<()> {
		let num_keys = self.providers.len();

		// Obtain the entry
		let providers = match self.providers.entry(record.key.clone()) {
			e @ hash_map::Entry::Occupied(_) => e,
			e @ hash_map::Entry::Vacant(_) => {
				if self.config.max_provided_keys == num_keys {
					return Err(Error::MaxProvidedKeys);
				}
				e
			},
		}
		.or_insert_with(Default::default);

		if let Some(i) = providers.iter().position(|p| p.provider == record.provider) {
			// In-place update of an existing provider record.
			providers.as_mut()[i] = record;
		} else {
			// It is a new provider record for that key.
			let local_key = self.local_key.clone();
			let key = KBucketKey::new(record.key.clone());
			let provider = KBucketKey::from(record.provider);
			if let Some(i) = providers.iter().position(|p| {
				let pk = KBucketKey::from(p.provider);
				provider.distance(&key) < pk.distance(&key)
			}) {
				// Insert the new provider.
				if local_key.preimage() == &record.provider {
					self.provided.insert(record.clone());
				}
				providers.insert(i, record);
				// Remove the excess provider, if any.
				if providers.len() > self.config.max_providers_per_key {
					if let Some(p) = providers.pop() {
						self.provided.remove(&p);
					}
				}
			} else if providers.len() < self.config.max_providers_per_key {
				// The distance of the new provider to the key is larger than
				// the distance of any existing provider, but there is still room.
				if local_key.preimage() == &record.provider {
					self.provided.insert(record.clone());
				}
				providers.push(record);
			}
		}
		Ok(())
	}

	fn providers(&self, key: &kad::RecordKey) -> Vec<ProviderRecord> {
		self.providers
			.get(key)
			.map_or_else(Vec::new, |ps| ps.clone().into_vec())
	}

	fn provided(&self) -> Self::ProvidedIter<'_> {
		self.provided.iter().map(Cow::Borrowed)
	}

	fn remove_provider(&mut self, key: &kad::RecordKey, provider: &PeerId) {
		if let hash_map::Entry::Occupied(mut e) = self.providers.entry(key.clone()) {
			let providers = e.get_mut();
			if let Some(i) = providers.iter().position(|p| &p.provider == provider) {
				let p = providers.remove(i);
				self.provided.remove(&p);
			}
			if providers.is_empty() {
				e.remove();
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use std::time::{Duration, Instant};

	use super::*;
	use libp2p::{kad::KBucketDistance, multihash::Multihash};
	use proptest::{
		prelude::{any, any_with},
		proptest,
		sample::size_range,
		strategy::Strategy,
	};
	use rand::Rng;

	const SHA_256_MH: u64 = 0x12;
	const MULTIHASH_SIZE: usize = 32;

	fn random_multihash() -> Multihash<MULTIHASH_SIZE> {
		Multihash::wrap(SHA_256_MH, &rand::thread_rng().gen::<[u8; 32]>()).unwrap()
	}

	fn distance(r: &ProviderRecord) -> KBucketDistance {
		KBucketKey::new(r.key.clone()).distance(&KBucketKey::from(r.provider))
	}

	fn arb_key() -> impl Strategy<Value = kad::RecordKey> {
		any::<[u8; 32]>().prop_map(|hash| {
			kad::RecordKey::from(Multihash::<MULTIHASH_SIZE>::wrap(SHA_256_MH, &hash).unwrap())
		})
	}

	fn arb_publisher() -> impl Strategy<Value = Option<PeerId>> {
		any::<bool>().prop_map(|has_publisher| has_publisher.then(PeerId::random))
	}

	fn arb_expires() -> impl Strategy<Value = Option<Instant>> {
		(any::<bool>(), 0..60u64).prop_map(|(expires, seconds)| {
			expires.then(|| Instant::now() + Duration::from_secs(seconds))
		})
	}

	fn arb_record() -> impl Strategy<Value = Record> {
		(
			arb_key(),
			any_with::<Vec<u8>>(size_range(1..2048).lift()),
			arb_publisher(),
			arb_expires(),
		)
			.prop_map(|(key, value, publisher, expires)| Record {
				key,
				value,
				publisher,
				expires,
			})
	}

	fn arb_provider_record() -> impl Strategy<Value = ProviderRecord> {
		(arb_key(), arb_expires()).prop_map(|(key, expires)| ProviderRecord {
			key,
			provider: PeerId::random(),
			expires,
			addresses: vec![],
		})
	}

	proptest! {
	#[test]
	fn put_get_remove_record(r in arb_record()) {
		let mut store = MemoryStore::new(PeerId::random());
		assert!(store.put(r.clone()).is_ok());
		assert_eq!(Some(Cow::Borrowed(&r)), store.get(&r.key));
		store.remove(&r.key);
		assert!(store.get(&r.key).is_none());
	}
	}

	proptest! {
	#[test]
	fn add_get_remove_provider(r in arb_provider_record()) {
		let mut store = MemoryStore::new(PeerId::random());
		assert!(store.add_provider(r.clone()).is_ok());
		assert!(store.providers(&r.key).contains(&r));
		store.remove_provider(&r.key, &r.provider);
		assert!(!store.providers(&r.key).contains(&r));
	}
	}

	proptest! {
	#[test]
	fn providers_ordered_by_distance_to_key(providers  in 0..256u32) {
		let providers = (0..providers).
			map(|_| KBucketKey::from(PeerId::random())).
				collect::<Vec<_>>();
		let mut store = MemoryStore::new(PeerId::random());
		let key = kad::RecordKey::from(random_multihash());

		let mut records = providers
			.into_iter()
			.map(|p| ProviderRecord::new(key.clone(), p.into_preimage(), Vec::new()))
			.collect::<Vec<_>>();

		for r in &records {
			assert!(store.add_provider(r.clone()).is_ok());
		}

		records.sort_by_key(distance);
		records.truncate(store.config.max_providers_per_key);
		assert!(records == store.providers(&key).to_vec())
	}
	}

	#[test]
	fn provided() {
		let id = PeerId::random();
		let mut store = MemoryStore::new(id);
		let key = random_multihash();
		let rec = ProviderRecord::new(key, id, Vec::new());
		assert!(store.add_provider(rec.clone()).is_ok());
		assert_eq!(
			vec![Cow::Borrowed(&rec)],
			store.provided().collect::<Vec<_>>()
		);
		store.remove_provider(&rec.key, &id);
		assert_eq!(store.provided().count(), 0);
	}

	#[test]
	fn update_provider() {
		let mut store = MemoryStore::new(PeerId::random());
		let key = random_multihash();
		let prv = PeerId::random();
		let mut rec = ProviderRecord::new(key, prv, Vec::new());
		assert!(store.add_provider(rec.clone()).is_ok());
		assert_eq!(vec![rec.clone()], store.providers(&rec.key).to_vec());
		rec.expires = Some(Instant::now());
		assert!(store.add_provider(rec.clone()).is_ok());
		assert_eq!(vec![rec.clone()], store.providers(&rec.key).to_vec());
	}

	#[test]
	fn max_provided_keys() {
		let mut store = MemoryStore::new(PeerId::random());
		for _ in 0..store.config.max_provided_keys {
			let key = random_multihash();
			let prv = PeerId::random();
			let rec = ProviderRecord::new(key, prv, Vec::new());
			let _ = store.add_provider(rec);
		}
		let key = random_multihash();
		let prv = PeerId::random();
		let rec = ProviderRecord::new(key, prv, Vec::new());
		match store.add_provider(rec) {
			Err(Error::MaxProvidedKeys) => {},
			_ => panic!("Unexpected result"),
		}
	}
}
