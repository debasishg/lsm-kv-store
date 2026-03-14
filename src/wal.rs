use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{KvError, Result};

/// A single entry in the write-ahead log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WalEntry {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

/// Append-only write-ahead log for crash recovery.
///
/// Each entry is written as a length-prefixed bincode blob:
/// `[u32 length][bincode bytes]`. On recovery, entries are read
/// sequentially; any trailing incomplete entry is silently ignored
/// (crash mid-write).
pub struct Wal {
    path: PathBuf,
    writer: BufWriter<File>,
}

impl Wal {
    /// Opens (or creates) a WAL file at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: BufWriter::new(file),
        })
    }

    /// Appends a single entry to the log.
    pub fn append(&mut self, entry: &WalEntry) -> Result<()> {
        let bytes = bincode::serialize(entry)?;
        let len = bytes.len() as u32;
        self.writer.write_all(&len.to_le_bytes())?;
        self.writer.write_all(&bytes)?;
        Ok(())
    }

    /// Flushes the buffer and calls `fsync` on the underlying file.
    pub fn fsync(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_all()?;
        Ok(())
    }

    /// Recovers all valid entries from a WAL file.
    ///
    /// Silently stops at the first incomplete or corrupted trailing entry
    /// (this is expected after a crash mid-write).
    pub fn recover(path: &Path) -> Result<Vec<WalEntry>> {
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();

        loop {
            // Read the 4-byte length prefix.
            let mut len_buf = [0u8; 4];
            match reader.read_exact(&mut len_buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(KvError::Io(e)),
            }
            let len = u32::from_le_bytes(len_buf) as usize;

            // Read the entry payload.
            let mut payload = vec![0u8; len];
            match reader.read_exact(&mut payload) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(KvError::Io(e)),
            }

            match bincode::deserialize::<WalEntry>(&payload) {
                Ok(entry) => entries.push(entry),
                Err(_) => break, // corrupted entry — stop recovery
            }
        }

        Ok(entries)
    }

    /// Resets the WAL by truncating it to zero length and reopening.
    pub fn reset(&mut self) -> Result<()> {
        self.writer.flush()?;
        let file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        self.writer = BufWriter::new(file);
        Ok(())
    }

    /// Returns the path of this WAL file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Deletes the WAL file from disk.
    pub fn remove(self) -> Result<()> {
        drop(self.writer);
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

    fn put_entry(key: &[u8], value: &[u8]) -> WalEntry {
        WalEntry::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }

    fn delete_entry(key: &[u8]) -> WalEntry {
        WalEntry::Delete { key: key.to_vec() }
    }

    #[test]
    fn test_append_and_recover() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        {
            let mut wal = Wal::open(&wal_path).unwrap();
            wal.append(&put_entry(b"k1", b"v1")).unwrap();
            wal.append(&put_entry(b"k2", b"v2")).unwrap();
            wal.append(&delete_entry(b"k1")).unwrap();
            wal.fsync().unwrap();
        }

        let entries = Wal::recover(&wal_path).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], put_entry(b"k1", b"v1"));
        assert_eq!(entries[1], put_entry(b"k2", b"v2"));
        assert_eq!(entries[2], delete_entry(b"k1"));
    }

    #[test]
    fn test_recover_empty_wal() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("empty.wal");

        {
            let wal = Wal::open(&wal_path).unwrap();
            drop(wal);
        }

        let entries = Wal::recover(&wal_path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_recover_nonexistent_file() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("does_not_exist.wal");

        let entries = Wal::recover(&wal_path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_recover_corrupted_trailing_bytes() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("corrupt.wal");

        {
            let mut wal = Wal::open(&wal_path).unwrap();
            wal.append(&put_entry(b"good", b"data")).unwrap();
            wal.fsync().unwrap();
        }

        // Append garbage bytes (incomplete length prefix).
        {
            let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
            file.write_all(&[0xFF, 0xFF]).unwrap();
        }

        let entries = Wal::recover(&wal_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], put_entry(b"good", b"data"));
    }

    #[test]
    fn test_recover_truncated_payload() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("truncated.wal");

        {
            let mut wal = Wal::open(&wal_path).unwrap();
            wal.append(&put_entry(b"ok", b"fine")).unwrap();
            wal.fsync().unwrap();
        }

        {
            let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
            let len: u32 = 100;
            file.write_all(&len.to_le_bytes()).unwrap();
            file.write_all(b"short").unwrap();
        }

        let entries = Wal::recover(&wal_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], put_entry(b"ok", b"fine"));
    }

    #[test]
    fn test_reset_clears_wal() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("reset.wal");

        let mut wal = Wal::open(&wal_path).unwrap();
        wal.append(&put_entry(b"a", b"1")).unwrap();
        wal.append(&put_entry(b"b", b"2")).unwrap();
        wal.fsync().unwrap();

        wal.reset().unwrap();

        let entries = Wal::recover(&wal_path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_close_reopen_append() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("reopen.wal");

        {
            let mut wal = Wal::open(&wal_path).unwrap();
            wal.append(&put_entry(b"k1", b"v1")).unwrap();
            wal.fsync().unwrap();
        }

        {
            let mut wal = Wal::open(&wal_path).unwrap();
            wal.append(&put_entry(b"k2", b"v2")).unwrap();
            wal.fsync().unwrap();
        }

        let entries = Wal::recover(&wal_path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], put_entry(b"k1", b"v1"));
        assert_eq!(entries[1], put_entry(b"k2", b"v2"));
    }
}
