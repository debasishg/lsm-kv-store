# TASKS.md — LSM KV Store MVP

All tasks are atomic, ordered by dependency. Execute top-down; one per iteration.

---

## Phase 1: Foundation

- [x] **T-001** Set up project structure: create `src/lib.rs` with module declarations (`error`, `memtable`, `wal`, `sstable`, `engine`), add dependencies to `Cargo.toml` (clap, serde, bincode, thiserror; tempfile + rand as dev-deps), ensure `cargo check` passes
- [x] **T-002** Implement `KvError` enum in `src/error.rs` using `thiserror`: variants for Io, Serialization, KeyNotFound, CorruptedWal, InvalidOperation; derive Debug/Clone where appropriate; re-export from `lib.rs`

## Phase 2: MemTable

- [x] **T-003** Implement `MemTable` struct in `src/memtable.rs`: `BTreeMap<Vec<u8>, Option<Vec<u8>>>` (None = tombstone), methods `put(&mut self, key, value)`, `get(&self, key) → Option<Option<Vec<u8>>>`, `delete(&mut self, key)`, `is_empty()`, `len()`, `approximate_size()`, `entries() → iterator`
- [x] **T-004** Write unit tests for MemTable: insert, overwrite, delete (tombstone), get missing key, size tracking, iteration order

## Phase 3: Write-Ahead Log

- [x] **T-005** Define `WalEntry` enum (Put { key, value } | Delete { key }) with serde Serialize/Deserialize; implement `Wal` struct in `src/wal.rs` with `open(path)`, `append(entry) → Result`, `fsync()`, `recover(path) → Vec<WalEntry>`
- [x] **T-006** Write unit tests for WAL: append entries, close & reopen, recover all entries in order, handle empty WAL, handle corrupted trailing bytes gracefully

## Phase 4: SSTable

- [ ] **T-007** Implement `SSTableWriter` in `src/sstable.rs`: takes sorted entries from MemTable, writes data block (length-prefixed bincode key-value pairs) + index block (key → byte offset) + footer (index offset), returns SSTable metadata (file path, entry count, min/max key)
- [ ] **T-008** Implement `SSTableReader` in `src/sstable.rs`: `open(path)` loads index into memory, `get(key) → Result<Option<Option<Vec<u8>>>>` does binary search on index then point read from data block
- [ ] **T-009** Write unit tests for SSTable: write then read back entries, point lookup hit/miss, tombstone handling, multiple SSTables with overlapping keys (newest wins)

## Phase 5: KvStore Engine

- [ ] **T-010** Implement `KvStore` struct in `src/engine.rs`: holds MemTable + WAL + list of SSTableReaders + config (memtable size threshold, db directory path); constructor `open(path) → Result<KvStore>` creates db dir, opens/recovers WAL, loads existing SSTables from manifest
- [ ] **T-011** Implement write path: `put(key, value)` and `delete(key)` — append to WAL, insert into MemTable; if MemTable exceeds threshold, flush to new SSTable, clear MemTable, reset WAL
- [ ] **T-012** Implement read path: `get(key) → Result<Option<String>>` — check MemTable first, then SSTables newest-to-oldest; first match wins; tombstone (None) means deleted → return None
- [ ] **T-013** Implement manifest file: simple metadata file tracking list of active SSTable files and their levels; written atomically (write-tmp + rename) on flush and compaction
- [ ] **T-014** Write integration tests for KvStore: put/get/delete round-trip, persistence across drop+reopen, overwrite semantics, tombstone semantics, WAL recovery after crash simulation

## Phase 6: Compaction

- [ ] **T-015** Implement size-tiered compaction in `src/engine.rs` or `src/compaction.rs`: when level N has ≥ threshold SSTables, merge them into one SSTable at level N+1; during merge, keep newest value for each key, drop tombstones that have no older references; update manifest; delete old SSTable files
- [ ] **T-016** Write compaction tests: multiple flushes trigger compaction, data integrity after compaction, tombstones correctly removed, manifest updated

## Phase 7: CLI

- [ ] **T-017** Implement CLI in `src/main.rs` with clap: subcommands `put <key> <value>`, `get <key>`, `delete <key>`, `list`; each opens KvStore at a default/configurable path, performs operation, prints result
- [ ] **T-018** Write integration tests for CLI: test via `std::process::Command` or by testing the underlying KvStore operations; cover put+get, delete+get (shows deleted), list output

## Phase 8: Polish & Benchmarks

- [ ] **T-019** Add doc comments (`///`) to all public types and methods across all modules
- [ ] **T-020** Write a simple benchmark (in `benches/` or as a test): write 10,000 random KV pairs, restart, read all back; verify ≥ 5k writes/sec on local machine; print timing results
- [ ] **T-021** Final pass: `cargo fmt`, `cargo clippy --all-targets -- -D warnings` clean, all tests green, README.md with usage examples

---

**Total: 21 tasks across 8 phases**
