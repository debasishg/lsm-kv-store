use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use lsm_kv_store::engine::{KvStore, KvStoreConfig};

/// A minimal persistent key-value store built on a Log-Structured Merge Tree.
#[derive(Parser)]
#[command(name = "lsm-kv", version, about)]
struct Cli {
    /// Path to the database directory.
    #[arg(short, long, default_value = "lsm_data")]
    db_path: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Insert or update a key-value pair.
    Put {
        /// The key to insert.
        key: String,
        /// The value to associate with the key.
        value: String,
    },
    /// Retrieve the value for a key.
    Get {
        /// The key to look up.
        key: String,
    },
    /// Delete a key.
    Delete {
        /// The key to delete.
        key: String,
    },
    /// List all live key-value pairs.
    List,
}

fn run() -> lsm_kv_store::Result<()> {
    let cli = Cli::parse();
    let config = KvStoreConfig::new(&cli.db_path);
    let mut store = KvStore::open(config)?;

    match cli.command {
        Command::Put { key, value } => {
            store.put(key.as_bytes(), value.as_bytes())?;
            println!("OK");
        }
        Command::Get { key } => match store.get(key.as_bytes())? {
            Some(value) => {
                let s = String::from_utf8_lossy(&value);
                println!("{s}");
            }
            None => {
                println!("Key not found");
            }
        },
        Command::Delete { key } => {
            store.delete(key.as_bytes())?;
            println!("OK");
        }
        Command::List => {
            let entries = store.list()?;
            if entries.is_empty() {
                println!("(empty)");
            } else {
                for (k, v) in &entries {
                    let ks = String::from_utf8_lossy(k);
                    let vs = String::from_utf8_lossy(v);
                    println!("{ks}\t{vs}");
                }
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
