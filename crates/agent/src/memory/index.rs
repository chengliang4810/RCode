//! Only explicitly typed project/reference pointers move out of system.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::context::{Domain, Snapshot, Status};

#[derive(Debug)]
pub struct IndexContext {
    pub protected: String,
    pub facts: Snapshot,
}

fn fact_target(root: &Path, line: &str) -> Option<PathBuf> {
    static LINK: OnceLock<regex::Regex> = OnceLock::new();
    let link = LINK.get_or_init(|| {
        regex::Regex::new(r"^\s*[-*+]\s+\[[^\]\r\n]+\]\(([^()\r\n]+)\)(?:\s+.*)?$").unwrap()
    });
    let target = link
        .captures(line)?
        .get(1)?
        .as_str()
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');
    let path = root.join(target);
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    {
        return None;
    }
    if !std::fs::metadata(&path).ok()?.is_file() {
        return None;
    }
    // Header only, bounded; never load or inject the topic's body. Ordinary trust-all paths apply.
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .ok()?
        .take(4096)
        .read_to_end(&mut bytes)
        .ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.lines();
    if lines.next()?.trim_start_matches('\u{feff}') != "---" {
        return None;
    }
    let mut header = String::new();
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        header.push_str(line);
        header.push('\n');
    }
    if !closed {
        return None;
    }
    let value: yaml_serde::Value = yaml_serde::from_str(&header).ok()?;
    if !matches!(value.get("type")?.as_str()?, "project" | "reference") {
        return None;
    }
    Some(path.canonicalize().unwrap_or(path))
}

fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_owned();
    #[cfg(windows)]
    let text = text.to_lowercase();
    text
}

pub fn load(root: &Path) -> IndexContext {
    let scope = key(&root.join(super::INDEX_FILE));
    let index = std::fs::metadata(root).and_then(|metadata| {
        if metadata.is_dir() {
            super::read_index(root)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "memory root is not a directory",
            ))
        }
    });
    match index {
        Ok(index) => {
            let index = index.unwrap_or_default();
            let lines: Vec<_> = index
                .lines()
                .map(|line| (line, fact_target(root, line).map(|p| key(&p))))
                .collect();
            let mut counts = BTreeMap::<&str, usize>::new();
            for (_, path) in &lines {
                if let Some(path) = path {
                    *counts.entry(path).or_default() += 1;
                }
            }
            let mut protected = Vec::new();
            let mut entries = BTreeMap::new();
            for (line, path) in &lines {
                if let Some(path) = path.as_ref().filter(|p| counts[p.as_str()] == 1) {
                    entries.insert(path.clone(), (*line).to_owned());
                } else {
                    protected.push(*line);
                }
            }
            IndexContext {
                protected: protected.join("\n"),
                facts: Snapshot {
                    domain: Domain::MemoryIndex,
                    scope,
                    status: Status::Ready,
                    entries,
                },
            }
        }
        Err(error) => {
            tracing::warn!(path = %root.display(), %error, "cannot read automatic memory index");
            IndexContext { protected: "(The index could not be read. Inspect it with the file tools before changing it.)".into(),
                facts: Snapshot { domain: Domain::MemoryIndex, scope, status: Status::Unavailable, entries: BTreeMap::new() } }
        }
    }
}

#[cfg(test)]
mod tests;
