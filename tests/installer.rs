//! Exercise the POSIX installer offline with mocked downloads and platform data.
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
};
use tempfile::TempDir;

const ASSET: &str = "pi-sandbox-linux-x86_64";
const INSTALLER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh");

struct Installer {
    _temp: TempDir,
    root: PathBuf,
    home: PathBuf,
    release: PathBuf,
}
impl Installer {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_owned();
        let home = root.join("home with spaces");
        let release = root.join("release");
        for path in [&home, &release, &root.join("bin"), &root.join("tmp")] {
            fs::create_dir_all(path).unwrap();
        }
        let payload = b"#!/bin/sh\n[ \"$1\" = --check ]\nprintf 'mock dependency check\\n'\n";
        fs::write(release.join(ASSET), payload).unwrap();
        fs::write(
            release.join(format!("{ASSET}.sha256")),
            format!("{:x}  {ASSET}\n", Sha256::digest(payload)),
        )
        .unwrap();
        for (name, script) in [
            (
                "uname",
                r#"#!/bin/sh
case "$1" in
-s) printf '%s\n' "${MOCK_OS:-Linux}" ;;
-m) printf '%s\n' "${MOCK_ARCH:-x86_64}" ;;
esac
"#,
            ),
            (
                "curl",
                r#"#!/bin/sh
set -eu
out=''
url=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output) out=$2; shift 2 ;;
        *) url=$1; shift ;;
    esac
done
printf '%s\n' "$url" >> "$MOCK_LOG"
[ "${MOCK_DOWNLOAD_FAIL:-0}" = 0 ] || exit 22
cp "$MOCK_RELEASE/${url##*/}" "$out"
"#,
            ),
        ] {
            let path = root.join("bin").join(name);
            fs::write(&path, script).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            _temp: temp,
            root,
            home,
            release,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .arg(INSTALLER)
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("HOME", &self.home)
            .env("TMPDIR", self.root.join("tmp"))
            .env("MOCK_RELEASE", &self.release)
            .env("MOCK_LOG", self.root.join("requests"));
        command
    }
    fn binary(&self) -> PathBuf {
        self.home.join(".local/bin/pi-sandbox")
    }
    fn assert_clean(&self) {
        assert_eq!(fs::read_dir(self.root.join("tmp")).unwrap().count(), 0);
        let bin = self.home.join(".local/bin");
        if bin.exists() {
            assert!(fs::read_dir(bin).unwrap().all(|p| {
                !p.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".pi-sandbox.")
            }));
        }
    }
}
fn ok(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn installer_downloads_verifies_and_installs_latest() {
    let test = Installer::new();
    ok(test.command().output().unwrap());
    assert_eq!(
        fs::read(test.binary()).unwrap(),
        fs::read(test.release.join(ASSET)).unwrap()
    );
    assert_eq!(
        fs::metadata(test.binary()).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let requests = fs::read_to_string(test.root.join("requests")).unwrap();
    assert!(requests.contains("https://github.com/ewiggin/pi-sandbox/releases/latest/download/"));
    test.assert_clean();
}

#[test]
fn installer_accepts_specific_version_and_custom_directory() {
    let test = Installer::new();
    let destination = test.root.join("custom install");
    ok(test
        .command()
        .env("PI_SANDBOX_VERSION", "v0.1.0")
        .env("PI_SANDBOX_INSTALL_DIR", &destination)
        .output()
        .unwrap());
    assert!(destination.join("pi-sandbox").is_file());
    assert!(
        fs::read_to_string(test.root.join("requests"))
            .unwrap()
            .contains("releases/download/v0.1.0/")
    );
    test.assert_clean();
}

#[test]
fn corrupted_download_preserves_previous_installation() {
    let test = Installer::new();
    fs::create_dir_all(test.binary().parent().unwrap()).unwrap();
    fs::write(test.binary(), "previous installation").unwrap();
    fs::write(test.release.join(ASSET), "corrupted payload").unwrap();
    let output = test.command().output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(test.binary()).unwrap(),
        "previous installation"
    );
    test.assert_clean();
}

#[test]
fn checksum_cannot_verify_a_different_file() {
    let test = Installer::new();
    let checksum = format!("{:x}  another-file\n", Sha256::digest(b"payload"));
    fs::write(test.release.join(format!("{ASSET}.sha256")), checksum).unwrap();
    assert!(!test.command().output().unwrap().status.success());
    assert!(!test.binary().exists());
    test.assert_clean();
}

#[test]
fn failed_download_does_not_install_or_leave_temporary_files() {
    let test = Installer::new();
    assert!(
        !test
            .command()
            .env("MOCK_DOWNLOAD_FAIL", "1")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!test.binary().exists());
    test.assert_clean();
}

#[test]
fn unsupported_platforms_fail_before_downloading() {
    for (name, value) in [("MOCK_OS", "Darwin"), ("MOCK_ARCH", "aarch64")] {
        let test = Installer::new();
        assert!(
            !test
                .command()
                .env(name, value)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(!test.root.join("requests").exists());
        assert!(!test.binary().exists());
    }
}

#[test]
fn installer_can_be_piped_to_sh() {
    let test = Installer::new();
    let command = test.command();
    // Same execution model as curl ... | sh: the script is supplied on stdin.
    let script = fs::File::open(INSTALLER).unwrap();
    let mut piped = Command::new("/bin/sh");
    piped.env_clear();
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            piped.env(key, value);
        }
    }
    piped
        .env_remove("PI_SANDBOX_VERSION")
        .env_remove("PI_SANDBOX_INSTALL_DIR")
        .stdin(script);
    ok(piped.output().unwrap());
    assert!(test.binary().is_file());
    test.assert_clean();
}
