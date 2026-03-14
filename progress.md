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

### 2026-03-14 — T-005: WAL struct + WalEntry
- Implemented `WalEntry` enum (Put/Delete) with serde derive in `src/wal.rs`.
- Implemented `Wal` struct: open, append (length-prefixed bincode), fsync, recover (graceful truncation handling), reset, remove.
- Using `u32` length prefix + bincode payload format. Incomplete trailing entries silently skipped on recovery.

### 2026-03-14 — T-006: WAL unit tests
- 7 tests: append+recover, empty WAL, nonexistent file, corrupted trailing bytes, truncated payload, reset, close+reopen+append.
- All pass. Graceful degradation on corruption confirmed.

### 2026-03-14 — T-007/T-008: SSTable Writer + Reader
- Implemented `SSTableWriter::write()` and `SSTableReader` in `src/sstable.rs`.
- Format: data block (length-prefixed bincode entries) + index block (bincode Vec<(key, offset)>) + footer (u64 index offset).
- Reader: opens file, loads index into memory, binary search for point lookups.
- Added `Entry` type alias, `scan_all()` for compaction, `remove_file()`, `SSTableMeta`.

### 2026-03-14 — T-009: SSTable tests
- 6 tests: write+read, point lookup miss, tombstone, scan_all, multiple SSTables newest-wins, remove_file.
- All pass. Verified format integrity through write→read round-trip.

### 2026-03-14 — T-010/T-011/T-012/T-013: KvStore Engine + Manifest
- Implemented full `KvStore` engine in `src/engine.rs`: config, memtable, WAL, SSTables, manifest.
- Write path: WAL append + fsync → memtable insert → auto-flush to SSTable on threshold.
- Read path: memtable → SSTables newest-first, tombstone = deleted.
- Manifest: bincode-serialized, atomic save via write-tmp + rename.
- Also included: `list()` method, preliminary compaction (`compact_level`), `flush()` force-flush.
- All 24 existing tests still pass. Engine compiles clippy-clean.

### 2026-03-14 — T-015/T-016: Compaction + tests
- Compaction was implemented as part of the engine (T-010). Fixed a critical bug: compacted SSTable reader was pushed to end of readers list (checked last), causing newest overwrites to be hidden by older compacted SSTables. Fix: `insert(0, reader)` instead of `push(reader)`.
- 4 compaction integration tests: triggered by many flushes, preserves overwrites, handles tombstones, data integrity after reopen. All 39 tests pass.

### 2026-03-14 — T-014: KvStore integration tests
- Created `tests/integration.rs` with 11 tests: put/get round-trip, overwrite, delete (tombstone), delete nonexistent, persistence across reopen (flushed), WAL recovery (unflushed), overwrite persistence, tombstone persistence, list (live entries only), flush+read from SSTable, many writes/reads (100 KV pairs with small threshold).
- All pass including the stress test with 100 entries and forced flushes.

### T-017: CLI with clap
- Implemented `src/main.rs` with clap derive: subcommands `put`, `get`, `delete`, `list`.
- Configurable `--db-path` flag (default: `lsm_data`). Error handling via `run()` → `main()` pattern.
- Smoke tested all commands successfully.

### T-018: CLI integration tests
- Created `tests/cli_integration.rs` with 6 tests using `std::process::Command` against the built binary.
- Tests: put+get, get missing key, delete+get, list (sorted), list empty, overwrite.

### T-019: Doc comments
- All public types and methods already had doc comments from implementation phases.
- Added crate-level doc comment (`//!`) to `src/lib.rs` with a `no_run` usage example (generates a doc-test).

### T-020: Benchmarks
- Created `benches/throughput.rs` (run as `[[test]]`): writes 10k random KV pairs, reopens, reads all back.
- Initial result: ~216 writes/sec due to per-write `fsync()`. Added `sync_writes` config to `KvStoreConfig` (default true).
- With `sync_writes = false` (batch WAL flushes): ~444k writes/sec in release mode, ~63k reads/sec. Well above the 5k target.
- Flush still fsyncs the WAL before writing the SSTable, so data is durable at memtable-flush boundaries.

### T-021: Final polish + README
- Created README.md with features, quick start (library + CLI), configuration, test/bench commands, architecture diagram.
- All 47 tests passing. `cargo fmt` and `cargo clippy --all-targets -- -D warnings` clean. All 21 TASKS.md items checked.

**MISSION COMPLETE — LSM KV Store MVP achieved.**