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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp_store() -> (PersistentStore<String>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.json");
        let store = PersistentStore::open(path).unwrap();
        (store, dir) // keep dir alive so it isn't deleted
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let (store, _dir) = temp_store();
        store.insert("key1".into(), "value1".into()).unwrap();

        let val = store.get("key1").unwrap();
        assert_eq!(val, Some("value1".to_string()));
    }

    #[test]
    fn insert_duplicate_key_overwrites() {
        let (store, _dir) = temp_store();
        store.insert("k".into(), "first".into()).unwrap();
        store.insert("k".into(), "second".into()).unwrap();

        let val = store.get("k").unwrap();
        assert_eq!(val, Some("second".to_string()));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let (store, _dir) = temp_store();
        let val = store.get("missing").unwrap();
        assert_eq!(val, None);
    }

    #[test]
    fn remove_returns_removed_value() {
        let (store, _dir) = temp_store();
        store.insert("k".into(), "v".into()).unwrap();

        let removed = store.remove("k").unwrap();
        assert_eq!(removed, Some("v".to_string()));

        // Subsequent get returns None
        assert_eq!(store.get("k").unwrap(), None);
    }

    #[test]
    fn remove_nonexistent_key_returns_none() {
        let (store, _dir) = temp_store();
        let removed = store.remove("nope").unwrap();
        assert_eq!(removed, None);
    }

    #[test]
    fn values_returns_all_values() {
        let (store, _dir) = temp_store();
        store.insert("a".into(), "alpha".into()).unwrap();
        store.insert("b".into(), "beta".into()).unwrap();
        store.insert("c".into(), "gamma".into()).unwrap();

        let mut vals = store.values().unwrap();
        vals.sort();
        assert_eq!(vals, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn values_empty_store() {
        let (store, _dir) = temp_store();
        let vals = store.values().unwrap();
        assert!(vals.is_empty());
    }

    #[test]
    fn find_with_predicate() {
        let (store, _dir) = temp_store();
        store.insert("a".into(), "apple".into()).unwrap();
        store.insert("b".into(), "banana".into()).unwrap();
        store.insert("c".into(), "cherry".into()).unwrap();

        let found = store.find(|v| v.starts_with('b')).unwrap();
        assert_eq!(found, Some("banana".to_string()));
    }

    #[test]
    fn find_no_match_returns_none() {
        let (store, _dir) = temp_store();
        store.insert("a".into(), "apple".into()).unwrap();

        let found = store.find(|v| v == "zebra").unwrap();
        assert_eq!(found, None);
    }

    #[test]
    fn update_modifies_value_in_place() {
        let (store, _dir) = temp_store();
        store.insert("k".into(), "hello".into()).unwrap();

        let updated = store
            .update("k", |v| {
                v.push_str(" world");
            })
            .unwrap();
        assert!(updated, "update should return true for existing key");

        let val = store.get("k").unwrap();
        assert_eq!(val, Some("hello world".to_string()));
    }

    #[test]
    fn update_nonexistent_returns_false() {
        let (store, _dir) = temp_store();
        let updated = store.update("nope", |v| v.push('x')).unwrap();
        assert!(!updated, "update should return false for missing key");
    }

    #[test]
    fn replace_entire_store() {
        let (store, _dir) = temp_store();
        store.insert("old".into(), "data".into()).unwrap();

        let mut new_map = HashMap::new();
        new_map.insert("x".into(), "one".to_string());
        new_map.insert("y".into(), "two".to_string());
        store.replace(new_map).unwrap();

        assert_eq!(store.get("old").unwrap(), None, "old key should be gone");
        assert_eq!(store.get("x").unwrap(), Some("one".to_string()));
        assert_eq!(store.get("y").unwrap(), Some("two".to_string()));
    }

    #[test]
    fn concurrent_read_access() {
        let (store, _dir) = temp_store();
        store.insert("shared".into(), "data".into()).unwrap();

        let store = Arc::new(store);
        let mut handles = Vec::new();

        for _ in 0..8 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    let val = s.get("shared").unwrap();
                    assert_eq!(val, Some("data".to_string()));
                }
            }));
        }

        for h in handles {
            h.join().expect("reader thread panicked");
        }
    }

    #[test]
    fn sequential_write_during_read_no_deadlock() {
        let (store, _dir) = temp_store();
        store.insert("k".into(), "v1".into()).unwrap();

        // Read
        let val = store.get("k").unwrap();
        assert_eq!(val, Some("v1".to_string()));

        // Write (should not deadlock since the read lock was dropped)
        store.insert("k".into(), "v2".into()).unwrap();

        // Read again
        let val = store.get("k").unwrap();
        assert_eq!(val, Some("v2".to_string()));
    }
}
