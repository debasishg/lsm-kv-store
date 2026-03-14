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
        let added = key.len() + value.len();
        if let Some(old) = self.entries.insert(key, Some(value)) {
            // Subtract the old value size (key is unchanged).
            let removed = old.as_ref().map_or(0, Vec::len);
            self.size = self.size + added - removed;
        } else {
            self.size += added;
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
