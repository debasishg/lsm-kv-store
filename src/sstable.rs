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
