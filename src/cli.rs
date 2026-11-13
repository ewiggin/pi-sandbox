use crate::Result;
use std::{env, ffi::OsString, path::PathBuf};

#[derive(Default, Debug)]
pub struct Options {
    pub project: Option<PathBuf>,
    pub ignores: Vec<String>,
    pub network: bool,
    pub check: bool,
    pub pi_dir: Option<PathBuf>,
    pub state_root: Option<PathBuf>,
    pub environment: Vec<OsString>,
    pub command: Option<Vec<OsString>>,
    pub pi_args: Vec<OsString>,
}

const HELP: &str = "Run Pi in Bubblewrap. Networking is disabled unless --network is set.

Usage: pi-sandbox [options] [-- Pi arguments...]

  --check            Check Pi and Bubblewrap without starting the sandbox
  --project PATH     Read-write project (default: current directory)
  --ignore PATTERN   Exclude a relative path/glob; repeatable, adds to .pi-sandboxignore
  --network          Share ALL host networking, including localhost
  --pi-dir PATH      Standalone Pi distribution (directory containing pi)
  --state-root PATH  Private per-project state, outside the project
  --env NAME         Explicitly forward a host environment variable; repeatable
  --exec COMMAND ... Run a command inside instead of Pi; must be the last option
  -h, --help         Show this help
  -V, --version      Show the launcher version

Examples:
  pi-sandbox --ignore .env --ignore secrets/ --network
  pi-sandbox --project /path/to/project --network -- --model openai-codex/gpt-5.4
  pi-sandbox --exec /bin/bash --noprofile --norc
";

pub fn parse() -> Result<Options> {
    parse_args(env::args_os().skip(1).collect())
}

fn parse_args(args: Vec<OsString>) -> Result<Options> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            options.pi_args.extend(args);
            break;
        }
        if arg == "--exec" {
            let command: Vec<_> = args.collect();
            if command.is_empty() {
                return Err("--exec requires a command".into());
            }
            options.command = Some(command);
            break;
        }
        if arg == "-h" || arg == "--help" {
            print!("{HELP}");
            std::process::exit(0);
        }
        if arg == "-V" || arg == "--version" {
            println!("pi-sandbox {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        if arg == "--check" {
            options.check = true;
            continue;
        }
        if arg == "--network" {
            options.network = true;
            continue;
        }
        // Keep filesystem paths, child argv and environment values byte-exact.
        // Only option names and ignore patterns require UTF-8.
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let bytes = arg.as_bytes();
        let equal = bytes.iter().position(|b| *b == b'=');
        let (name, inline) = match equal {
            Some(i) => (
                &bytes[..i],
                Some(OsString::from_vec(bytes[i + 1..].to_vec())),
            ),
            None => (bytes, None),
        };
        if !matches!(
            name,
            b"--project" | b"--ignore" | b"--pi-dir" | b"--state-root" | b"--env"
        ) {
            if !bytes.starts_with(b"-") {
                options.pi_args.push(arg);
                options.pi_args.extend(args);
                break;
            }
            return Err(format!(
                "unknown option: {}; pass Pi arguments after --",
                arg.to_string_lossy()
            )
            .into());
        }
        let value = inline
            .or_else(|| args.next())
            .ok_or_else(|| format!("{} requires a value", String::from_utf8_lossy(name)))?;
        match name {
            b"--project" => options.project = Some(value.into()),
            b"--pi-dir" => options.pi_dir = Some(value.into()),
            b"--state-root" => options.state_root = Some(value.into()),
            b"--env" => options.environment.push(value),
            b"--ignore" => options
                .ignores
                .push(value.into_string().map_err(|_| "--ignore requires UTF-8")?),
            _ => unreachable!(),
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(values: &[&str]) -> Result<Options> {
        parse_args(values.iter().map(OsString::from).collect())
    }
    #[test]
    fn options_and_passthrough() {
        let opts = parse(&[
            "--ignore=.env",
            "--ignore",
            "*.pem",
            "--network",
            "--",
            "--version",
        ])
        .unwrap();
        assert_eq!(opts.ignores, [".env", "*.pem"]);
        assert!(opts.network);
        assert_eq!(opts.pi_args, ["--version"]);
        let opts = parse(&["--exec", "/bin/sh", "-c", "exit 42"]).unwrap();
        assert_eq!(opts.command.unwrap(), ["/bin/sh", "-c", "exit 42"]);
    }
    #[test]
    fn bad_options_fail() {
        for args in [&["--exec"][..], &["--project"], &["--unknown"]] {
            assert!(parse(args).is_err());
        }
    }
}
