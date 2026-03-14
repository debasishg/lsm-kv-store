# lsm-kv-store

A minimal, embeddable, persistent key-value store built on a **Log-Structured Merge Tree** (LSM tree) in pure Rust.

## Features

- **MemTable** — in-memory `BTreeMap` with tombstone support for deletes
- **Write-Ahead Log (WAL)** — append-only, length-prefixed bincode entries for crash recovery
- **SSTables** — sorted, immutable on-disk files with in-memory index and binary-search lookups
- **Size-tiered compaction** — automatic merging of SSTables across levels
- **Configurable durability** — per-write `fsync` (safe) or batched flushes (fast)
- **CLI** — `put`, `get`, `delete`, `list` subcommands via clap

## Quick start

### As a library

```rust
use lsm_kv_store::engine::KvStore;

let mut store = KvStore::open_default("my_data").unwrap();
store.put("hello", "world").unwrap();
assert_eq!(store.get("hello").unwrap(), Some(b"world".to_vec()));
store.delete("hello").unwrap();
assert_eq!(store.get("hello").unwrap(), None);
```

### Via the CLI

```bash
cargo run -- put greeting hello
cargo run -- get greeting        # prints: hello
cargo run -- list                # prints: greeting    hello
cargo run -- delete greeting
cargo run -- get greeting        # prints: Key not found
```

Use `--db-path <DIR>` to specify a custom data directory (default: `lsm_data`).

## Configuration

```rust
use lsm_kv_store::engine::{KvStore, KvStoreConfig};

let mut config = KvStoreConfig::new("my_data");
config.memtable_threshold = 1024 * 1024;  // flush at 1 MB
config.compaction_threshold = 4;           // compact when 4+ SSTables at a level
config.sync_writes = false;               // batch WAL flushes (faster, less durable)
let mut store = KvStore::open(config).unwrap();
```

## Running tests

```bash
cargo test
```

## Running the throughput benchmark

```bash
cargo test --test throughput --release -- --nocapture
```

## Architecture

```
put/delete ──► WAL (append) ──► MemTable (BTreeMap)
                                    │
                          (threshold exceeded)
                                    ▼
                              SSTable (L0)
                                    │
                          (4+ SSTables at level)
                                    ▼
                            Compact → SSTable (L1)
                                    │
                                   ...

get ──► MemTable ──► SSTables (newest first)
```

## License

This project is for educational purposes.
