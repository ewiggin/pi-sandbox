use crate::{
    Result,
    diagnostics::{PathContext, path_error},
    ignores::Pattern,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, symlink},
    },
    path::{Path, PathBuf},
};
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

// Keep the Python launcher's identity format byte-for-byte. This is a data
// encoding compatibility function, not a Python runtime dependency.
fn legacy_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::from(quote);
    for c in value.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c == quote => {
                result.push('\\');
                result.push(c);
            }
            c if c != ' '
                && matches!(
                    c.general_category_group(),
                    GeneralCategoryGroup::Other | GeneralCategoryGroup::Separator
                ) =>
            {
                let code = c as u32;
                if code <= 0xff {
                    result.push_str(&format!("\\x{code:02x}"));
                } else if code <= 0xffff {
                    result.push_str(&format!("\\u{code:04x}"));
                } else {
                    result.push_str(&format!("\\U{code:08x}"));
                }
            }
            c => result.push(c),
        }
    }
    result.push(quote);
    result
}

pub fn key(project: &Path, patterns: &[Pattern]) -> String {
    let mut hash = Sha256::new();
    hash.update(project.as_os_str().as_bytes());
    if !patterns.is_empty() {
        hash.update(b"\0ignores-v1\0");
        let encoded = patterns
            .iter()
            .map(|p| {
                format!(
                    "({}, {})",
                    legacy_repr(&p.text),
                    if p.directories_only { "True" } else { "False" }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        hash.update(format!("[{encoded}]"));
    }
    format!("{:x}", hash.finalize())
}

fn private_directory(path: &Path, recursive: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "state must be a directory, not a symbolic link: {}",
                    path.display()
                )
                .into());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder
                .mode(0o700)
                .recursive(recursive)
                .create(path)
                .at("create private state directory", path)?;
        }
        Err(e) => return Err(path_error("inspect private state directory", path, e)),
    }
    Ok(())
}

pub struct PreparedState {
    pub home: PathBuf,
    pub auth_directory: Option<PathBuf>,
}

pub fn validate_auth_profile(name: &str) -> Result<()> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 64
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(b))
    {
        return Err("invalid auth profile: use 1-64 ASCII letters, digits, dots, underscores or hyphens, starting with a letter or digit".into());
    }
    Ok(())
}

fn prepare_auth_directory(root: &Path, name: &str) -> Result<PathBuf> {
    let profiles = root.join("auth-profiles");
    private_directory(&profiles, false)?;
    let profile = profiles.join(name);
    private_directory(&profile, false)?;
    let auth = profile.join("auth.json");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&auth)
    {
        Ok(mut file) => file
            .write_all(b"{}\n")
            .at("initialize shared credentials", &auth)?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let meta = fs::symlink_metadata(&auth).at("inspect shared credentials", &auth)?;
            if !meta.is_file() || meta.is_symlink() || meta.nlink() != 1 {
                return Err(format!(
                    "shared credentials must be a regular file without symlinks or hardlinks: {}",
                    auth.display()
                )
                .into());
            }
        }
        Err(e) => return Err(path_error("create shared credentials", &auth, e)),
    }
    Ok(profile)
}

fn link_shared_auth(home: &Path) -> Result<()> {
    let pi = home.join(".pi");
    private_directory(&pi, false)?;
    let agent = pi.join("agent");
    private_directory(&agent, false)?;
    let auth = agent.join("auth.json");
    let target = Path::new("/auth-profile/auth.json");
    match fs::symlink_metadata(&auth) {
        Ok(meta) if meta.is_symlink() => {
            if fs::read_link(&auth).at("read shared credential link", &auth)? != target {
                return Err(format!("unexpected shared credential link: {}", auth.display()).into());
            }
        }
        Ok(_) => return Err(format!("expected a shared credential link, but found another file: {}; review it before removing it", auth.display()).into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            symlink(target, &auth).at("link shared credentials", &auth)?;
        }
        Err(e) => return Err(path_error("inspect shared credential link", &auth, e)),
    }
    Ok(())
}

pub fn prepare(
    root: &Path,
    project: &Path,
    patterns: &[Pattern],
    auth_profile: Option<&str>,
) -> Result<PreparedState> {
    if let Some(name) = auth_profile {
        validate_auth_profile(name)?;
    }
    private_directory(root, true)?;
    let state = root.join(key(project, patterns));
    private_directory(&state, false)?;
    let (home, auth_directory) = match auth_profile {
        None => (state.join("home"), None),
        Some(name) => {
            // A separate HOME prevents an opt-in shared profile from overwriting
            // or inheriting the project's original private credentials.
            let homes = state.join("auth-profile-homes");
            private_directory(&homes, false)?;
            (homes.join(name), Some(prepare_auth_directory(root, name)?))
        }
    };
    private_directory(&home, false)?;
    if auth_directory.is_some() {
        link_shared_auth(&home)?;
    }
    Ok(PreparedState {
        home,
        auth_directory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_string_encoding() {
        assert_eq!(legacy_repr(".env"), "'.env'");
        assert_eq!(legacy_repr("it's"), "\"it's\"");
        assert_eq!(legacy_repr("a'\"b\\\n"), "'a\\'\"b\\\\\\n'");
        assert_eq!(legacy_repr("ñ\u{a0}\u{200b}"), "'ñ\\xa0\\u200b'");
    }
    #[test]
    fn legacy_identity_fixtures() {
        // Generated by the original Python launcher's hashlib + repr encoding.
        let project = Path::new("/tmp/legacy-project");
        for (text, directories_only, expected) in [
            (
                ".env",
                false,
                "572e63d99e3a9c8eccf474d10c9ddbca47927818978c1dc53e5ae50fc022df9a",
            ),
            (
                "it's",
                false,
                "0fdcab08b6af8925cfec9136bcf0013954e442d8a1b906c1586bdbc212aa6c6f",
            ),
            (
                "a'\"b\\\n",
                false,
                "05bb3dd3da4c6022cef7d6415b3b47e53bc34238dbd377a9e7a51048bf722109",
            ),
            (
                "ñ\u{a0}\u{200b}",
                false,
                "4388a58afdc83eab0dfdcd3db951d0d6c69d02fe9327e41f24a28f30c0b848ca",
            ),
        ] {
            assert_eq!(
                key(
                    project,
                    &[Pattern {
                        text: text.into(),
                        directories_only
                    }]
                ),
                expected
            );
        }
        let patterns = vec![
            Pattern {
                text: "*.pem".into(),
                directories_only: false,
            },
            Pattern {
                text: ".env".into(),
                directories_only: false,
            },
            Pattern {
                text: "secrets".into(),
                directories_only: true,
            },
        ];
        assert_eq!(
            key(project, &patterns),
            "ee3b75c7fc6b31386f65a43a9586df587c46eed9b88749e0ef079a031493c4f3"
        );
    }

    #[test]
    fn no_rules_uses_path_sha256() {
        assert_eq!(
            key(Path::new("/project"), &[]),
            format!("{:x}", Sha256::digest(b"/project"))
        );
    }
}
