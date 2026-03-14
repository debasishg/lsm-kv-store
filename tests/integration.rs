use lsm_kv_store::engine::{KvStore, KvStoreConfig};
use tempfile::tempdir;

fn store_with_threshold(threshold: usize) -> (KvStore, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let mut config = KvStoreConfig::new(dir.path());
    config.memtable_threshold = threshold;
    let store = KvStore::open(config).unwrap();
    (store, dir)
}

fn open_store(dir: &std::path::Path) -> KvStore {
    KvStore::open_default(dir).unwrap()
}

#[test]
fn test_put_get_roundtrip() {
    let (mut store, _dir) = store_with_threshold(4 * 1024 * 1024);
    store.put("hello", "world").unwrap();
    store.put("foo", "bar").unwrap();

    assert_eq!(store.get("hello").unwrap(), Some(b"world".to_vec()));
    assert_eq!(store.get("foo").unwrap(), Some(b"bar".to_vec()));
    assert_eq!(store.get("missing").unwrap(), None);
}

#[test]
fn test_overwrite_semantics() {
    let (mut store, _dir) = store_with_threshold(4 * 1024 * 1024);
    store.put("key", "v1").unwrap();
    assert_eq!(store.get("key").unwrap(), Some(b"v1".to_vec()));

    store.put("key", "v2").unwrap();
    assert_eq!(store.get("key").unwrap(), Some(b"v2".to_vec()));
}

#[test]
fn test_delete_tombstone() {
    let (mut store, _dir) = store_with_threshold(4 * 1024 * 1024);
    store.put("key", "value").unwrap();
    assert_eq!(store.get("key").unwrap(), Some(b"value".to_vec()));

    store.delete("key").unwrap();
    assert_eq!(store.get("key").unwrap(), None);
}

#[test]
fn test_delete_nonexistent_key() {
    let (mut store, _dir) = store_with_threshold(4 * 1024 * 1024);
    // Deleting a key that was never put should not error.
    store.delete("phantom").unwrap();
    assert_eq!(store.get("phantom").unwrap(), None);
}

#[test]
fn test_persistence_across_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().to_path_buf();

    // Session 1: write some data and force flush.
    {
        let mut store = open_store(&db_path);
        store.put("persist_key", "persist_value").unwrap();
        store.flush().unwrap();
    }

    // Session 2: reopen and verify data is still there.
    {
        let store = open_store(&db_path);
        assert_eq!(
            store.get("persist_key").unwrap(),
            Some(b"persist_value".to_vec())
        );
    }
}

#[test]
fn test_wal_recovery_without_flush() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().to_path_buf();

    // Session 1: write data without flushing — only WAL.
    {
        let mut store = open_store(&db_path);
        store.put("wal_key", "wal_value").unwrap();
        // Drop without flush — WAL should persist the entry.
    }

    // Session 2: reopen — WAL recovery should restore the entry.
    {
        let store = open_store(&db_path);
        assert_eq!(store.get("wal_key").unwrap(), Some(b"wal_value".to_vec()));
    }
}

#[test]
fn test_overwrite_persists_across_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().to_path_buf();

    {
        let mut store = open_store(&db_path);
        store.put("key", "v1").unwrap();
        store.put("key", "v2").unwrap();
        store.flush().unwrap();
    }

    {
        let store = open_store(&db_path);
        assert_eq!(store.get("key").unwrap(), Some(b"v2".to_vec()));
    }
}

#[test]
fn test_tombstone_persists_across_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().to_path_buf();

    {
        let mut store = open_store(&db_path);
        store.put("doomed", "value").unwrap();
        store.delete("doomed").unwrap();
        store.flush().unwrap();
    }

    {
        let store = open_store(&db_path);
        assert_eq!(store.get("doomed").unwrap(), None);
    }
}

#[test]
fn test_list_returns_live_entries_only() {
    let (mut store, _dir) = store_with_threshold(4 * 1024 * 1024);
    store.put("a", "1").unwrap();
    store.put("b", "2").unwrap();
    store.put("c", "3").unwrap();
    store.delete("b").unwrap();

    let entries = store.list().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], (b"a".to_vec(), b"1".to_vec()));
    assert_eq!(entries[1], (b"c".to_vec(), b"3".to_vec()));
}

#[test]
fn test_flush_and_read_from_sstable() {
    // Use a tiny threshold to force flush.
    let (mut store, _dir) = store_with_threshold(10);
    store.put("big_enough_key", "big_enough_value").unwrap();
    // At this point, memtable should have flushed.
    assert_eq!(
        store.get("big_enough_key").unwrap(),
        Some(b"big_enough_value".to_vec())
    );
}

#[test]
fn test_many_writes_and_reads() {
    let (mut store, _dir) = store_with_threshold(256);

    for i in 0..100 {
        let key = format!("key_{i:04}");
        let value = format!("value_{i:04}");
        store.put(key.as_bytes(), value.as_bytes()).unwrap();
    }

    for i in 0..100 {
        let key = format!("key_{i:04}");
        let value = format!("value_{i:04}");
        assert_eq!(
            store.get(key.as_bytes()).unwrap(),
            Some(value.into_bytes()),
            "Failed to read key_{i:04}"
        );
    }
}
