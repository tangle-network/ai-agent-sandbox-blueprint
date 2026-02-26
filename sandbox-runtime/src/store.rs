use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

pub use blueprint_sdk::stores::local_database::{Error as StoreError, LocalDatabase};

use crate::error::{Result, SandboxError};

impl From<StoreError> for SandboxError {
    fn from(err: StoreError) -> Self {
        SandboxError::Storage(err.to_string())
    }
}

/// Resolve the state directory from `BLUEPRINT_STATE_DIR` env var,
/// defaulting to `./blueprint-state`.
///
/// Creates the directory with restrictive permissions (0o700) if it doesn't exist.
pub fn state_dir() -> PathBuf {
    let dir = std::env::var("BLUEPRINT_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("blueprint-state"));

    if !dir.exists() {
        std::fs::create_dir_all(&dir).ok();
        // Restrict directory permissions: only owner can read/write/traverse.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
        }
    }

    dir
}

/// Convenience wrapper that bridges `LocalDatabase` to our `SandboxError` types.
/// Keys are serialized to strings for storage.
///
/// All operations are protected by a `RwLock` to prevent concurrent
/// read-modify-write races across multiple tokio tasks (reaper, GC,
/// API handlers). Read operations acquire a shared read lock; write
/// operations acquire an exclusive write lock.
pub struct PersistentStore<V> {
    db: RwLock<LocalDatabase<V>>,
}

impl<V> PersistentStore<V>
where
    V: serde::Serialize + serde::de::DeserializeOwned + Clone,
{
    pub fn open(path: PathBuf) -> Result<Self> {
        let db = LocalDatabase::open(path)?;
        Ok(Self {
            db: RwLock::new(db),
        })
    }

    pub fn get(&self, key: &str) -> Result<Option<V>> {
        let db = self
            .db
            .read()
            .map_err(|_| SandboxError::Storage("PersistentStore RwLock poisoned (read)".into()))?;
        Ok(db.get(key)?)
    }

    pub fn find<F>(&self, predicate: F) -> Result<Option<V>>
    where
        F: Fn(&V) -> bool,
    {
        let db = self
            .db
            .read()
            .map_err(|_| SandboxError::Storage("PersistentStore RwLock poisoned (read)".into()))?;
        Ok(db.find(predicate)?)
    }

    pub fn values(&self) -> Result<Vec<V>> {
        let db = self
            .db
            .read()
            .map_err(|_| SandboxError::Storage("PersistentStore RwLock poisoned (read)".into()))?;
        Ok(db.values()?)
    }

    pub fn insert(&self, key: String, value: V) -> Result<()> {
        let db = self
            .db
            .write()
            .map_err(|_| SandboxError::Storage("PersistentStore RwLock poisoned (write)".into()))?;
        Ok(db.set(&key, value)?)
    }

    pub fn remove(&self, key: &str) -> Result<Option<V>> {
        let db = self
            .db
            .write()
            .map_err(|_| SandboxError::Storage("PersistentStore RwLock poisoned (write)".into()))?;
        Ok(db.remove(key)?)
    }

    pub fn update<F>(&self, key: &str, f: F) -> Result<bool>
    where
        F: FnOnce(&mut V),
    {
        let db = self
            .db
            .write()
            .map_err(|_| SandboxError::Storage("PersistentStore RwLock poisoned (write)".into()))?;
        Ok(db.update(key, f)?)
    }

    pub fn replace(&self, map: HashMap<String, V>) -> Result<()> {
        let db = self
            .db
            .write()
            .map_err(|_| SandboxError::Storage("PersistentStore RwLock poisoned (write)".into()))?;
        Ok(db.replace(map)?)
    }
}
