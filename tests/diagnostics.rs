//! Reproduce pre-launch EACCES without changing permissions on user data.
use std::{
    env, fs,
    os::unix::{
        fs::{PermissionsExt, symlink},
        process::CommandExt,
    },
    path::PathBuf,
    process::Command,
};

fn binary() -> PathBuf {
    env::var_os("PI_SANDBOX_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_pi-sandbox")))
}

#[test]
fn access_errors_report_the_exact_path_and_operation() {
    for scenario in ["ignore-file", "project-tree", "private-state"] {
        let temp = tempfile::tempdir().unwrap();
        // Make the fixture accessible when a root test runner drops privileges.
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let state = temp.path().join("state");
        let pi = temp.path().join("pi-distribution");
        for path in [&home, &project, &pi] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        // The failure must occur before executing Pi; only its ELF header matters.
        symlink("/usr/bin/true", pi.join("pi")).unwrap();
        symlink("/usr/bin/true", pi.join("bwrap")).unwrap();
        let (blocked, operation) = match scenario {
            "ignore-file" => {
                let file = project.join(".pi-sandboxignore");
                fs::write(&file, ".env\n").unwrap();
                (file, "read ignore file")
            }
            "project-tree" => {
                let directory = project.join("private");
                fs::create_dir(&directory).unwrap();
                fs::write(project.join(".pi-sandboxignore"), "private/\n").unwrap();
                (
                    directory,
                    "scan project directory for exclusions and hardlink aliases",
                )
            }
            _ => {
                fs::create_dir(&state).unwrap();
                (state.clone(), "inspect state path")
            }
        };
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
        let launcher = temp.path().join("pi-sandbox");
        fs::copy(binary(), &launcher).unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = Command::new(launcher);
        command
            .env("PATH", &pi)
            .env("HOME", &home)
            .arg("--project")
            .arg(&project)
            .arg("--pi-dir")
            .arg(&pi)
            .arg("--state-root")
            .arg(&state)
            .args(["--", "--version"]);
        // Root bypasses mode bits. Run only the test child as nobody in that case.
        // SAFETY: geteuid has no preconditions or side effects.
        if unsafe { libc::geteuid() } == 0 {
            command.gid(65534).uid(65534);
        }
        let output = command.output().unwrap();
        // Restore fixture access before assertions so cleanup works on failure too.
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{scenario}");
        assert!(
            message.contains("Permission denied"),
            "{scenario}: {message}"
        );
        assert!(
            message.contains(blocked.to_str().unwrap()),
            "{scenario}: {message}"
        );
        // An existing state root can be stat'ed but cannot be traversed; the
        // failure is inspecting its per-project child, not resolving the root.
        if scenario == "private-state" {
            assert!(
                message.contains("inspect private state directory"),
                "{message}"
            );
        } else {
            assert!(message.contains(operation), "{scenario}: {message}");
        }
    }
}
