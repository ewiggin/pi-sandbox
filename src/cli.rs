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

const HELP: &str = "Pi aislado con Bubblewrap. Sin red salvo --network.

Uso: pi-sandbox [opciones] [-- argumentos de Pi...]

  --check            Comprobar Pi y Bubblewrap sin iniciar el sandbox
  --project PATH     Proyecto RW (por defecto: carpeta actual)
  --ignore PATTERN   Excluir ruta/glob; repetible, suma .pi-sandboxignore
  --network          Compartir TODA la red del host, incluido localhost
  --pi-dir PATH      Distribución autónoma de Pi (directorio que contiene pi)
  --state-root PATH  Estado privado por proyecto, fuera del proyecto
  --env NAME         Pasar explícitamente una variable del host; repetible
  --exec COMMAND ... Ejecutar un comando dentro; debe ser la última opción
  -h, --help         Mostrar esta ayuda
  -V, --version      Mostrar la versión del launcher

Ejemplos:
  pi-sandbox --ignore .env --ignore secrets/ --network
  pi-sandbox --project /ruta/proyecto --network -- --model openai-codex/gpt-5.4
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
                return Err("--exec requiere un comando".into());
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
                "opción desconocida: {}; pasa los argumentos de Pi después de --",
                arg.to_string_lossy()
            )
            .into());
        }
        let value = inline
            .or_else(|| args.next())
            .ok_or_else(|| format!("{} requiere un valor", String::from_utf8_lossy(name)))?;
        match name {
            b"--project" => options.project = Some(value.into()),
            b"--pi-dir" => options.pi_dir = Some(value.into()),
            b"--state-root" => options.state_root = Some(value.into()),
            b"--env" => options.environment.push(value),
            b"--ignore" => options
                .ignores
                .push(value.into_string().map_err(|_| "--ignore requiere UTF-8")?),
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
