use std::time::Instant;

use lsm_kv_store::engine::{KvStore, KvStoreConfig};
use rand::Rng;
use tempfile::tempdir;

const NUM_ENTRIES: usize = 10_000;
const KEY_SIZE: usize = 16;
const VALUE_SIZE: usize = 100;

fn random_kv_pairs(n: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| {
            let key: Vec<u8> = (0..KEY_SIZE).map(|_| rng.r#gen::<u8>()).collect();
            let value: Vec<u8> = (0..VALUE_SIZE).map(|_| rng.r#gen::<u8>()).collect();
            (key, value)
        })
        .collect()
}

#[test]
fn bench_write_read_10k() {
    let dir = tempdir().unwrap();
    let pairs = random_kv_pairs(NUM_ENTRIES);

    // ── Write phase ──
    let write_start = Instant::now();
    {
        let mut config = KvStoreConfig::new(dir.path());
        config.sync_writes = false; // batch WAL flushes for throughput
        let mut store = KvStore::open(config).unwrap();
        for (k, v) in &pairs {
            store.put(k.clone(), v.clone()).unwrap();
        }
        store.flush().unwrap();
    }
    let write_elapsed = write_start.elapsed();
    let writes_per_sec = NUM_ENTRIES as f64 / write_elapsed.as_secs_f64();

    println!("\n=== Benchmark Results ===");
    println!(
        "Writes: {NUM_ENTRIES} entries in {:.2?} ({:.0} writes/sec)",
        write_elapsed, writes_per_sec
    );

    // ── Read phase (after reopen) ──
    let read_start = Instant::now();
    {
        let config = KvStoreConfig::new(dir.path());
        let store = KvStore::open(config).unwrap();
        for (k, v) in &pairs {
            let got = store.get(k).unwrap();
            assert_eq!(got.as_deref(), Some(v.as_slice()));
        }
    }
    let read_elapsed = read_start.elapsed();
    let reads_per_sec = NUM_ENTRIES as f64 / read_elapsed.as_secs_f64();

    println!(
        "Reads:  {NUM_ENTRIES} entries in {:.2?} ({:.0} reads/sec)",
        read_elapsed, reads_per_sec
    );
    println!("=========================\n");

    assert!(
        writes_per_sec >= 5000.0,
        "Write throughput {writes_per_sec:.0}/sec is below 5,000/sec target"
    );
}
