use crate::Result;
use std::{
    env,
    fs::{self, File},
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub struct Dependencies {
    pub bwrap: PathBuf,
    pub pi_dir: PathBuf,
}

fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn which(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join(name))
        .find(|p| executable(p))
        .and_then(|p| fs::canonicalize(p).ok())
}

fn pi_directory(explicit: Option<&Path>) -> Result<PathBuf> {
    let directory = match explicit {
        Some(path) => {
            let path = if let Ok(rest) = path.strip_prefix("~") {
                PathBuf::from(env::var_os("HOME").ok_or("HOME no está definido para expandir ~")?).join(rest)
            } else {
                path.to_path_buf()
            };
            fs::canonicalize(&path).map_err(|e| format!("Pi: no se puede acceder a {}: {e}", path.display()))?
        }
        None => which("pi").ok_or("Pi: no se encuentra el ejecutable pi en PATH. Instala la distribución autónoma desde https://pi.dev o usa --pi-dir /ruta/a/pi")?
            .parent().ok_or("Pi: no se puede determinar su directorio")?.to_path_buf(),
    };
    let binary = directory.join("pi");
    if !executable(&binary) {
        return Err(format!(
            "Pi: {} no existe o no es ejecutable; --pi-dir debe contener el ejecutable pi",
            binary.display()
        )
        .into());
    }
    let mut magic = [0; 4];
    let valid_elf = File::open(&binary)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map(|_| magic == *b"\x7fELF")
        .unwrap_or(false);
    if !valid_elf {
        return Err(format!("Pi: {} no es un binario ELF de la distribución autónoma; el wrapper npm no está soportado", binary.display()).into());
    }
    Ok(directory)
}

/// Validate both dependencies before inspecting the project or creating state.
/// Do not execute Pi on the host, auto-install tools or fall back to no sandbox.
pub fn check(pi_dir: Option<&Path>) -> Result<Dependencies> {
    let bwrap = which("bwrap");
    let pi = pi_directory(pi_dir);
    let mut errors = Vec::new();
    if bwrap.is_none() {
        errors.push("Bubblewrap: no se encuentra el ejecutable bwrap en PATH. En Arch: sudo pacman -S bubblewrap; en Debian/Ubuntu: sudo apt install bubblewrap".to_owned());
    }
    if let Err(error) = &pi {
        errors.push(error.to_string());
    }
    if !errors.is_empty() {
        return Err(format!(
            "dependencias no disponibles:\n  - {}",
            errors.join("\n  - ")
        )
        .into());
    }
    Ok(Dependencies {
        bwrap: bwrap.unwrap(),
        pi_dir: pi?,
    })
}
