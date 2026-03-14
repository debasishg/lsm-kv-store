# LSM KV Store – Ralph Loop Progress Log

This file is auto-appended by Claude Opus 4.6 in Ralph Wiggum mode after each completed task.  
Format: Date/time (IST) | Task completed | Key outcomes & decisions | Any notes/issues

## Progress Entries

### 2026-03-14 — RalphPlanner: Initial Planning Complete
- **PRD.md** updated with architecture diagram, key design decisions, and technical constraints.
- **TASKS.md** created: 21 atomic tasks across 8 phases (Foundation → MemTable → WAL → SSTable → Engine → Compaction → CLI → Polish).
- Task ordering follows strict dependency chain: error types first, then data structures bottom-up, engine integration, compaction, CLI last.
- Key design choices: `Vec<u8>` keys/values internally, `BTreeMap` for MemTable, bincode serialization, size-tiered compaction, atomic manifest writes via write-tmp+rename.
- Ready for RalphExecutor to begin with **T-001** (project structure setup).

### 2026-03-14 — T-001: Set up project structure
- Created `src/lib.rs` with module declarations: error, memtable, wal, sstable, engine.
- Added dependencies to `Cargo.toml`: clap 4 (derive), serde 1 (derive), bincode 1, thiserror 2; dev-deps: tempfile 3, rand 0.8.
- Created placeholder module files: `src/error.rs`, `src/memtable.rs`, `src/wal.rs`, `src/sstable.rs`, `src/engine.rs`.
- `cargo check`, `cargo fmt --check`, `cargo clippy` all pass clean.

### 2026-03-14 — T-002: Implement KvError enum
- Created `KvError` enum in `src/error.rs` with thiserror: Io (from io::Error), Serialization (from bincode::Error), KeyNotFound, CorruptedWal(String), InvalidOperation(String).
- Added `Result<T>` type alias. Re-exported `KvError` and `Result` from `lib.rs`.

### 2026-03-14 — T-003: Implement MemTable
- Implemented `MemTable` in `src/memtable.rs`: BTreeMap-backed, with put/get/delete/is_empty/len/approximate_size/entries/drain methods.
- Size tracking: incremental on put/delete, accounts for overwrites and tombstones.
- Added `Default` impl. `drain()` method for flush-to-SSTable path.

### 2026-03-14 — T-004: MemTable unit tests
- Added 11 unit tests covering: put/get, missing key, overwrite, delete (tombstone), delete nonexistent, is_empty/len, approximate_size, sorted iteration, drain, put-after-delete.
- Fixed size tracking bug: overwrite was double-counting key bytes. Fixed by only adjusting value size diff on overwrite.