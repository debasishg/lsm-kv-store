//! # lsm-kv-store
//!
//! A minimal, embeddable, persistent key-value store built on a
//! Log-Structured Merge Tree (LSM tree) in pure Rust.
//!
//! ## Quick start
//!
//! ```no_run
//! use lsm_kv_store::engine::KvStore;
//!
//! let mut store = KvStore::open_default("my_data").unwrap();
//! store.put("hello", "world").unwrap();
//! assert_eq!(store.get("hello").unwrap(), Some(b"world".to_vec()));
//! store.delete("hello").unwrap();
//! assert_eq!(store.get("hello").unwrap(), None);
//! ```

pub mod engine;
pub mod error;
pub mod memtable;
pub mod sstable;
pub mod wal;

pub use error::{KvError, Result};
