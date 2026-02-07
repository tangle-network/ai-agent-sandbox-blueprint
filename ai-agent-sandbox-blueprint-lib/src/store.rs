use std::collections::HashMap;
use std::path::PathBuf;

pub use blueprint_sdk::stores::local_database::{Error as StoreError, LocalDatabase};

use crate::error::{Result, SandboxError};

impl From<StoreError> for SandboxError {
    fn from(err: StoreError) -> Self {
        SandboxError::Storage(err.to_string())
    }
}

/// Resolve the state directory from `BLUEPRINT_STATE_DIR` env var,
/// defaulting to `./blueprint-state`.
pub fn state_dir() -> PathBuf {
    std::env::var("BLUEPRINT_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("blueprint-state"))
}

/// Convenience wrapper that bridges `LocalDatabase` to our `SandboxError` types.
/// Keys are serialized to strings for storage.
pub struct PersistentStore<V> {
    db: LocalDatabase<V>,
}

impl<V> PersistentStore<V>
where
    V: serde::Serialize + serde::de::DeserializeOwned + Clone,
{
    pub fn open(path: PathBuf) -> Result<Self> {
        let db = LocalDatabase::open(path)?;
        Ok(Self { db })
    }

    pub fn get(&self, key: &str) -> Result<Option<V>> {
        Ok(self.db.get(key)?)
    }

    pub fn find<F>(&self, predicate: F) -> Result<Option<V>>
    where
        F: Fn(&V) -> bool,
    {
        Ok(self.db.find(predicate)?)
    }

    pub fn values(&self) -> Result<Vec<V>> {
        Ok(self.db.values()?)
    }

    pub fn insert(&self, key: String, value: V) -> Result<()> {
        Ok(self.db.set(&key, value)?)
    }

    pub fn remove(&self, key: &str) -> Result<Option<V>> {
        Ok(self.db.remove(key)?)
    }

    pub fn update<F>(&self, key: &str, f: F) -> Result<bool>
    where
        F: FnOnce(&mut V),
    {
        Ok(self.db.update(key, f)?)
    }

    pub fn replace(&self, map: HashMap<String, V>) -> Result<()> {
        Ok(self.db.replace(map)?)
    }
}
