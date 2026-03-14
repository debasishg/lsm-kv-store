use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::memtable::MemTable;
use crate::sstable::{SSTableMeta, SSTableReader, SSTableWriter};
use crate::wal::{Wal, WalEntry};

/// Default memtable flush threshold: 4 MB.
const DEFAULT_MEMTABLE_THRESHOLD: usize = 4 * 1024 * 1024;

/// Default: trigger compaction when a level has this many SSTables.
const DEFAULT_COMPACTION_THRESHOLD: usize = 4;

/// Configuration for a `KvStore` instance.
#[derive(Debug, Clone)]
pub struct KvStoreConfig {
    /// Directory where all data files (WAL, SSTables, manifest) are stored.
    pub db_path: PathBuf,
    /// Flush the memtable to an SSTable when its approximate size exceeds this.
    pub memtable_threshold: usize,
    /// Trigger compaction when a level has ≥ this many SSTables.
    pub compaction_threshold: usize,
    /// If `true`, fsync the WAL after every write (durable but slower).
    /// If `false`, WAL writes are buffered and fsynced only on memtable flush
    /// (higher throughput, but recent writes may be lost on crash).
    pub sync_writes: bool,
}

impl KvStoreConfig {
    /// Creates a config with defaults and the given database directory.
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            memtable_threshold: DEFAULT_MEMTABLE_THRESHOLD,
            compaction_threshold: DEFAULT_COMPACTION_THRESHOLD,
            sync_writes: true,
        }
    }
}

// ─────────────────────────── Manifest ───────────────────────────

/// Persistent metadata: tracks active SSTable files and their levels.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Manifest {
    sstables: Vec<SSTableMeta>,
}

impl Manifest {
    fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read(path)?;
        let manifest: Self = bincode::deserialize(&data)?;
        Ok(manifest)
    }

    /// Atomically writes the manifest (write-tmp + rename).
    fn save(&self, path: &Path) -> Result<()> {
        let data = bincode::serialize(self)?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &data)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

// ─────────────────────────── KvStore ───────────────────────────

/// The main key-value store engine.
///
/// Ties together the MemTable, WAL, SSTables, and manifest.
pub struct KvStore {
    config: KvStoreConfig,
    memtable: MemTable,
    wal: Wal,
    /// SSTable readers, ordered newest-first.
    sstables: Vec<SSTableReader>,
    manifest: Manifest,
    /// Monotonic counter for generating unique SSTable filenames.
    next_sst_id: u64,
}

impl KvStore {
    /// Opens (or creates) a KvStore at the given directory.
    ///
    /// - Creates the directory if it doesn't exist.
    /// - Loads the manifest and opens existing SSTables.
    /// - Recovers any WAL entries into the memtable.
    pub fn open(config: KvStoreConfig) -> Result<Self> {
        fs::create_dir_all(&config.db_path)?;

        let manifest_path = config.db_path.join("MANIFEST");
        let manifest = Manifest::load(&manifest_path)?;

        // Open existing SSTable readers (newest first for read priority).
        let mut sstables = Vec::new();
        for meta in manifest.sstables.iter().rev() {
            if meta.path.exists() {
                let reader = SSTableReader::open(&meta.path, meta.level)?;
                sstables.push(reader);
            }
        }

        // Derive next_sst_id from existing SSTable filenames.
        let next_sst_id = Self::derive_next_sst_id(&manifest);

        // Open and recover WAL.
        let wal_path = config.db_path.join("wal.log");
        let wal_entries = Wal::recover(&wal_path)?;
        let mut memtable = MemTable::new();
        for entry in wal_entries {
            match entry {
                WalEntry::Put { key, value } => memtable.put(key, value),
                WalEntry::Delete { key } => memtable.delete(key),
            }
        }
        let wal = Wal::open(&wal_path)?;

        Ok(Self {
            config,
            memtable,
            wal,
            sstables,
            manifest,
            next_sst_id,
        })
    }

    /// Convenience: open with default config at the given path.
    pub fn open_default(db_path: impl Into<PathBuf>) -> Result<Self> {
        Self::open(KvStoreConfig::new(db_path))
    }

    // ── Write path ──

    /// Inserts or updates a key-value pair.
    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Result<()> {
        let key = key.into();
        let value = value.into();
        self.wal.append(&WalEntry::Put {
            key: key.clone(),
            value: value.clone(),
        })?;
        if self.config.sync_writes {
            self.wal.fsync()?;
        }
        self.memtable.put(key, value);
        self.maybe_flush()?;
        Ok(())
    }

    /// Deletes a key by writing a tombstone.
    pub fn delete(&mut self, key: impl Into<Vec<u8>>) -> Result<()> {
        let key = key.into();
        self.wal.append(&WalEntry::Delete { key: key.clone() })?;
        if self.config.sync_writes {
            self.wal.fsync()?;
        }
        self.memtable.delete(key);
        self.maybe_flush()?;
        Ok(())
    }

    // ── Read path ──

    /// Looks up a key, returning `Some(value)` or `None` if not found/deleted.
    ///
    /// Check order: MemTable → SSTables (newest first).
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        let key = key.as_ref();

        // 1. Check memtable.
        if let Some(entry) = self.memtable.get(key) {
            return match entry {
                Some(value) => Ok(Some(value.clone())),
                None => Ok(None), // tombstone
            };
        }

        // 2. Check SSTables (already ordered newest-first).
        for reader in &self.sstables {
            if let Some(entry) = reader.get(key)? {
                return match entry {
                    Some(value) => Ok(Some(value)),
                    None => Ok(None), // tombstone
                };
            }
        }

        Ok(None)
    }

    /// Lists all live key-value pairs. Returns them sorted by key.
    pub fn list(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        use std::collections::BTreeMap;
        let mut merged: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();

        // SSTables oldest-first so newer entries overwrite older.
        for reader in self.sstables.iter().rev() {
            for (k, v) in reader.scan_all()? {
                merged.insert(k, v);
            }
        }

        // MemTable is newest — overwrites everything.
        for (k, v) in self.memtable.entries() {
            merged.insert(k.clone(), v.clone());
        }

        // Filter out tombstones.
        Ok(merged
            .into_iter()
            .filter_map(|(k, v)| v.map(|val| (k, val)))
            .collect())
    }

    // ── Flush ──

    /// Flushes the memtable to a new SSTable if the size threshold is exceeded.
    fn maybe_flush(&mut self) -> Result<()> {
        if self.memtable.approximate_size() < self.config.memtable_threshold {
            return Ok(());
        }
        self.flush()?;
        Ok(())
    }

    /// Forces the memtable to be flushed to a new SSTable.
    pub fn flush(&mut self) -> Result<()> {
        if self.memtable.is_empty() {
            return Ok(());
        }

        // Ensure WAL is durable before flushing (important when sync_writes is off).
        self.wal.fsync()?;

        let entries = self.memtable.drain();
        let sst_path = self.next_sst_path(0);
        let meta = SSTableWriter::write(&sst_path, &entries, 0)?;

        // Open reader for the new SSTable and prepend (newest-first).
        let reader = SSTableReader::open(&sst_path, 0)?;
        self.sstables.insert(0, reader);

        // Update manifest.
        self.manifest.sstables.push(meta);
        self.save_manifest()?;

        // Reset WAL.
        self.wal.reset()?;

        // Trigger compaction if needed.
        self.maybe_compact()?;

        Ok(())
    }

    // ── Compaction ──

    /// Checks if compaction is needed and runs it.
    fn maybe_compact(&mut self) -> Result<()> {
        // Count SSTables per level.
        let max_level = self
            .manifest
            .sstables
            .iter()
            .map(|m| m.level)
            .max()
            .unwrap_or(0);
        for level in 0..=max_level {
            let count = self
                .manifest
                .sstables
                .iter()
                .filter(|m| m.level == level)
                .count();
            if count >= self.config.compaction_threshold {
                self.compact_level(level)?;
            }
        }
        Ok(())
    }

    /// Merges all SSTables at `level` into one SSTable at `level + 1`.
    fn compact_level(&mut self, level: usize) -> Result<()> {
        use std::collections::BTreeMap;

        // Collect paths of SSTables to compact.
        let to_compact: Vec<PathBuf> = self
            .manifest
            .sstables
            .iter()
            .filter(|m| m.level == level)
            .map(|m| m.path.clone())
            .collect();

        if to_compact.len() < 2 {
            return Ok(());
        }

        // Merge entries: older SSTables first, so newer entries overwrite.
        let mut merged: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
        for path in &to_compact {
            if let Ok(reader) = SSTableReader::open(path, level) {
                for (k, v) in reader.scan_all()? {
                    merged.insert(k, v);
                }
            }
        }

        // Remove tombstones that have no older SSTable at deeper levels
        // containing the same key. For simplicity in v1, we keep tombstones
        // if there are SSTables at deeper levels; otherwise we can drop them.
        let has_deeper = self.manifest.sstables.iter().any(|m| m.level > level + 1);

        let entries: Vec<(Vec<u8>, Option<Vec<u8>>)> = if has_deeper {
            merged.into_iter().collect()
        } else {
            // Safe to drop tombstones at the deepest compaction target.
            merged.into_iter().filter(|(_, v)| v.is_some()).collect()
        };

        if entries.is_empty() {
            // All entries were tombstones that got cleaned up.
            self.remove_sstables(&to_compact)?;
            return Ok(());
        }

        // Write merged SSTable.
        let new_level = level + 1;
        let new_path = self.next_sst_path(new_level);
        let new_meta = SSTableWriter::write(&new_path, &entries, new_level)?;

        // Remove old SSTable files and readers.
        self.remove_sstables(&to_compact)?;

        // Add new SSTable (newest-first: insert at front).
        let reader = SSTableReader::open(&new_path, new_level)?;
        self.sstables.insert(0, reader);
        self.manifest.sstables.push(new_meta);
        self.save_manifest()?;

        Ok(())
    }

    /// Removes SSTable files and their entries from manifest + readers list.
    fn remove_sstables(&mut self, paths: &[PathBuf]) -> Result<()> {
        // Remove from readers.
        self.sstables
            .retain(|r| !paths.contains(&r.path().to_path_buf()));

        // Remove from manifest.
        self.manifest.sstables.retain(|m| !paths.contains(&m.path));

        // Delete files.
        for path in paths {
            if path.exists() {
                fs::remove_file(path)?;
            }
        }

        Ok(())
    }

    // ── Helpers ──

    fn next_sst_path(&mut self, level: usize) -> PathBuf {
        let id = self.next_sst_id;
        self.next_sst_id += 1;
        self.config.db_path.join(format!("L{level}_{id:06}.sst"))
    }

    fn save_manifest(&self) -> Result<()> {
        let manifest_path = self.config.db_path.join("MANIFEST");
        self.manifest.save(&manifest_path)
    }

    fn derive_next_sst_id(manifest: &Manifest) -> u64 {
        // Parse IDs from existing SSTable filenames, pick max + 1.
        let max_id = manifest
            .sstables
            .iter()
            .filter_map(|m| {
                m.path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.split('_').nth(1))
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0);

        // Also use timestamp as fallback to avoid collisions.
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(max_id + 1);

        max_id.max(ts) + 1
    }
}
