# Copilot Instructions — lsm-kv-store

## Project Overview

This is **lsm-kv-store**, a minimal, embeddable, persistent key-value store built on a Log-Structured Merge Tree (LSM tree) in **pure Rust**. It is an educational/learning project inspired by LevelDB/RocksDB basics.

## Architecture

- **MemTable** — in-memory `BTreeMap<String, Option<String>>` (tombstones for deletes)
- **Write-Ahead Log (WAL)** — append-only file for crash recovery
- **SSTables** — sorted, immutable on-disk files flushed from the MemTable
- **Compaction** — size-tiered merge of SSTables across levels
- **CLI** — `clap`-based subcommands: `put`, `get`, `delete`, `list`

## Coding Conventions

- **Language/edition**: Rust 2024 edition, stable toolchain.
- **Error handling**: Use `Result` everywhere with a custom `Error` enum. Never `unwrap()` / `expect()` in library code; only in tests or CLI `main()`.
- **Style**: `cargo fmt` applied, `cargo clippy` clean (no warnings).
- **Naming**: snake_case for functions/variables, PascalCase for types/traits, SCREAMING_SNAKE for constants.
- **Visibility**: Prefer the most restrictive visibility (`pub(crate)`, private) unless the item is part of the public API.
- **Modules**: Core logic in `src/lib.rs` (or submodules under `src/`), CLI entry point in `src/main.rs`.

## Dependencies

Keep dependencies minimal — only:
- `clap` — CLI argument parsing
- `serde` / `bincode` — serialization
- `tempfile`, `rand` — test utilities only (dev-dependencies)

Do **not** add external DB crates; the whole point is building from scratch.

## Testing

- Unit tests live in the same file (`#[cfg(test)] mod tests { ... }`).
- Integration tests go in `tests/`.
- Target ≥ 90% coverage on core logic.
- Always test: persistence (write → restart → read), tombstone deletes, key overwrites, WAL recovery.

## Performance

- Write-heavy workload optimisation: append-only writes, batch WAL flushes.
- Target ≥ 5k writes/sec on consumer SSD.
- Keep data structures simple; no premature optimisation.

## Things to Avoid

- Multi-threaded concurrent access (out of scope for v1).
- Range scans / iterators (point `get` only).
- Compression, bloom filters, snapshots / MVCC (all stretch goals).
- `unsafe` code unless absolutely necessary and well-justified.

## When Generating Code

1. Ensure all new public items have idiomatic doc comments (`///`).
2. Prefer returning `Result<T, Error>` over panicking.
3. Use `#[derive(Debug, Clone, ...)]` where appropriate.
4. Write at least one unit test for every new function.
5. Run `cargo clippy` and `cargo fmt` mentally — generated code should pass both.
