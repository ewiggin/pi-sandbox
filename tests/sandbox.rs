//! Real Bubblewrap integration tests. No Python, credentials or provider calls.
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    net::TcpListener,
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::symlink, process::CommandExt},
    },
    path::PathBuf,
    process::{Command, Output},
};
use tempfile::TempDir;

fn binary() -> PathBuf {
    env::var_os("PI_SANDBOX_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_pi-sandbox")))
}

struct Sandbox {
    _temp: TempDir,
    root: PathBuf,
    project: PathBuf,
    state: PathBuf,
}
impl Sandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let project = root.join("project with spaces");
        fs::create_dir(&project).unwrap();
        Self {
            _temp: temp,
            state: root.join("state"),
            root,
            project,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(binary());
        command
            .arg("--project")
            .arg(&self.project)
            .arg("--state-root")
            .arg(&self.state);
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn shell(&self, options: &[&str], script: &str) -> Output {
        self.command()
            .args(options)
            .args(["--exec", "/bin/sh", "-eu", "-c", script])
            .output()
            .unwrap()
    }
    fn write(&self, name: &str, text: &str) {
        let path = self.project.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
fn ok(output: Output) {
    assert!(
        output.status.success(),
        "status={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn filesystem_environment_processes_and_inherited_fds() {
    let sandbox = Sandbox::new();
    let secret = sandbox.root.join("host-secret");
    fs::write(&secret, "host secret").unwrap();
    symlink(&secret, sandbox.project.join("escape")).unwrap();
    let file = fs::File::open(&secret).unwrap();
    let fd = file.as_raw_fd();
    let host_pid = std::process::id().to_string();
    let host_home = env::var_os("HOME").unwrap();
    let mut command = sandbox.command();
    command.env("SANDBOX_TEST_SECRET", "not forwarded")
        .env("SSH_AUTH_SOCK", "/fake/agent").env("PI_SESSION_ID", "host-session")
        .args(["--exec", "/bin/sh", "-eu", "-c", r#"
        test "$PWD" = /workspace
        test "$HOME" = /home/sandbox
        test -z "${SANDBOX_TEST_SECRET+x}"
        test -z "${SSH_AUTH_SOCK+x}"
        test -z "${PI_SESSION_ID+x}"
        test -z "${DBUS_SESSION_BUS_ADDRESS+x}"
        test -z "${DISPLAY+x}"
        test -z "${WAYLAND_DISPLAY+x}"
        test ! -e "$1"
        test ! -e "$2/.ssh"
        test ! -e "/proc/$3"
        test ! -e /proc/self/fd/198
        test ! -e /workspace/escape
        test ! -e /run/user
        test ! -e /var/run/docker.sock
        test ! -e /sys
        printf ok > /workspace/output
        printf saved > ~/persistent
        printf temporary > /tmp/ephemeral
        for path in /usr/should-not-write /opt/pi/should-not-write /etc/should-not-write /should-not-write; do
            if (printf bad > "$path") 2>/dev/null; then exit 1; fi
        done
        "#, "probe"]).arg(&secret).arg(host_home).arg(host_pid);
    // Give the launcher a deliberately inheritable FD, as an unsafe caller might.
    // SAFETY: only async-signal-safe dup2 runs between fork and exec; fd is open.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(fd, 198) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    ok(command.output().unwrap());
    assert_eq!(
        fs::read_to_string(sandbox.project.join("output")).unwrap(),
        "ok"
    );
    ok(sandbox.shell(
        &[],
        "test \"$(cat ~/persistent)\" = saved; test ! -e /tmp/ephemeral",
    ));
}

#[test]
fn network_off_and_explicit_host_network() {
    let sandbox = Sandbox::new();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let script = format!(": > /dev/tcp/127.0.0.1/{port}");
    assert!(
        !sandbox
            .run(&["--exec", "/bin/bash", "-c", &script])
            .status
            .success()
    );
    ok(sandbox.run(&["--network", "--exec", "/bin/bash", "-c", &script]));
}

#[test]
fn state_is_separate_per_project_and_persistent() {
    let mut sandbox = Sandbox::new();
    ok(sandbox.shell(&[], "echo private > ~/marker"));
    ok(sandbox.shell(&[], "test \"$(cat ~/marker)\" = private"));
    sandbox.project = sandbox.root.join("other-project");
    fs::create_dir(&sandbox.project).unwrap();
    ok(sandbox.shell(&[], "test ! -e ~/marker"));
}

#[test]
fn nested_user_namespaces_are_disabled() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[
        "--exec",
        "/usr/bin/unshare",
        "--user",
        "--map-root-user",
        "true",
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("Operation not permitted") || error.contains("No space left on device"),
        "{error}"
    );
}

#[test]
fn explicit_environment_and_exit_code() {
    let sandbox = Sandbox::new();
    ok(sandbox
        .command()
        .env("SANDBOX_EXPLICIT", "yes")
        .args([
            "--env",
            "SANDBOX_EXPLICIT",
            "--exec",
            "/bin/sh",
            "-c",
            "test \"$SANDBOX_EXPLICIT\" = yes",
        ])
        .output()
        .unwrap());
    assert_eq!(sandbox.shell(&[], "exit 42").status.code(), Some(42));
    assert!(
        !sandbox
            .run(&["--env", "BAD-NAME", "--exec", "true"])
            .status
            .success()
    );
}

#[test]
fn reject_home_and_overlapping_state() {
    let sandbox = Sandbox::new();
    assert!(
        !Command::new(binary())
            .arg("--project")
            .arg(env::var_os("HOME").unwrap())
            .output()
            .unwrap()
            .status
            .success()
    );
    let bad_state = sandbox.project.join("bad-state");
    assert!(
        !sandbox
            .command()
            .arg("--state-root")
            .arg(&bad_state)
            .args(["--exec", "true"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!bad_state.exists());
}

#[test]
fn ignore_file_and_cli_masks_preserve_direct_writes() {
    let sandbox = Sandbox::new();
    sandbox.write(".pi-sandboxignore", "# private\n.env\nsecrets/\n*.pem\n");
    for name in [
        ".env",
        "cli-only",
        "secrets/token",
        "src/key.pem",
        "src/allowed",
    ] {
        sandbox.write(name, "SECRET");
    }
    sandbox.write(".gitignore", "must-stay\n");
    sandbox.write("must-stay", "visible");
    fs::hard_link(
        sandbox.project.join("secrets/token"),
        sandbox.project.join("hardlink-alias"),
    )
    .unwrap();
    symlink("secrets", sandbox.project.join("alias")).unwrap();
    ok(sandbox.shell(
        &["--ignore", "cli-only"],
        r#"
        for name in .env cli-only src/key.pem hardlink-alias secrets/token alias/token; do
            if data=$(cat "$name" 2>/dev/null); then test -z "$data"; fi
            if (printf overwrite > "$name") 2>/dev/null; then exit 1; fi
        done
        test "$(cat must-stay)" = visible
        printf after > src/allowed
        printf persisted > new-file
        printf persisted > src/new-file
        printf atomic > src/replacement
        mv src/replacement src/allowed
    "#,
    ));
    for name in [".env", "cli-only", "secrets/token", "src/key.pem"] {
        assert_eq!(
            fs::read_to_string(sandbox.project.join(name)).unwrap(),
            "SECRET"
        );
    }
    assert_eq!(
        fs::read_to_string(sandbox.project.join("src/allowed")).unwrap(),
        "atomic"
    );
    for name in ["new-file", "src/new-file"] {
        assert_eq!(
            fs::read_to_string(sandbox.project.join(name)).unwrap(),
            "persisted"
        );
    }
}

#[test]
fn current_directory_default_and_accumulated_ignores() {
    let sandbox = Sandbox::new();
    sandbox.write(".pi-sandboxignore", "from-file\n");
    for name in ["from-file", "from-cli-1", "from-cli-2", "allowed"] {
        sandbox.write(name, "original");
    }
    ok(Command::new(binary())
        .current_dir(&sandbox.project)
        .arg("--state-root")
        .arg(&sandbox.state)
        .args([
            "--ignore",
            "from-cli-1",
            "--ignore",
            "from-cli-2",
            "--exec",
            "/bin/sh",
            "-eu",
            "-c",
            r#"
            for name in from-file from-cli-1 from-cli-2; do
                if data=$(cat "$name" 2>/dev/null); then test -z "$data"; fi
            done
            test "$(cat allowed)" = original
        "#,
        ])
        .output()
        .unwrap());
}

#[test]
fn renaming_parent_does_not_unmask_nested_secret() {
    let sandbox = Sandbox::new();
    sandbox.write("nested/secret", "HIDDEN");
    ok(sandbox.shell(
        &["--ignore", "nested/secret"],
        r#"
        if mv nested moved 2>/dev/null; then
            secret=moved/secret
            mkdir nested
            test ! -e nested/secret
        else
            secret=nested/secret
        fi
        if data=$(cat "$secret" 2>/dev/null); then test -z "$data"; fi
    "#,
    ));
}

#[test]
fn globs_are_relative_and_recursive_only_with_double_star() {
    let sandbox = Sandbox::new();
    for name in [
        "src/root.pem",
        "src/deep/key.pem",
        "other/key.pem",
        "other/secrets/token",
    ] {
        sandbox.write(name, "content");
    }
    ok(sandbox.shell(
        &["--ignore", "src/*.pem", "--ignore", "**/secrets/"],
        r#"
        if data=$(cat src/root.pem 2>/dev/null); then test -z "$data"; fi
        test "$(cat src/deep/key.pem)" = content
        test "$(cat other/key.pem)" = content
        if cat other/secrets/token 2>/dev/null; then exit 1; fi
    "#,
    ));
    ok(sandbox.shell(
        &["--ignore", "src/**/*.pem"],
        "test ! -s src/root.pem; test ! -s src/deep/key.pem",
    ));
}

#[test]
fn different_ignore_policy_has_separate_state() {
    let sandbox = Sandbox::new();
    ok(sandbox.shell(&[], "echo old-secret > ~/marker"));
    ok(sandbox.shell(
        &["--ignore", ".env"],
        "test ! -e ~/marker; echo limited > ~/marker",
    ));
    ok(sandbox.shell(&["--ignore", ".env"], "test \"$(cat ~/marker)\" = limited"));
    ok(sandbox.shell(&[], "test \"$(cat ~/marker)\" = old-secret"));
}

#[test]
fn reject_unsafe_ignores_and_symlink_config() {
    let sandbox = Sandbox::new();
    for pattern in ["/etc", "../secret", ".", "!secret", "a/../b", ""] {
        assert!(
            !sandbox
                .run(&["--ignore", pattern, "--exec", "true"])
                .status
                .success(),
            "{pattern}"
        );
    }
    symlink("does-not-exist", sandbox.project.join(".pi-sandboxignore")).unwrap();
    let output = sandbox.run(&["--exec", "true"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("symbolic link"));
}

#[test]
fn recursive_name_exclusions_allow_unrelated_symlinks_and_still_mask_targets() {
    let sandbox = Sandbox::new();
    sandbox.write("authentik/packages/client-ts/allowed", "visible");
    sandbox.write("authentik/packages/client-ts/key.pem", "private");
    sandbox.write("authentik/postgres-data/token", "private");
    fs::create_dir_all(sandbox.project.join("authentik/web/packages")).unwrap();
    symlink(
        "../../packages/client-ts",
        sandbox.project.join("authentik/web/packages/client-ts"),
    )
    .unwrap();
    symlink("../postgres-data", sandbox.project.join("authentik/web/db")).unwrap();
    ok(sandbox.shell(&["--ignore", "**/postgres-data", "--ignore", "**/**/*.pem"], r#"
        test "$(cat authentik/web/packages/client-ts/allowed)" = visible
        for path in authentik/postgres-data/token authentik/web/db/token; do
            if cat "$path" 2>/dev/null; then exit 1; fi
        done
        for path in authentik/packages/client-ts/key.pem authentik/web/packages/client-ts/key.pem; do
            if data=$(cat "$path" 2>/dev/null); then test -z "$data"; fi
        done
    "#));
}

#[test]
fn reject_rules_on_or_through_symlinks() {
    let sandbox = Sandbox::new();
    sandbox.write("actual/secret", "private");
    symlink("actual", sandbox.project.join("link")).unwrap();
    for pattern in ["link", "link/", "link/secret", "**/link/**"] {
        let output = sandbox.run(&["--ignore", pattern, "--exec", "true"]);
        assert!(!output.status.success(), "{pattern}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("symlink"));
    }
}

#[test]
fn real_pi_starts_without_host_auth() {
    let sandbox = Sandbox::new();
    let version = sandbox.run(&["--", "--version"]);
    assert!(!version.stdout.is_empty());
    ok(version);
    let help = sandbox.run(&["--", "--help"]);
    assert!(String::from_utf8_lossy(&help.stdout).contains("--model"));
    ok(help);
}

#[test]
fn reuse_state_from_legacy_python_launcher() {
    let sandbox = Sandbox::new();
    for (rules, encoding) in [
        (vec![], ""),
        (vec!["--ignore", ".env"], "[('.env', False)]"),
    ] {
        let mut hash = Sha256::new();
        hash.update(sandbox.project.as_os_str().as_bytes());
        if !encoding.is_empty() {
            hash.update(b"\0ignores-v1\0");
            hash.update(encoding);
        }
        let old_home = sandbox
            .state
            .join(format!("{:x}", hash.finalize()))
            .join("home");
        fs::create_dir_all(&old_home).unwrap();
        fs::write(old_home.join("legacy-login-marker"), "preserved").unwrap();
        ok(sandbox.shell(&rules, "test \"$(cat ~/legacy-login-marker)\" = preserved"));
    }
}

#[test]
fn state_symlinks_are_rejected() {
    let sandbox = Sandbox::new();
    let key = format!(
        "{:x}",
        Sha256::digest(sandbox.project.as_os_str().as_bytes())
    );
    fs::create_dir_all(&sandbox.state).unwrap();
    symlink(&sandbox.root, sandbox.state.join(key)).unwrap();
    assert!(!sandbox.run(&["--exec", "true"]).status.success());
}

#[test]
fn non_utf8_paths_are_supported_but_undecodable_ignored_names_fail_closed() {
    use std::os::unix::ffi::OsStringExt;
    let mut sandbox = Sandbox::new();
    sandbox.project = sandbox
        .root
        .join(std::ffi::OsString::from_vec(b"project-\xff".to_vec()));
    fs::create_dir(&sandbox.project).unwrap();
    ok(sandbox.shell(&[], "printf ok > output"));
    assert_eq!(fs::read(sandbox.project.join("output")).unwrap(), b"ok");
    fs::write(
        sandbox
            .project
            .join(std::ffi::OsString::from_vec(b"secret-\xff".to_vec())),
        "secret",
    )
    .unwrap();
    assert!(
        !sandbox
            .run(&["--ignore", "secret-*", "--exec", "true"])
            .status
            .success()
    );
}

#[test]
fn no_unsandboxed_fallback_when_bwrap_fails() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let bin = sandbox.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let fake_bwrap = bin.join("bwrap");
    fs::write(&fake_bwrap, "#!/bin/sh\nexit 73\n").unwrap();
    fs::set_permissions(fake_bwrap, fs::Permissions::from_mode(0o755)).unwrap();
    let pi = Command::new("/bin/sh")
        .args(["-c", "command -v pi"])
        .output()
        .unwrap();
    let pi = fs::canonicalize(String::from_utf8(pi.stdout).unwrap().trim()).unwrap();
    let output = sandbox
        .command()
        .env("PATH", &bin)
        .arg("--pi-dir")
        .arg(pi.parent().unwrap())
        .args(["--exec", "/bin/sh", "-c", "touch /workspace/should-not-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(73));
    assert!(!sandbox.project.join("should-not-run").exists());
}
