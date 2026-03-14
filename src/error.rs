use std::io;

/// Custom error types for the LSM key-value store.
#[derive(Debug, thiserror::Error)]
pub enum KvError {
    /// Wraps `std::io::Error` for file and I/O operations.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Serialization or deserialization failure (bincode).
    #[error("serialization error: {0}")]
    Serialization(#[from] bincode::Error),

    /// The requested key was not found.
    #[error("key not found")]
    KeyNotFound,

    /// The write-ahead log is corrupted or contains invalid data.
    #[error("corrupted WAL: {0}")]
    CorruptedWal(String),

    /// An invalid operation was attempted.
    #[error("invalid operation: {0}")]
    InvalidOperation(String),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, KvError>;
