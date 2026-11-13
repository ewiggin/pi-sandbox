use crate::{
    Result,
    diagnostics::{PathContext, path_error},
};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{self, Metadata},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Pattern {
    pub text: String,
    pub directories_only: bool,
}

pub fn load(project: &Path, extra: &[String]) -> Result<Vec<Pattern>> {
    let config = project.join(".pi-sandboxignore");
    let mut lines = Vec::new();
    match fs::symlink_metadata(&config) {
        Ok(meta) => {
            if meta.is_symlink() {
                return Err(".pi-sandboxignore must not be a symbolic link".into());
            }
            if !meta.is_file() {
                return Err(".pi-sandboxignore must be a regular file".into());
            }
            lines.extend(
                fs::read_to_string(&config)
                    .at("read ignore file", &config)?
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.starts_with('#'))
                    .map(str::to_owned),
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(path_error("inspect ignore file", &config, e)),
    }
    lines.extend_from_slice(extra);
    let mut patterns = Vec::new();
    for value in lines {
        let value = value.trim();
        let mut value = value;
        while let Some(rest) = value.strip_prefix("./") {
            value = rest;
        }
        let directories_only = value.ends_with('/');
        let text = value.trim_end_matches('/');
        if text.is_empty()
            || text.starts_with(['/', '!'])
            || text.contains('\0')
            || text.split('/').any(|s| matches!(s, "" | "." | ".."))
        {
            return Err(format!(
                "invalid exclusion (use relative paths without .. or negations): {text:?}"
            )
            .into());
        }
        patterns.push(Pattern {
            text: text.to_owned(),
            directories_only,
        });
    }
    patterns.sort();
    patterns.dedup();
    Ok(patterns)
}

#[derive(Debug)]
enum Token {
    Any,
    One,
    Literal(char),
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

// Python fnmatch dialect: no backslash escaping; ! negates a bracket class.
// Match Unicode characters rather than locale-dependent bytes.
fn tokens(pattern: &str) -> Vec<Token> {
    let chars: Vec<_> = pattern.chars().collect();
    let mut result = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => result.push(Token::Any),
            '?' => result.push(Token::One),
            '[' => {
                let start = i + 1;
                let negated = chars.get(start) == Some(&'!');
                let content = start + usize::from(negated);
                let search = content + usize::from(chars.get(content) == Some(&']'));
                if let Some(end) = (search..chars.len()).find(|j| chars[*j] == ']') {
                    let mut ranges = Vec::new();
                    let mut k = content;
                    while k < end {
                        if k + 2 < end && chars[k + 1] == '-' {
                            ranges.push((chars[k], chars[k + 2]));
                            k += 3;
                        } else {
                            ranges.push((chars[k], chars[k]));
                            k += 1;
                        }
                    }
                    result.push(Token::Class { negated, ranges });
                    i = end;
                } else {
                    result.push(Token::Literal('['));
                }
            }
            c => result.push(Token::Literal(c)),
        }
        i += 1;
    }
    result
}

fn name_matches(pattern: &str, name: &str) -> bool {
    let name: Vec<_> = name.chars().collect();
    let mut previous = vec![false; name.len() + 1];
    previous[0] = true;
    for token in tokens(pattern) {
        let mut next = vec![false; name.len() + 1];
        if matches!(token, Token::Any) {
            next[0] = previous[0];
        }
        for (i, c) in name.iter().enumerate() {
            next[i + 1] = match &token {
                Token::Any => previous[i + 1] || next[i],
                Token::One => previous[i],
                Token::Literal(literal) => previous[i] && c == literal,
                Token::Class { negated, ranges } => {
                    previous[i] && (ranges.iter().any(|(a, b)| a <= c && c <= b) != *negated)
                }
            };
        }
        previous = next;
    }
    previous[name.len()]
}

fn match_parts(pattern: &[&str], parts: &[&str]) -> bool {
    let mut previous = vec![false; parts.len() + 1];
    previous[0] = true;
    for component in pattern {
        let mut next = vec![false; parts.len() + 1];
        if *component == "**" {
            next[0] = previous[0];
        }
        for i in 0..parts.len() {
            next[i + 1] = if *component == "**" {
                previous[i + 1] || next[i]
            } else {
                previous[i] && name_matches(component, parts[i])
            };
        }
        previous = next;
    }
    previous[parts.len()]
}

fn matches(pattern: &str, relative: &str) -> bool {
    if !pattern.contains('/') {
        name_matches(pattern, relative.rsplit('/').next().unwrap())
    } else {
        match_parts(
            &pattern.split('/').collect::<Vec<_>>(),
            &relative.split('/').collect::<Vec<_>>(),
        )
    }
}

struct Entry {
    relative: PathBuf,
    metadata: Metadata,
}

fn walk(
    project: &Path,
    directory: &Path,
    inherited: bool,
    patterns: &[Pattern],
    entries: &mut Vec<Entry>,
    excluded: &mut HashSet<PathBuf>,
    hidden_inodes: &mut HashSet<(u64, u64)>,
) -> Result<()> {
    let mut children = fs::read_dir(directory)
        .at(
            "scan project directory for exclusions and hardlink aliases",
            directory,
        )?
        .collect::<std::io::Result<Vec<_>>>()
        .at("read project directory entries", directory)?;
    children.sort_by_key(|e| e.file_name());
    for child in children {
        let path = child.path();
        let relative = path.strip_prefix(project)?.to_path_buf();
        // Never silently fail to match an undecodable secret filename.
        let name = relative.to_str().ok_or_else(|| {
            format!(
                "non-UTF-8 path with active exclusions: {}; rename it before starting",
                relative.display()
            )
        })?;
        let metadata = fs::symlink_metadata(&path).at("inspect project entry", &path)?;
        let is_dir = metadata.is_dir();
        let hidden = inherited
            || patterns
                .iter()
                .any(|p| matches(&p.text, name) && (!p.directories_only || is_dir));
        if metadata.is_symlink() && !inherited {
            let traversed = patterns.iter().any(|p| {
                let parts: Vec<_> = p.text.split('/').collect();
                let relative_parts: Vec<_> = name.split('/').collect();
                (1..parts.len()).any(|n| {
                    // A prefix made only of ** imposes no path constraint.
                    // For **/postgres-data, the real directory is masked wherever
                    // it occurs; an unrelated symlink cannot bypass that mask.
                    // Keep rejecting aliases that match a meaningful prefix,
                    // e.g. src -> actual with the rule src/**/*.key.
                    parts[..n].iter().any(|part| *part != "**")
                        && match_parts(&parts[..n], &relative_parts)
                })
            });
            if hidden
                || traversed
                || patterns
                    .iter()
                    .any(|p| p.directories_only && matches(&p.text, name))
            {
                return Err(format!("exclusion on/through a symlink is not supported: {name}; exclude its real target or parent directory").into());
            }
        }
        if hidden {
            excluded.insert(relative.clone());
            if !is_dir && !metadata.is_symlink() {
                hidden_inodes.insert((metadata.dev(), metadata.ino()));
            }
        }
        entries.push(Entry { relative, metadata });
        // Also scan excluded directories to find hardlink aliases elsewhere.
        if is_dir {
            walk(
                project,
                &path,
                hidden,
                patterns,
                entries,
                excluded,
                hidden_inodes,
            )?;
        }
    }
    Ok(())
}

pub fn mounts(project: &Path, patterns: &[Pattern]) -> Result<(Vec<OsString>, usize)> {
    if patterns.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let mut entries = Vec::new();
    let mut excluded = HashSet::new();
    let mut hidden_inodes = HashSet::new();
    walk(
        project,
        project,
        false,
        patterns,
        &mut entries,
        &mut excluded,
        &mut hidden_inodes,
    )?;
    for entry in &entries {
        if !entry.metadata.is_dir()
            && hidden_inodes.contains(&(entry.metadata.dev(), entry.metadata.ino()))
        {
            excluded.insert(entry.relative.clone());
        }
    }
    let mut args = Vec::new();
    let mut count = 0;
    for entry in entries {
        if !excluded.contains(&entry.relative)
            || entry
                .relative
                .ancestors()
                .skip(1)
                .any(|p| excluded.contains(p))
        {
            continue;
        }
        let destination = Path::new("/workspace")
            .join(&entry.relative)
            .into_os_string();
        if entry.metadata.is_dir() {
            args.extend([
                "--tmpfs".into(),
                destination.clone(),
                "--chmod".into(),
                "000".into(),
                destination.clone(),
                "--remount-ro".into(),
                destination,
            ]);
        } else {
            args.extend(["--ro-bind".into(), "/dev/null".into(), destination]);
        }
        count += 1;
    }
    Ok((args, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wildcard_dialect() {
        for (pattern, name, expected) in [
            ("*.pem", "deep/key.pem", true),
            ("src/*.pem", "src/deep/key.pem", false),
            ("src/**/*.pem", "src/key.pem", true),
            ("src/**/*.pem", "src/deep/key.pem", true),
            ("[!a-c]?.pem", "dé.pem", true),
            ("[a-c].pem", "d.pem", false),
            ("[]].pem", "].pem", true),
            ("a\\b", "a\\b", true),
            ("[unclosed", "[unclosed", true),
            ("?.pem", "ñ.pem", true),
        ] {
            assert_eq!(matches(pattern, name), expected, "{pattern} {name}");
        }
    }
    #[test]
    fn rules_normalized_and_sorted() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(".pi-sandboxignore"),
            "# comment\n.env\nsecrets/\n",
        )
        .unwrap();
        let rules = load(temp.path(), &["./.env".into(), "*.pem".into()]).unwrap();
        assert_eq!(
            rules.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            ["*.pem", ".env", "secrets"]
        );
        assert!(rules[2].directories_only);
    }
}
