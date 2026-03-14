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
