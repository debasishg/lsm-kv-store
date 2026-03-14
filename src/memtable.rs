use std::collections::BTreeMap;

/// In-memory sorted key-value store backed by a `BTreeMap`.
///
/// Values are `Option<Vec<u8>>`: `Some(v)` for live entries, `None` for
/// tombstones (deleted keys). The memtable is flushed to an SSTable when
/// its approximate byte size exceeds a configured threshold.
#[derive(Debug)]
pub struct MemTable {
    entries: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    /// Approximate byte size of all keys + values held in the map.
    size: usize,
}

impl MemTable {
    /// Creates a new, empty `MemTable`.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            size: 0,
        }
    }

    /// Inserts or overwrites a key-value pair.
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        let key_len = key.len();
        let val_len = value.len();
        if let Some(old) = self.entries.insert(key, Some(value)) {
            // Key already counted — only adjust for value size change.
            let old_val_len = old.as_ref().map_or(0, Vec::len);
            self.size = self.size + val_len - old_val_len;
        } else {
            self.size += key_len + val_len;
        }
    }

    /// Looks up a key. Returns:
    /// - `Some(Some(value))` — key exists with a value
    /// - `Some(None)` — key has been deleted (tombstone)
    /// - `None` — key not present in this memtable
    pub fn get(&self, key: &[u8]) -> Option<&Option<Vec<u8>>> {
        self.entries.get(key)
    }

    /// Marks a key as deleted by inserting a tombstone (`None` value).
    pub fn delete(&mut self, key: Vec<u8>) {
        let key_len = key.len();
        if let Some(old) = self.entries.insert(key, None) {
            let removed = old.as_ref().map_or(0, Vec::len);
            self.size -= removed;
        } else {
            // New key entry (tombstone only), count the key bytes.
            self.size += key_len;
        }
    }

    /// Returns `true` if the memtable contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of entries (including tombstones).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the approximate byte size of all data held.
    pub fn approximate_size(&self) -> usize {
        self.size
    }

    /// Returns an iterator over all entries in sorted key order.
    pub fn entries(&self) -> impl Iterator<Item = (&Vec<u8>, &Option<Vec<u8>>)> {
        self.entries.iter()
    }

    /// Drains all entries out of the memtable, resetting it to empty.
    /// Returns entries in sorted key order.
    pub fn drain(&mut self) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
        self.size = 0;
        std::mem::take(&mut self.entries).into_iter().collect()
    }
}

impl Default for MemTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_and_get() {
        let mut mt = MemTable::new();
        mt.put(b"key1".to_vec(), b"value1".to_vec());
        assert_eq!(mt.get(b"key1"), Some(&Some(b"value1".to_vec())));
    }

    #[test]
    fn test_get_missing_key() {
        let mt = MemTable::new();
        assert_eq!(mt.get(b"nonexistent"), None);
    }

    #[test]
    fn test_overwrite() {
        let mut mt = MemTable::new();
        mt.put(b"key1".to_vec(), b"v1".to_vec());
        mt.put(b"key1".to_vec(), b"v2".to_vec());
        assert_eq!(mt.get(b"key1"), Some(&Some(b"v2".to_vec())));
        assert_eq!(mt.len(), 1);
    }

    #[test]
    fn test_delete_tombstone() {
        let mut mt = MemTable::new();
        mt.put(b"key1".to_vec(), b"value1".to_vec());
        mt.delete(b"key1".to_vec());
        assert_eq!(mt.get(b"key1"), Some(&None));
        assert_eq!(mt.len(), 1);
    }

    #[test]
    fn test_delete_nonexistent_key() {
        let mut mt = MemTable::new();
        mt.delete(b"ghost".to_vec());
        assert_eq!(mt.get(b"ghost"), Some(&None));
        assert_eq!(mt.len(), 1);
    }

    #[test]
    fn test_is_empty_and_len() {
        let mut mt = MemTable::new();
        assert!(mt.is_empty());
        assert_eq!(mt.len(), 0);

        mt.put(b"a".to_vec(), b"1".to_vec());
        assert!(!mt.is_empty());
        assert_eq!(mt.len(), 1);

        mt.put(b"b".to_vec(), b"2".to_vec());
        assert_eq!(mt.len(), 2);
    }

    #[test]
    fn test_approximate_size() {
        let mut mt = MemTable::new();
        assert_eq!(mt.approximate_size(), 0);

        mt.put(b"key".to_vec(), b"value".to_vec());
        assert_eq!(mt.approximate_size(), 8);

        mt.put(b"key".to_vec(), b"v".to_vec());
        assert_eq!(mt.approximate_size(), 4);

        mt.delete(b"key".to_vec());
        assert_eq!(mt.approximate_size(), 3);
    }

    #[test]
    fn test_size_after_delete_nonexistent() {
        let mut mt = MemTable::new();
        mt.delete(b"abc".to_vec());
        assert_eq!(mt.approximate_size(), 3);
    }

    #[test]
    fn test_entries_sorted_order() {
        let mut mt = MemTable::new();
        mt.put(b"charlie".to_vec(), b"3".to_vec());
        mt.put(b"alpha".to_vec(), b"1".to_vec());
        mt.put(b"bravo".to_vec(), b"2".to_vec());

        let keys: Vec<&Vec<u8>> = mt.entries().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            vec![&b"alpha".to_vec(), &b"bravo".to_vec(), &b"charlie".to_vec()]
        );
    }

    #[test]
    fn test_drain_empties_table() {
        let mut mt = MemTable::new();
        mt.put(b"x".to_vec(), b"1".to_vec());
        mt.put(b"y".to_vec(), b"2".to_vec());
        mt.delete(b"z".to_vec());

        let drained = mt.drain();
        assert_eq!(drained.len(), 3);
        assert!(mt.is_empty());
        assert_eq!(mt.approximate_size(), 0);

        assert_eq!(drained[0].0, b"x");
        assert_eq!(drained[1].0, b"y");
        assert_eq!(drained[2].0, b"z");
        assert_eq!(drained[2].1, None);
    }

    #[test]
    fn test_put_after_delete_restores_value() {
        let mut mt = MemTable::new();
        mt.put(b"key".to_vec(), b"v1".to_vec());
        mt.delete(b"key".to_vec());
        mt.put(b"key".to_vec(), b"v2".to_vec());
        assert_eq!(mt.get(b"key"), Some(&Some(b"v2".to_vec())));
    }
}
