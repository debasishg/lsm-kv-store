# LSM Tree Key-Value Store (lsm-kv-store)

## Overview
Build a simple, embeddable, persistent key-value store using a Log-Structured Merge Tree (LSM tree) in pure Rust.  
Inspired by LevelDB / RocksDB basics, but minimal: no replication, no transactions beyond basic atomic writes, no compaction throttling.

## Goals
- Correctness first: durability via WAL, no data loss on crash (assuming fsync or similar)
- Write-heavy friendly: fast append-only writes
- Reasonable read performance for < 100k entries
- CLI interface for basic usage and testing
- Educational / learning project — clean, idiomatic Rust code with good tests

## Non-Goals (out of scope for v1)
- Multi-threaded concurrent access
- Range scans / iterators (just point get)
- Compression
- Bloom filters
- Snapshots / MVCC

## Functional Requirements
- Support String keys and String values (UTF-8)
- Operations: put(key, value), get(key) → Option<String>, delete(key) (tombstone)
- Persistence: survive process restart
- CLI commands:
  - put <key> <value>
  - get <key>
  - delete <key>
  - list (optional, dump all live entries)
- Basic compaction to prevent unbounded growth

## Success Criteria
- All unit + integration tests pass (≥ 90% coverage on core logic)
- Can write 10,000 random key-value pairs, restart, and read them back correctly
- No segfaults / panics in normal operation
- cargo clippy clean, cargo fmt applied
- Reasonable performance: ≥ 5k writes/sec on consumer SSD (measured via simple bench)

## Constraints
- Use only std + small crates (clap, serde/bincode/tempfile/rand for tests)
- No external DB crates (build from scratch for learning)

## Architecture

```
CLI (clap)
  │
  ▼
KvStore Engine
  ├── MemTable (BTreeMap<Vec<u8>, Option<Vec<u8>>>)  ← in-memory, sorted
  ├── WAL (append-only bincode log, fsync)            ← crash recovery
  ├── SSTables (sorted immutable on-disk files)       ← persistent storage
  └── Compaction (size-tiered merge across levels)    ← space reclamation
```

**Write path**: WAL append → MemTable insert → flush to SSTable when threshold hit  
**Read path**: MemTable → SSTables (newest → oldest), first match wins  
**Delete**: Write tombstone (key → None) through same write path  
**Recovery**: Replay WAL on startup to rebuild MemTable  

## Key Design Decisions

- **Keys & values**: `Vec<u8>` internally, `String` at the CLI boundary
- **Serialization**: `bincode` for WAL entries and SSTable data blocks
- **MemTable threshold**: configurable, default 4 MB
- **SSTable format**: data block (sorted key-value pairs) + index block (key → offset)
- **Compaction trigger**: when a level has ≥ N SSTables, merge into next level
- **Tombstone cleanup**: tombstones removed during compaction when no older SSTable contains the key
- **Manifest/metadata**: simple JSON or bincode file tracking active SSTables and their levels

## Technical Constraints

- Language: Rust 2024 edition, stable toolchain
- Dependencies: clap, serde, bincode, thiserror; tempfile + rand for dev only
- Testing: `#[cfg(test)]` inline + `tests/` integration; target ≥ 90% coverage on core logic
- Style: `cargo fmt` + `cargo clippy --all-targets -- -D warnings` clean

## Out of Scope (v1)

- Multi-threaded concurrent access
- Range scans / iterators
- Compression / bloom filters
- Snapshots / MVCC
- WAL truncation after SSTable flush (stretch goal)

Version: v0.1 – MVP with single-level SST + basic compaction