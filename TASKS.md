# LSM Tree Implementation Tasks

High priority first. One task per loop iteration ideally.

- [ ] Initialize Cargo project: add dependencies (clap, serde, bincode, tempfile, rand)
- [ ] Create basic project structure: src/lib.rs for core logic, src/bin/cli.rs or src/main.rs
- [ ] Define public API in lib.rs: struct LsmKvStore { ... }, impl with put/get/delete/new
- [ ] Implement in-memory MemTable using BTreeMap<String, Option<String>> (tombstones for delete)
- [ ] Add put/get/delete to MemTable + unit tests (test_memtable.rs)
- [ ] Implement Write-Ahead Log (WAL): append-only file, recover MemTable from WAL on open
- [ ] Add WAL fsync option (configurable, default true for durability)
- [ ] Implement flush: when MemTable > threshold (e.g. 4MB), sort + write immutable SSTable to disk
- [ ] Define SSTable on-disk format (simple: sorted key-value pairs + index block)
- [ ] Implement read path: check MemTable first, then search recent → older SSTables
- [ ] Add basic compaction: merge two SSTables into one when level size exceeds limit
- [ ] Implement level 0 (L0) → level 1 merge (size-tiered style initially)
- [ ] Integrate CLI with clap: subcommands put/get/delete + optional --db-path
- [ ] Add recovery test: write 1000 pairs, kill process, restart and verify
- [ ] Add integration test suite: persistence, delete (tombstone), overwrite
- [ ] Implement simple benchmark binary (src/bin/bench.rs) for write/read throughput
- [ ] Handle errors properly (Result everywhere, custom Error enum)
- [ ] Run cargo clippy, fix warnings; cargo fmt
- [ ] Write README.md with usage, architecture overview, and benchmark results
- [ ] (Stretch) Add bloom filter per SSTable for faster negative lookups

When all boxes are checked → append "MISSION COMPLETE – MVP LSM KV Store" to progress.md and stop suggesting new tasks.