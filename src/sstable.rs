use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Sorted key-value entry: `(key, Option<value>)` where `None` is a tombstone.
pub type Entry = (Vec<u8>, Option<Vec<u8>>);

/// Metadata about an SSTable file on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SSTableMeta {
    /// Path to the SSTable file.
    pub path: PathBuf,
    /// Number of entries (including tombstones).
    pub entry_count: usize,
    /// Smallest key in this SSTable.
    pub min_key: Vec<u8>,
    /// Largest key in this SSTable.
    pub max_key: Vec<u8>,
    /// Level in the LSM tree (0 = freshly flushed).
    pub level: usize,
}

/// On-disk SSTable format:
///
/// ```text
/// [data block]   — sequential length-prefixed bincode entries
/// [index block]  — bincode-serialized Vec<(key, offset)>
/// [footer]       — u64 LE: byte offset where index block starts
/// ```
///
/// Each data entry is stored as: `[u32 len][bincode (key, Option<value>)]`.
/// Writes a sorted sequence of key-value entries to a new SSTable file.
pub struct SSTableWriter;

impl SSTableWriter {
    /// Flushes `entries` (must already be sorted by key) to a new SSTable file at `path`.
    ///
    /// Returns metadata describing the written table.
    pub fn write(path: &Path, entries: &[Entry], level: usize) -> Result<SSTableMeta> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        // Index: (key, byte_offset) for each entry.
        let mut index: Vec<(Vec<u8>, u64)> = Vec::with_capacity(entries.len());
        let mut offset: u64 = 0;

        // ── Data block ──
        for (key, value) in entries {
            index.push((key.clone(), offset));

            let record: (&[u8], &Option<Vec<u8>>) = (key, value);
            let encoded = bincode::serialize(&record)?;
            let len = encoded.len() as u32;
            writer.write_all(&len.to_le_bytes())?;
            writer.write_all(&encoded)?;
            offset += 4 + encoded.len() as u64;
        }

        // ── Index block ──
        let index_offset = offset;
        let index_bytes = bincode::serialize(&index)?;
        let index_len = index_bytes.len() as u32;
        writer.write_all(&index_len.to_le_bytes())?;
        writer.write_all(&index_bytes)?;

        // ── Footer ──
        writer.write_all(&index_offset.to_le_bytes())?;

        writer.flush()?;

        let meta = SSTableMeta {
            path: path.to_path_buf(),
            entry_count: entries.len(),
            min_key: entries.first().map(|(k, _)| k.clone()).unwrap_or_default(),
            max_key: entries.last().map(|(k, _)| k.clone()).unwrap_or_default(),
            level,
        };

        Ok(meta)
    }
}

// ─────────────────────────────── Reader ───────────────────────────────

/// Reads point-lookups from an SSTable file, keeping only the index in memory.
pub struct SSTableReader {
    path: PathBuf,
    /// In-memory index: sorted list of (key, data_offset).
    index: Vec<(Vec<u8>, u64)>,
    pub meta: SSTableMeta,
}

impl SSTableReader {
    /// Opens an existing SSTable and loads its index into memory.
    pub fn open(path: &Path, level: usize) -> Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Read footer (last 8 bytes): index block offset.
        reader.seek(SeekFrom::End(-8))?;
        let mut footer_buf = [0u8; 8];
        reader.read_exact(&mut footer_buf)?;
        let index_offset = u64::from_le_bytes(footer_buf);

        // Read index block.
        reader.seek(SeekFrom::Start(index_offset))?;
        let mut index_len_buf = [0u8; 4];
        reader.read_exact(&mut index_len_buf)?;
        let index_len = u32::from_le_bytes(index_len_buf) as usize;

        let mut index_bytes = vec![0u8; index_len];
        reader.read_exact(&mut index_bytes)?;
        let index: Vec<(Vec<u8>, u64)> = bincode::deserialize(&index_bytes)?;

        let entry_count = index.len();
        let min_key = index.first().map(|(k, _)| k.clone()).unwrap_or_default();
        let max_key = index.last().map(|(k, _)| k.clone()).unwrap_or_default();

        let meta = SSTableMeta {
            path: path.to_path_buf(),
            entry_count,
            min_key,
            max_key,
            level,
        };

        Ok(Self {
            path: path.to_path_buf(),
            index,
            meta,
        })
    }

    /// Point-lookup for a key.
    ///
    /// Returns:
    /// - `Ok(Some(Some(value)))` — key found with a live value
    /// - `Ok(Some(None))` — key found as a tombstone
    /// - `Ok(None)` — key not in this SSTable
    pub fn get(&self, key: &[u8]) -> Result<Option<Option<Vec<u8>>>> {
        // Binary search the index for this key.
        let pos = self.index.binary_search_by(|(k, _)| k.as_slice().cmp(key));
        let offset = match pos {
            Ok(i) => self.index[i].1,
            Err(_) => return Ok(None),
        };

        // Seek to the data offset and read the record.
        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(offset))?;

        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut payload = vec![0u8; len];
        reader.read_exact(&mut payload)?;

        let (_key, value): (Vec<u8>, Option<Vec<u8>>) = bincode::deserialize(&payload)?;
        Ok(Some(value))
    }

    /// Returns the path to this SSTable file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads all entries from this SSTable in sorted order.
    /// Used during compaction.
    pub fn scan_all(&self) -> Result<Vec<Entry>> {
        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);
        let mut entries = Vec::with_capacity(self.index.len());

        for &(_, offset) in &self.index {
            reader.seek(SeekFrom::Start(offset))?;
            let mut len_buf = [0u8; 4];
            reader.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;

            let mut payload = vec![0u8; len];
            reader.read_exact(&mut payload)?;

            let (key, value): (Vec<u8>, Option<Vec<u8>>) = bincode::deserialize(&payload)?;
            entries.push((key, value));
        }

        Ok(entries)
    }

    /// Deletes this SSTable file from disk.
    pub fn remove_file(&self) -> Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_entries() -> Vec<Entry> {
        vec![
            (b"alpha".to_vec(), Some(b"1".to_vec())),
            (b"bravo".to_vec(), Some(b"2".to_vec())),
            (b"charlie".to_vec(), None), // tombstone
            (b"delta".to_vec(), Some(b"4".to_vec())),
        ]
    }

    #[test]
    fn test_write_and_read_back() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.sst");
        let entries = sample_entries();

        let meta = SSTableWriter::write(&path, &entries, 0).unwrap();
        assert_eq!(meta.entry_count, 4);
        assert_eq!(meta.min_key, b"alpha");
        assert_eq!(meta.max_key, b"delta");
        assert_eq!(meta.level, 0);

        let reader = SSTableReader::open(&path, 0).unwrap();
        assert_eq!(reader.get(b"alpha").unwrap(), Some(Some(b"1".to_vec())));
        assert_eq!(reader.get(b"bravo").unwrap(), Some(Some(b"2".to_vec())));
        assert_eq!(reader.get(b"delta").unwrap(), Some(Some(b"4".to_vec())));
    }

    #[test]
    fn test_point_lookup_miss() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("miss.sst");
        SSTableWriter::write(&path, &sample_entries(), 0).unwrap();
        let reader = SSTableReader::open(&path, 0).unwrap();

        assert_eq!(reader.get(b"nonexistent").unwrap(), None);
    }

    #[test]
    fn test_tombstone_handling() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tomb.sst");
        SSTableWriter::write(&path, &sample_entries(), 0).unwrap();
        let reader = SSTableReader::open(&path, 0).unwrap();

        assert_eq!(reader.get(b"charlie").unwrap(), Some(None));
    }

    #[test]
    fn test_scan_all() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("scan.sst");
        let entries = sample_entries();
        SSTableWriter::write(&path, &entries, 0).unwrap();
        let reader = SSTableReader::open(&path, 0).unwrap();

        let scanned = reader.scan_all().unwrap();
        assert_eq!(scanned, entries);
    }

    #[test]
    fn test_multiple_sstables_newest_wins() {
        let dir = tempdir().unwrap();

        let old_path = dir.path().join("old.sst");
        let old_entries = vec![(b"alpha".to_vec(), Some(b"old".to_vec()))];
        SSTableWriter::write(&old_path, &old_entries, 0).unwrap();
        let old_reader = SSTableReader::open(&old_path, 0).unwrap();

        let new_path = dir.path().join("new.sst");
        let new_entries = vec![(b"alpha".to_vec(), Some(b"new".to_vec()))];
        SSTableWriter::write(&new_path, &new_entries, 0).unwrap();
        let new_reader = SSTableReader::open(&new_path, 0).unwrap();

        // Simulate engine read: check newest first.
        let readers = [&new_reader, &old_reader];
        let mut result = None;
        for reader in &readers {
            if let Some(val) = reader.get(b"alpha").unwrap() {
                result = Some(val);
                break;
            }
        }
        assert_eq!(result, Some(Some(b"new".to_vec())));
    }

    #[test]
    fn test_remove_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("removable.sst");
        SSTableWriter::write(&path, &sample_entries(), 0).unwrap();
        let reader = SSTableReader::open(&path, 0).unwrap();

        assert!(path.exists());
        reader.remove_file().unwrap();
        assert!(!path.exists());
    }
}
