use crate::{
    Result,
    cli::Options,
    dependencies,
    diagnostics::{PathContext, path_error},
    ignores, state,
};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Component, Path, PathBuf},
    process::Command,
};

fn expand_user(path: &Path, home: &Path) -> PathBuf {
    if path == Path::new("~") {
        home.to_path_buf()
    } else if let Ok(rest) = path.strip_prefix("~") {
        home.join(rest)
    } else {
        path.to_path_buf()
    }
}

// Like Path.resolve(strict=False): resolve existing ancestors and symlinks,
// including a state-root whose final directories do not exist yet.
fn resolve(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .at("resolve current directory", Path::new("."))?
            .join(path)
    };
    let mut result = PathBuf::from("/");
    let mut pending: std::collections::VecDeque<OsString> = absolute
        .components()
        .map(|c| c.as_os_str().to_owned())
        .collect();
    let mut links = 0;
    while let Some(part) = pending.pop_front() {
        if part == "/" || part == "." {
            continue;
        }
        if part == ".." {
            result.pop();
            continue;
        }
        result.push(&part);
        match fs::symlink_metadata(&result) {
            Ok(m) if m.is_symlink() => {
                links += 1;
                if links > 40 {
                    return Err("too many symbolic links in path".into());
                }
                let target = fs::read_link(&result).at("read state path symbolic link", &result)?;
                result.pop();
                if target.is_absolute() {
                    result = PathBuf::from("/");
                }
                for component in target.components().rev() {
                    if component != Component::RootDir {
                        pending.push_front(component.as_os_str().to_owned());
                    }
                }
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(path_error("inspect state path", &result, e)),
        }
    }
    Ok(result)
}

fn bind(command: &mut Command, flag: &str, source: &Path, destination: &Path) {
    command.arg(flag).arg(source).arg(destination);
}

fn system_file(command: &mut Command, name: &str) -> Result<()> {
    let path = Path::new(name);
    match fs::canonicalize(path) {
        Ok(source) => bind(command, "--ro-bind", &source, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(path_error("resolve system file", path, e)),
    }
    Ok(())
}

fn valid_env(name: &OsStr) -> bool {
    let name = name.as_bytes();
    name.first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && name.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn close_extra_fds() -> Result<()> {
    // Unlike Command::exec alone, this also prevents FDs inherited from the
    // invoking process from leaking. CLOEXEC keeps Rust's own live FDs valid
    // until exec and marks every descriptor, including ones above RLIMIT_NOFILE.
    // SAFETY: close_range with CLOEXEC only changes flags on this process' FDs.
    if unsafe {
        libc::syscall(
            libc::SYS_close_range,
            3u32,
            u32::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        )
    } == 0
    {
        return Ok(());
    }
    // Older kernels may not implement close_range/CLOEXEC. Enumerate /proc;
    // inability to sanitize descriptors is an error, never a silent fallback.
    let fds = fs::read_dir("/proc/self/fd")
        .at(
            "list inherited file descriptors",
            Path::new("/proc/self/fd"),
        )?
        .map(|e| {
            let e = e?;
            e.file_name()
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
                .ok_or_else(|| std::io::Error::other("invalid file descriptor in /proc/self/fd"))
        })
        .collect::<std::io::Result<Vec<_>>>()
        .at(
            "read inherited file descriptor entries",
            Path::new("/proc/self/fd"),
        )?;
    for fd in fds.into_iter().filter(|fd| *fd > 2) {
        // SAFETY: fcntl operates on numeric FDs only; EBADF may be the already
        // closed read_dir FD and is harmless. Any other failure is fatal.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EBADF) {
                continue;
            }
            return Err(error.into());
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

pub fn run(options: Options) -> Result<()> {
    let dependencies = dependencies::check(options.pi_dir.as_deref())?;
    if options.check {
        println!("OK Bubblewrap: {}", dependencies.bwrap.display());
        println!(
            "OK Pi (standalone ELF): {}",
            dependencies.pi_dir.join("pi").display()
        );
        println!(
            "Dependencies found. Namespace availability is checked when starting the sandbox."
        );
        return Ok(());
    }
    let bwrap = dependencies.bwrap;
    let pi_dir = dependencies.pi_dir;
    let home = env::var_os("HOME").ok_or("HOME is not set")?;
    let home_path = PathBuf::from(home);
    let home = fs::canonicalize(&home_path).at("resolve HOME directory", &home_path)?;
    let project_path = match options.project {
        Some(path) => expand_user(&path, &home),
        None => env::current_dir().at("resolve current project directory", Path::new("."))?,
    };
    let project = fs::canonicalize(&project_path).at("resolve project directory", &project_path)?;
    if !project.is_dir() {
        return Err("--project must be a directory".into());
    }
    if home.starts_with(&project) {
        return Err("exposing your HOME or any of its parent directories is not allowed".into());
    }
    for base in ["/usr", "/etc", "/dev", "/proc", "/sys", "/run"] {
        if project.starts_with(base) {
            return Err(format!("the project cannot be under {base}").into());
        }
    }
    let state_root = resolve(&expand_user(
        &options
            .state_root
            .unwrap_or(home.join(".local/state/pi-sandbox")),
        &home,
    ))?;
    if state_root.starts_with(&project) || project.starts_with(&state_root) {
        return Err(
            "state and project directories must be separate; neither may contain the other".into(),
        );
    }
    if pi_dir.starts_with(&project) || project.starts_with(&pi_dir) {
        return Err("the Pi installation must be outside the project".into());
    }
    let mut forwarded = Vec::new();
    for name in options.environment {
        if !valid_env(&name) {
            return Err(format!("invalid environment variable name: {name:?}").into());
        }
        let value = env::var_os(&name)
            .ok_or_else(|| format!("environment variable {} is not set", name.to_string_lossy()))?;
        forwarded.push((name, value));
    }
    let patterns = ignores::load(&project, &options.ignores)?;
    let (masks, count) = ignores::mounts(&project, &patterns)?;
    let prepared = state::prepare(
        &state_root,
        &project,
        &patterns,
        options.auth_profile.as_deref(),
    )?;
    let mut command = Command::new(&bwrap);
    command.env_clear().env("PATH", "/usr/bin:/bin").args([
        "--unshare-all",
        "--unshare-user",
        "--disable-userns",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--hostname",
        "pi-sandbox",
        "--clearenv",
        "--ro-bind",
        "/usr",
        "/usr",
    ]);
    if options.network {
        command.arg("--share-net");
    }
    for name in ["/bin", "/sbin", "/lib", "/lib64"] {
        let path = Path::new(name);
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.is_symlink() => {
                command
                    .arg("--symlink")
                    .arg(fs::read_link(path).at("read system symbolic link", path)?)
                    .arg(path);
            }
            Ok(_) => bind(&mut command, "--ro-bind", path, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(path_error("inspect system path", path, e)),
        }
    }
    command.args([
        "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--tmpfs", "/run", "--dir", "/etc",
        "--dir", "/home",
    ]);
    bind(
        &mut command,
        "--bind",
        &prepared.home,
        Path::new("/home/sandbox"),
    );
    if let Some(directory) = &prepared.auth_directory {
        bind(
            &mut command,
            "--bind",
            directory,
            Path::new("/auth-profile"),
        );
    }
    bind(&mut command, "--ro-bind", &pi_dir, Path::new("/opt/pi"));
    bind(&mut command, "--bind", &project, Path::new("/workspace"));
    command.args(["--chdir", "/workspace"]).args(masks);
    for name in [
        "/etc/ssl/certs",
        "/etc/ca-certificates/extracted",
        "/etc/ld.so.cache",
        "/etc/localtime",
    ] {
        system_file(&mut command, name)?;
    }
    if options.network {
        for name in ["/etc/resolv.conf", "/etc/hosts", "/etc/nsswitch.conf"] {
            system_file(&mut command, name)?;
        }
    }
    for (name, value) in [
        ("HOME", "/home/sandbox"),
        ("USER", "sandbox"),
        ("LOGNAME", "sandbox"),
        ("PATH", "/opt/pi:/usr/local/bin:/usr/bin:/bin"),
        ("SHELL", "/bin/bash"),
        ("LANG", "C.UTF-8"),
        ("TMPDIR", "/tmp"),
        ("XDG_CONFIG_HOME", "/home/sandbox/.config"),
        ("XDG_CACHE_HOME", "/home/sandbox/.cache"),
        ("XDG_DATA_HOME", "/home/sandbox/.local/share"),
        ("XDG_STATE_HOME", "/home/sandbox/.local/state"),
        ("PI_CODING_AGENT_DIR", "/home/sandbox/.pi/agent"),
        ("PI_PACKAGE_DIR", "/opt/pi"),
        ("PI_SKIP_VERSION_CHECK", "1"),
        ("PI_TELEMETRY", "0"),
    ] {
        command.args(["--setenv", name, value]);
    }
    command
        .args(["--setenv", "TERM"])
        .arg(env::var_os("TERM").unwrap_or_else(|| "xterm-256color".into()));
    if !options.network {
        command.args(["--setenv", "PI_OFFLINE", "1"]);
    }
    for (name, value) in forwarded {
        command.arg("--setenv").arg(name).arg(value);
    }
    command.args(["--remount-ro", "/", "--"]);
    match options.command {
        Some(payload) => {
            command.args(payload);
        }
        None => {
            command.arg("/opt/pi/pi").args(options.pi_args);
        }
    }
    eprintln!(
        "pi-sandbox: {} -> /workspace | startup exclusions: {count} | network: {}",
        project.display(),
        if options.network {
            "HOST (unfiltered)"
        } else {
            "disabled"
        }
    );
    if let Some(name) = &options.auth_profile {
        eprintln!(
            "pi-sandbox: shared auth profile {name:?}; credentials can be read and modified by every project using this profile"
        );
    }
    close_extra_fds()?;
    // Replace the launcher: no supervising runtime, signal forwarding or leaked
    // child pipes needed. There is deliberately no unsandboxed fallback.
    Err(path_error("execute Bubblewrap", &bwrap, command.exec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_missing_state_root_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(temp.path(), temp.path().join("alias")).unwrap();
        assert_eq!(
            resolve(&temp.path().join("alias/missing/../state")).unwrap(),
            temp.path().join("state")
        );
    }
    #[test]
    fn environment_names() {
        assert!(valid_env(OsStr::new("OPENAI_API_KEY")));
        for name in ["", "1KEY", "A=B", "KEY-X"] {
            assert!(!valid_env(OsStr::new(name)));
        }
    }
}
