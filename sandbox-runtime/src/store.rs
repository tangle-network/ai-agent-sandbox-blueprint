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
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::error!(path = %dir.display(), error = %e, "Failed to create state directory");
        }
        // Restrict directory permissions: only owner can read/write/traverse.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)) {
                tracing::warn!(path = %dir.display(), error = %e, "Failed to set state directory permissions");
            }
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
///
/// **Limitation**: No OS-level file locking (flock/fcntl) is applied.
/// Two operator processes sharing the same `BLUEPRINT_STATE_DIR` can
/// corrupt the JSON store. Each operator must use a unique state directory.
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
    fn remove_returns_removed_value() {
        let (store, _dir) = temp_store();
        store.insert("k".into(), "v".into()).unwrap();

        let removed = store.remove("k").unwrap();
        assert_eq!(removed, Some("v".to_string()));

        // Subsequent get returns None
        assert_eq!(store.get("k").unwrap(), None);
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
    fn concurrent_write_access() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent_write.json");
        let store: Arc<PersistentStore<String>> = Arc::new(PersistentStore::open(path).unwrap());

        let mut handles = Vec::new();
        for thread_idx in 0..8u32 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in 0..50u32 {
                    let key = format!("t{thread_idx}_k{i}");
                    let val = format!("t{thread_idx}_v{i}");
                    s.insert(key, val).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().expect("writer thread panicked");
        }

        // All 8 * 50 = 400 keys must be present
        let vals = store.values().unwrap();
        assert_eq!(
            vals.len(),
            400,
            "expected 400 keys after concurrent writes, got {}",
            vals.len()
        );
        // Spot-check a few keys from different threads
        for thread_idx in [0u32, 3, 7] {
            for i in [0u32, 25, 49] {
                let key = format!("t{thread_idx}_k{i}");
                let expected = format!("t{thread_idx}_v{i}");
                assert_eq!(
                    store.get(&key).unwrap(),
                    Some(expected),
                    "missing or wrong value for key {key}"
                );
            }
        }
    }

    #[test]
    fn concurrent_read_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent_rw.json");
        let store: Arc<PersistentStore<String>> = Arc::new(PersistentStore::open(path).unwrap());

        // Pre-insert a key that readers will read
        store
            .insert("shared_read".into(), "stable_value".into())
            .unwrap();

        let mut handles = Vec::new();

        // 4 writer threads — each writes unique keys
        for thread_idx in 0..4u32 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in 0..50u32 {
                    let key = format!("w{thread_idx}_{i}");
                    s.insert(key, format!("val_{thread_idx}_{i}")).unwrap();
                }
            }));
        }

        // 4 reader threads — each reads the pre-inserted key repeatedly
        for _ in 0..4u32 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    let val = s.get("shared_read").unwrap();
                    assert_eq!(
                        val,
                        Some("stable_value".to_string()),
                        "reader saw unexpected value"
                    );
                }
            }));
        }

        for h in handles {
            h.join()
                .expect("thread panicked during concurrent read/write");
        }

        // Verify all writer keys are present (4 writers * 50 keys = 200)
        // plus the 1 pre-inserted key = 201 total
        let vals = store.values().unwrap();
        assert_eq!(
            vals.len(),
            201,
            "expected 201 total values, got {}",
            vals.len()
        );
    }

    #[test]
    fn update_concurrent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent_update.json");
        let store: Arc<PersistentStore<i32>> = Arc::new(PersistentStore::open(path).unwrap());

        // Seed the counter at 0
        store.insert("counter".into(), 0).unwrap();

        let mut handles = Vec::new();
        for _ in 0..4u32 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    s.update("counter", |v| *v += 1).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().expect("updater thread panicked");
        }

        let final_val = store.get("counter").unwrap().expect("counter key missing");
        assert_eq!(
            final_val, 400,
            "expected counter=400 after 4 threads * 100 increments, got {final_val}"
        );
    }

    // ── Phase 3E: Concurrent Store CRUD Tests ───────────────────────────

    #[test]
    fn concurrent_update_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent_update_remove.json");
        let store: Arc<PersistentStore<String>> = Arc::new(PersistentStore::open(path).unwrap());

        // Seed entries
        for i in 0..100u32 {
            store
                .insert(format!("k{i}"), format!("v{i}"))
                .unwrap();
        }

        let mut handles = Vec::new();

        // 2 threads updating even keys
        for t in 0..2u32 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in (0..100u32).step_by(2) {
                    let _ = s.update(&format!("k{i}"), |v| {
                        v.push_str(&format!("-t{t}"));
                    });
                }
            }));
        }

        // 2 threads removing odd keys
        for _ in 0..2u32 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in (1..100u32).step_by(2) {
                    let _ = s.remove(&format!("k{i}"));
                }
            }));
        }

        for h in handles {
            h.join().expect("thread should not panic during concurrent update+remove");
        }

        // Even keys should still exist (possibly modified)
        for i in (0..100u32).step_by(2) {
            assert!(
                store.get(&format!("k{i}")).unwrap().is_some(),
                "even key k{i} should survive"
            );
        }
    }

    #[test]
    fn concurrent_find_while_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent_find_write.json");
        let store: Arc<PersistentStore<String>> = Arc::new(PersistentStore::open(path).unwrap());

        // Seed a known entry
        store.insert("target".into(), "findme".into()).unwrap();

        let mut handles = Vec::new();

        // 4 writer threads inserting new keys
        for t in 0..4u32 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in 0..50u32 {
                    s.insert(format!("w{t}_{i}"), "data".to_string()).unwrap();
                }
            }));
        }

        // 4 finder threads searching for "findme"
        for _ in 0..4u32 {
            let s = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    let found = s.find(|v| v == "findme").unwrap();
                    assert_eq!(
                        found,
                        Some("findme".to_string()),
                        "find should consistently locate the target value"
                    );
                }
            }));
        }

        for h in handles {
            h.join().expect("thread should not panic during concurrent find+insert");
        }
    }
}
