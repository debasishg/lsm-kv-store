use std::process::Command;
use tempfile::tempdir;

/// Returns the path to the built binary.
fn binary() -> std::path::PathBuf {
    // `cargo test` places the test binary's deps next to the main binary.
    env!("CARGO_BIN_EXE_lsm-kv-store").into()
}

fn run_cli(db_path: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .arg("--db-path")
        .arg(db_path)
        .args(args)
        .output()
        .expect("failed to execute CLI binary")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

#[test]
fn test_cli_put_get() {
    let dir = tempdir().unwrap();
    let out = run_cli(dir.path(), &["put", "greeting", "hello"]);
    assert!(out.status.success(), "put failed: {}", stderr(&out));
    assert_eq!(stdout(&out), "OK");

    let out = run_cli(dir.path(), &["get", "greeting"]);
    assert!(out.status.success(), "get failed: {}", stderr(&out));
    assert_eq!(stdout(&out), "hello");
}

#[test]
fn test_cli_get_missing_key() {
    let dir = tempdir().unwrap();
    let out = run_cli(dir.path(), &["get", "nonexistent"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "Key not found");
}

#[test]
fn test_cli_delete_then_get() {
    let dir = tempdir().unwrap();
    run_cli(dir.path(), &["put", "k", "v"]);

    let out = run_cli(dir.path(), &["delete", "k"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "OK");

    let out = run_cli(dir.path(), &["get", "k"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "Key not found");
}

#[test]
fn test_cli_list() {
    let dir = tempdir().unwrap();
    run_cli(dir.path(), &["put", "b_key", "b_val"]);
    run_cli(dir.path(), &["put", "a_key", "a_val"]);

    let out = run_cli(dir.path(), &["list"]);
    assert!(out.status.success());
    let output = stdout(&out);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 2);
    // Keys should be sorted.
    assert_eq!(lines[0], "a_key\ta_val");
    assert_eq!(lines[1], "b_key\tb_val");
}

#[test]
fn test_cli_list_empty() {
    let dir = tempdir().unwrap();
    let out = run_cli(dir.path(), &["list"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "(empty)");
}

#[test]
fn test_cli_overwrite() {
    let dir = tempdir().unwrap();
    run_cli(dir.path(), &["put", "k", "old"]);
    run_cli(dir.path(), &["put", "k", "new"]);

    let out = run_cli(dir.path(), &["get", "k"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "new");
}
