# Architecture — lsm-kv-store

This document describes the internal architecture of `lsm-kv-store`, a minimal persistent key-value store built on a **Log-Structured Merge Tree (LSM tree)** in pure Rust.

---

## 1. High-Level Overview

```
                          ┌──────────────┐
                          │   CLI / API   │  src/main.rs
                          └──────┬───────┘
                                 │
                          ┌──────▼───────┐
                          │   KvStore    │  src/engine.rs
                          │   (Engine)   │
                          └──┬───┬───┬───┘
                             │   │   │
              ┌──────────────┘   │   └──────────────┐
              │                  │                   │
       ┌──────▼──────┐   ┌──────▼──────┐   ┌───────▼───────┐
       │   MemTable   │   │     WAL     │   │   SSTables    │
       │  (BTreeMap)  │   │ (append-only│   │ (sorted disk  │
       │              │   │   log file) │   │    files)     │
       └──────────────┘   └─────────────┘   └───────┬───────┘
              src/memtable.rs   src/wal.rs          │
                                             ┌──────▼──────┐
                                             │  Compaction  │
                                             │ (size-tiered │
                                             │   merge)     │
                                             └──────┬───────┘
                                                    │
                                             ┌──────▼──────┐
                                             │  Manifest    │
                                             │ (SSTable     │
                                             │  registry)   │
                                             └─────────────┘
```

The store is **single-threaded** and follows the classic LSM tree design:

1. **Writes** go to the WAL (for durability) and the MemTable (for fast lookups).
2. When the MemTable exceeds a size threshold, it is **flushed** to an immutable SSTable on disk.
3. When too many SSTables accumulate at a level, **compaction** merges them into a single SSTable at the next level.
4. **Reads** check the MemTable first, then SSTables newest-to-oldest; the first match wins.

---

## 2. Module Map

| Module | File | Responsibility |
|--------|------|----------------|
| `error` | `src/error.rs` | `KvError` enum and `Result<T>` type alias |
| `memtable` | `src/memtable.rs` | In-memory sorted buffer (`BTreeMap`) |
| `wal` | `src/wal.rs` | Crash-recovery write-ahead log |
| `sstable` | `src/sstable.rs` | On-disk sorted table writer and reader |
| `engine` | `src/engine.rs` | Orchestrator: ties MemTable + WAL + SSTables + Manifest + Compaction |
| `main` | `src/main.rs` | CLI entry point (clap) |
| `lib` | `src/lib.rs` | Crate root, re-exports |

---

## 3. Data Flow

### 3.1 Write Path (`put` / `delete`)

```
Client
  │
  ▼
KvStore::put(key, value)
  │
  ├─1─► WAL::append(WalEntry::Put { key, value })
  │       └─► [optional] WAL::fsync()          ← controlled by sync_writes
  │
  ├─2─► MemTable::put(key, value)
  │
  └─3─► maybe_flush()
          │
          ├── MemTable size < threshold → return
          │
          └── MemTable size ≥ threshold:
                ├── WAL::fsync()                 ← ensures durability before flush
                ├── MemTable::drain() → sorted entries
                ├── SSTableWriter::write(entries) → L0 SSTable file
                ├── SSTableReader::open() → prepend to readers list
                ├── Manifest::save()             ← atomic write-tmp + rename
                ├── WAL::reset()                 ← truncate to zero
                └── maybe_compact()
```

**Key design decisions:**
- The WAL entry is written *before* the MemTable is updated (write-ahead guarantee).
- `sync_writes = true` (default) calls `fsync()` after every WAL append for full durability. Setting `sync_writes = false` defers `fsync()` to memtable flush time, trading recent-write durability for ~2000x higher throughput.
- Tombstones (deletes) are stored as `None` values in `Option<Vec<u8>>`.

### 3.2 Read Path (`get`)

```
Client
  │
  ▼
KvStore::get(key)
  │
  ├─1─► MemTable::get(key)
  │       ├── Some(Some(value)) → return Ok(Some(value))
  │       ├── Some(None)        → return Ok(None)  (tombstone = deleted)
  │       └── None              → continue to SSTables
  │
  └─2─► for reader in sstables (newest-first):
          SSTableReader::get(key)
            ├── Some(Some(value)) → return Ok(Some(value))
            ├── Some(None)        → return Ok(None)  (tombstone)
            └── None              → try next SSTable
          ...
          All SSTables exhausted → return Ok(None)
```

**Newest-first ordering** guarantees that the most recent write for any key is found first, without needing to scan all SSTables.

### 3.3 List Path

The `list()` operation performs a full merge across all SSTables (oldest-first) and the MemTable, then filters out tombstones. This is an O(N) scan over all data.

---

## 4. Component Details

### 4.1 MemTable (`src/memtable.rs`)

```
┌─────────────────────────────────────┐
│            MemTable                 │
├─────────────────────────────────────┤
│  entries: BTreeMap<Vec<u8>,         │
│           Option<Vec<u8>>>          │
│  size: usize  (approx byte count)  │
├─────────────────────────────────────┤
│  put(key, value)                    │
│  get(key) → Option<&Option<Vec>>    │
│  delete(key)       (inserts None)   │
│  drain() → Vec<(key, Option<val>)>  │
│  approximate_size() → usize        │
│  is_empty() / len() / entries()     │
└─────────────────────────────────────┘
```

- Backed by `BTreeMap` — entries are always sorted by key.
- **Size tracking** is incremental: `put` adds key+value bytes (adjusting for overwrites), `delete` subtracts the old value bytes.
- `drain()` consumes and returns all entries for flushing to an SSTable, resetting size to zero.
- No concurrency primitives — single-threaded access only.

### 4.2 Write-Ahead Log (`src/wal.rs`)

```
┌──────────────────────────────────────┐
│               WAL                    │
├──────────────────────────────────────┤
│  path: PathBuf                       │
│  writer: BufWriter<File>             │
├──────────────────────────────────────┤
│  open(path) → Wal                    │
│  append(entry) → Result              │
│  fsync()                             │
│  recover(path) → Vec<WalEntry>       │
│  reset()           (truncate)        │
│  remove()          (delete file)     │
└──────────────────────────────────────┘
```

**On-disk format** (per entry):

```
┌─────────────┬──────────────────────┐
│ u32 LE len  │  bincode(WalEntry)   │
│  (4 bytes)  │  (variable length)   │
└─────────────┴──────────────────────┘
```

- `WalEntry` is an enum: `Put { key: Vec<u8>, value: Vec<u8> }` or `Delete { key: Vec<u8> }`.
- **Recovery** reads entries sequentially. An incomplete or corrupted trailing entry (crash mid-write) is silently ignored — this is safe because the MemTable update happens *after* the WAL append.
- `reset()` truncates the file after a successful memtable flush.

### 4.3 SSTable (`src/sstable.rs`)

**On-disk format:**

```
┌──────────────────────────────────────────────────────┐
│ Data Block                                           │
│  ┌─────────┬──────────────────────────────┐          │
│  │ u32 len │ bincode( (key, Option<val>) )│ entry 0  │
│  ├─────────┼──────────────────────────────┤          │
│  │ u32 len │ bincode( (key, Option<val>) )│ entry 1  │
│  ├─────────┼──────────────────────────────┤          │
│  │  ...    │            ...               │  ...     │
│  └─────────┴──────────────────────────────┘          │
├──────────────────────────────────────────────────────┤
│ Index Block                                          │
│  ┌─────────┬──────────────────────────────┐          │
│  │ u32 len │ bincode( Vec<(key, offset)> )│          │
│  └─────────┴──────────────────────────────┘          │
├──────────────────────────────────────────────────────┤
│ Footer                                               │
│  ┌──────────────────┐                                │
│  │ u64 LE           │  byte offset of index block    │
│  │  (8 bytes)       │                                │
│  └──────────────────┘                                │
└──────────────────────────────────────────────────────┘
```

- **SSTableWriter**: receives pre-sorted entries from the MemTable (or compaction), writes data block + index block + footer.
- **SSTableReader**: on open, reads the footer to locate the index block, loads the full index into memory. Point lookups use **binary search** on the in-memory index, then a single disk seek + read.
- `scan_all()` reads every entry sequentially (used during compaction).
- **SSTableMeta**: serializable metadata (path, entry count, min/max key, level) stored in the Manifest.

**Filename convention:** `L{level}_{id:06}.sst` (e.g. `L0_000042.sst`).

### 4.4 Engine (`src/engine.rs`)

The `KvStore` struct is the central orchestrator:

```
┌────────────────────────────────────────────┐
│              KvStore                       │
├────────────────────────────────────────────┤
│  config: KvStoreConfig                     │
│  memtable: MemTable                        │
│  wal: Wal                                  │
│  sstables: Vec<SSTableReader>  (newest→)   │
│  manifest: Manifest                        │
│  next_sst_id: u64                          │
├────────────────────────────────────────────┤
│  open(config) / open_default(path)         │
│  put(key, value) / delete(key)             │
│  get(key) → Option<Vec<u8>>                │
│  list() → Vec<(key, value)>                │
│  flush()                                   │
│  [private] maybe_flush()                   │
│  [private] maybe_compact()                 │
│  [private] compact_level(level)            │
└────────────────────────────────────────────┘
```

**Startup sequence (`open`):**
1. Create database directory if missing.
2. Load the `MANIFEST` file (or create empty).
3. Open `SSTableReader` for each recorded SSTable (newest-first order).
4. Derive `next_sst_id` from existing filenames + timestamp.
5. Recover WAL entries into a fresh MemTable.
6. Open the WAL for writing.

### 4.5 Manifest

```
┌──────────────────────────────────┐
│           Manifest               │
├──────────────────────────────────┤
│  sstables: Vec<SSTableMeta>      │
├──────────────────────────────────┤
│  load(path) → Manifest           │
│  save(path)                      │
│    └─ write to MANIFEST.tmp      │
│    └─ rename to MANIFEST         │
└──────────────────────────────────┘
```

- Serialized with bincode.
- **Atomic writes** via the write-tmp + rename pattern — if the process crashes mid-write, the old MANIFEST is still intact.
- Updated after every flush and compaction.

### 4.6 Compaction

**Strategy: size-tiered compaction.**

```
Level 0:  [SST_a] [SST_b] [SST_c] [SST_d]    ← 4 SSTables (threshold reached)
                      │
                  compact_level(0)
                      │
                      ▼
Level 1:          [SST_merged]                 ← 1 merged SSTable
```

- Triggered after each memtable flush when any level has ≥ `compaction_threshold` (default 4) SSTables.
- **Merge process:** read all SSTables at the target level oldest-first (so newer entries overwrite older), merge into a `BTreeMap`, write a single new SSTable at `level + 1`.
- **Tombstone cleanup:** tombstones are dropped during compaction if there are no SSTables at deeper levels (i.e. no older data could be shadowed).
- After compaction: old SSTable files are deleted, new SSTable reader is prepended (newest-first), manifest is updated.

### 4.7 CLI (`src/main.rs`)

```
lsm-kv [--db-path <DIR>] <COMMAND>

Commands:
  put <key> <value>   Insert or update a key-value pair
  get <key>           Retrieve the value for a key
  delete <key>        Delete a key
  list                List all live key-value pairs
```

- Built with `clap` (derive mode).
- Each invocation opens and closes the `KvStore`, so the CLI is stateless between commands.
- Keys and values are treated as UTF-8 strings at the CLI boundary but stored as `Vec<u8>` internally.

### 4.8 Error Handling

All fallible operations return `Result<T, KvError>` using a custom enum:

| Variant | Source | When |
|---------|--------|------|
| `Io` | `std::io::Error` | File I/O failures |
| `Serialization` | `bincode::Error` | Encode/decode failures |
| `KeyNotFound` | — | Explicit key-not-found (unused in current API, reserved) |
| `CorruptedWal` | — | WAL integrity issues |
| `InvalidOperation` | — | Logical errors |

Library code never panics (`unwrap`/`expect` are confined to tests and `main()`).

---

## 5. On-Disk Layout

A database directory contains:

```
my_data/
├── MANIFEST            ← bincode-serialized SSTable registry
├── wal.log             ← active write-ahead log
├── L0_000001.sst       ← level-0 SSTable (freshly flushed)
├── L0_000002.sst
├── L1_000003.sst       ← level-1 SSTable (compaction output)
└── ...
```

---

## 6. Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `db_path` | — (required) | Database directory path |
| `memtable_threshold` | 4 MB | Flush memtable to SSTable when exceeded |
| `compaction_threshold` | 4 | Compact a level when it has ≥ N SSTables |
| `sync_writes` | `true` | `fsync` after every write (`false` = batch flushes) |

---

## 7. Performance Characteristics

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| `put` / `delete` | O(log n) memtable + O(1) WAL append | Amortized by buffered I/O |
| `get` (memtable hit) | O(log n) | BTreeMap lookup |
| `get` (SSTable) | O(log n) per SSTable | Binary search on in-memory index + 1 disk read |
| `get` (worst case) | O(L × log n) | L = number of SSTables checked |
| `list` | O(N) | Full scan of all SSTables + memtable |
| `flush` | O(n log n) | n = memtable entries (already sorted) |
| `compaction` | O(n) | n = total entries across SSTables being merged |

**Benchmarked throughput** (10k random 16-byte keys, 100-byte values, release mode):
- Writes (`sync_writes = false`): ~444,000 ops/sec
- Reads (after reopen): ~63,000 ops/sec

---

## 8. Possible Enhancements

### 8.1 Read Performance

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Bloom filters** | Per-SSTable probabilistic filter to skip disk reads for keys that definitely aren't present. Reduces unnecessary I/O on `get` misses from O(L) to O(1) expected. | Medium |
| **Block cache** | LRU cache for recently-read data blocks. Avoids repeated disk I/O for hot keys. | Medium |
| **Sparse index / block-level index** | Instead of indexing every key, index one key per data block (e.g. every 4 KB). Reduces in-memory index size at the cost of a short sequential scan within a block. | Medium |
| **Memory-mapped I/O (mmap)** | Map SSTable files into virtual memory to leverage the OS page cache and avoid explicit read/seek calls. | Low–Medium |
| **Prefix-compressed keys** | Delta-encode keys within a block (store shared prefix length + suffix). Reduces both disk and memory footprint. | Medium |

### 8.2 Write Performance

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Write batching API** | Expose a `WriteBatch` that groups multiple put/delete operations into a single WAL append + single fsync. Critical for transactional semantics. | Low |
| **Group commit** | Buffer WAL entries from multiple callers and fsync once for the batch (relevant when adding concurrency). | Medium |
| **WAL checksum** | Add CRC32 per WAL entry for corruption detection beyond just length mismatches. | Low |
| **Immutable memtable + double buffering** | When flushing, freeze the current memtable and switch writes to a new one immediately. Flush the frozen memtable in the background, eliminating write stalls. | Medium |

### 8.3 Compaction

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Leveled compaction** (LevelDB/RocksDB style) | Instead of merging all SSTables at a level, pick overlapping SSTables and merge them into the next level. Provides better read amplification and space amplification guarantees. | High |
| **Background compaction** | Run compaction in a separate thread to avoid blocking writes. Requires careful synchronization of SSTable reader lists and the manifest. | High |
| **Compaction scheduling / priority** | Rate-limit compaction I/O to avoid starving foreground reads/writes. Prioritize levels with the most impact. | Medium |
| **Tiered + leveled hybrid** (RocksDB Universal) | Use size-tiered for L0 and leveled for deeper levels, balancing write and read amplification. | High |
| **Tombstone TTL / garbage collection** | Drop tombstones after a configurable time-to-live, even if deeper levels exist. | Low |

### 8.4 Concurrency & Parallelism

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Read-write lock on memtable** | `RwLock<MemTable>` — multiple concurrent readers, exclusive writer. Foundation for multi-threaded access. | Medium |
| **Lock-free skip list memtable** | Replace `BTreeMap` with a concurrent skip list (e.g. `crossbeam-skiplist`) for lock-free reads and writes. | Medium–High |
| **MVCC (Multi-Version Concurrency Control)** | Attach a sequence number to every entry, allowing snapshot reads without blocking writers. | High |
| **Concurrent SSTable readers** | Since SSTables are immutable, reads can proceed concurrently with no locking. The reader list needs an `Arc<Vec<...>>` with atomic swaps on flush/compaction. | Medium |

### 8.5 Durability & Recovery

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **WAL checksums (CRC32)** | Append a checksum per WAL entry. On recovery, validate each entry's integrity rather than relying solely on length-prefix framing. | Low |
| **WAL segmentation** | Split the WAL into fixed-size segments. Old segments can be recycled or archived. Prevents unbounded WAL growth before flush. | Medium |
| **Manifest versioning / WAL** | Instead of overwriting the manifest, append changes (like a WAL for metadata). Enables manifest recovery and version history. | Medium |
| **Snapshots** | Capture a point-in-time consistent view of the store. Useful for backups and consistent reads during compaction. | High |
| **Backup / restore** | Copy SSTable files + manifest to a backup location without blocking the store. SSTables are immutable so this is inherently safe. | Low–Medium |

### 8.6 Data Features

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Range scans / iterators** | Return all key-value pairs in a key range `[start, end)`. Requires merging iterators across MemTable and SSTables. | Medium |
| **Prefix scans** | Efficiently scan all keys with a given prefix. Leverages sorted order in BTreeMap and SSTable index. | Medium |
| **TTL (time-to-live) per key** | Automatically expire keys after a duration. Store expiration timestamp alongside the value; clean up during compaction. | Medium |
| **Column families / namespaces** | Multiple independent key-value spaces in the same database, each with its own MemTable + SSTables. | High |
| **Secondary indexes** | Maintain auxiliary SSTables indexed by value fields. Kept consistent via two-phase write to primary + index. | High |
| **Merge operators** | User-defined read-modify-write operations (e.g. counters, append-to-list) that are applied lazily during compaction. | High |

### 8.7 Compression & Encoding

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Block compression** (LZ4/Zstd/Snappy) | Compress SSTable data blocks. LZ4 for speed, Zstd for ratio. Dramatically reduces disk usage and I/O. | Medium |
| **Dictionary compression** | Train a Zstd dictionary on a sample of keys/values for better compression of small entries. | Medium |
| **Variable-length integer encoding** | Use varint encoding for length prefixes (instead of fixed u32). Saves bytes for small entries. | Low |
| **Alternative serialization** | Replace bincode with `postcard` (no-std, smaller output) or `rkyv` (zero-copy deserialization). | Low–Medium |

### 8.8 Observability & Operations

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Metrics / statistics** | Track read/write latency, cache hit rate, compaction stats, SSTable count per level, disk usage, memtable size. | Medium |
| **Structured logging** | Use `tracing` crate for structured, leveled logs of operations (flush, compact, recover). | Low |
| **Database introspection CLI** | Commands like `info` (show SSTable count, levels, disk usage), `compact` (force compaction), `dump` (export all data). | Low |
| **Prometheus / OpenTelemetry export** | Expose metrics for monitoring dashboards. | Medium |

### 8.9 API & Ergonomics

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Typed value wrapper** | Generic `KvStore<V: Serialize + DeserializeOwned>` that serializes/deserializes values automatically. | Low |
| **Async API** | `async fn put/get/delete` using `tokio::fs` for non-blocking I/O. | Medium–High |
| **Embedded HTTP / gRPC server** | Expose the store over the network for multi-process access. | High |
| **`Drop`-based cleanup** | Implement `Drop` for `KvStore` to flush the memtable and fsync on graceful shutdown. | Low |
| **Builder pattern for config** | `KvStoreConfig::builder().db_path("x").threshold(1024).build()` for ergonomic configuration. | Low |
| **Transactions** | Multi-key atomic read-modify-write operations with optimistic concurrency control. | High |

### 8.10 Testing & Reliability

| Enhancement | Description | Complexity |
|-------------|-------------|------------|
| **Fuzz testing** | Use `cargo-fuzz` or `proptest` to generate random operation sequences and verify invariants (no data loss, no panics). | Medium |
| **Fault injection** | Simulate disk failures, partial writes, and OS crashes (e.g. using `failpoints` crate) to test recovery paths. | Medium–High |
| **Deterministic simulation** | Replace real I/O with an in-memory simulated filesystem for reproducible, fast testing under adversarial conditions. | High |
| **Benchmark suite** | Systematic benchmarks: sequential writes, random writes, sequential reads, random reads, mixed workloads, varying key/value sizes. | Medium |

---

## 9. Dependency Graph

```
src/lib.rs
    ├── src/error.rs          (standalone — no internal deps)
    ├── src/memtable.rs       (standalone — uses only std)
    ├── src/wal.rs            (depends on: error)
    ├── src/sstable.rs        (depends on: error)
    └── src/engine.rs         (depends on: error, memtable, wal, sstable)

src/main.rs                   (depends on: engine via lib crate)

External crates:
    clap 4        — CLI parsing (main.rs only)
    serde 1       — serialization derives
    bincode 1     — binary serialization (WAL, SSTable, Manifest)
    thiserror 2   — error enum derive
    tempfile 3    — test utility (dev-dependency)
    rand 0.8      — test utility (dev-dependency)
```

---

## 10. Test Coverage

| Category | Count | Location |
|----------|-------|----------|
| MemTable unit tests | 11 | `src/memtable.rs` |
| WAL unit tests | 7 | `src/wal.rs` |
| SSTable unit tests | 6 | `src/sstable.rs` |
| KvStore integration tests | 15 | `tests/integration.rs` |
| CLI integration tests | 6 | `tests/cli_integration.rs` |
| Throughput benchmark | 1 | `benches/throughput.rs` |
| Doc-tests | 1 | `src/lib.rs` |
| **Total** | **47** | |
