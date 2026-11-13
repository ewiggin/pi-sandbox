//! Dependency preflight must be useful even when no project can be opened.
use std::{
    env, fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::Command,
};

fn binary() -> PathBuf {
    env::var_os("PI_SANDBOX_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_pi-sandbox")))
}

#[test]
fn check_succeeds_without_opening_project_or_creating_state() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let output = Command::new(binary())
        .arg("--check")
        .arg("--project")
        .arg(temp.path().join("missing-project"))
        .arg("--state-root")
        .arg(&state)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("OK Bubblewrap:"));
    assert!(stdout.contains("OK Pi (ELF autónomo):"));
    assert!(!state.exists());
}

#[test]
fn automatic_preflight_reports_both_missing_dependencies_and_stops() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let marker = temp.path().join("should-not-run");
    // Empty PATH, invalid project: dependency errors must come first.
    let output = Command::new(binary())
        .env("PATH", temp.path())
        .arg("--project")
        .arg(temp.path().join("missing-project"))
        .arg("--state-root")
        .arg(&state)
        .args(["--exec", "/usr/bin/touch"])
        .arg(&marker)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("Bubblewrap:"), "{error}");
    assert!(error.contains("Pi:"), "{error}");
    assert!(error.contains("pacman") && error.contains("https://pi.dev"));
    assert!(!state.exists());
    assert!(!marker.exists());
}

#[test]
fn missing_pi_is_reported_when_bubblewrap_exists() {
    let temp = tempfile::tempdir().unwrap();
    // Only test existence here: --check must not execute host programs.
    symlink("/usr/bin/true", temp.path().join("bwrap")).unwrap();
    let output = Command::new(binary())
        .env("PATH", temp.path())
        .arg("--check")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("Pi:"));
    assert!(!error.contains("Bubblewrap:"));
}

#[test]
fn explicit_pi_dir_works_without_pi_in_path_and_detects_missing_bubblewrap() {
    let temp = tempfile::tempdir().unwrap();
    let pi_dir = temp.path().join("pi-distribution");
    fs::create_dir(&pi_dir).unwrap();
    // A known ELF is sufficient for file-format validation; never execute it.
    fs::copy("/usr/bin/true", pi_dir.join("pi")).unwrap();
    let output = Command::new(binary())
        .env("PATH", temp.path())
        .arg("--check")
        .arg("--pi-dir")
        .arg(&pi_dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("Bubblewrap:"));
    assert!(!error.contains("Pi:"));
    symlink("/usr/bin/true", temp.path().join("bwrap")).unwrap();
    let output = Command::new(binary())
        .env("PATH", temp.path())
        .arg("--check")
        .arg("--pi-dir")
        .arg(pi_dir)
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn check_rejects_pi_wrappers_and_non_executable_files() {
    let temp = tempfile::tempdir().unwrap();
    let pi = temp.path().join("pi");
    fs::write(&pi, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&pi, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(binary())
        .arg("--check")
        .arg("--pi-dir")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wrapper npm"));
    fs::set_permissions(pi, fs::Permissions::from_mode(0o644)).unwrap();
    let output = Command::new(binary())
        .arg("--check")
        .arg("--pi-dir")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no existe o no es ejecutable"));
}
