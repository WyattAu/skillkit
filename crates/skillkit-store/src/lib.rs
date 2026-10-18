//! skillkit-store — deterministic skill archives in a content-addressed
//! store ([`cas_kit`], BLAKE3 + zstd).
//!
//! The archive format is canonical: a `SKILLPACK1` header, the sorted
//! file count, then length-prefixed `(relative path, content)` pairs
//! with paths sorted byte-wise. Same directory ⇒ same bytes ⇒ same
//! hash, so `verify` is a hash comparison, and identical skills
//! deduplicate in the store for free.

use std::io::Read;

/// Errors from archiving or store access.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Filesystem failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// cas-kit store failure.
    #[error("cas store error: {0}")]
    Cas(#[from] cas_kit::CasError),
    /// A required input was wrong.
    #[error("{0}")]
    Other(String),
}

/// A canonical archive of one skill directory.
#[derive(Debug, Clone)]
pub struct Archive {
    /// Canonical archive bytes (what gets hashed and stored).
    pub bytes: Vec<u8>,
    /// How many files were archived.
    pub file_count: usize,
}

/// Deterministically walk `dir`, returning sorted `(relative path,
/// bytes)` pairs. Hidden entries and `target/` are skipped.
fn collect_files(dir: &std::path::Path) -> Result<Vec<(String, Vec<u8>)>, StoreError> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&current)
            .map_err(StoreError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name: String = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            let path = current.join(&name);
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel: String = path
                    .strip_prefix(dir)
                    .map_err(|e| StoreError::Other(format!("path walk failed: {e}")))?
                    .to_string_lossy()
                    .into_owned();
                let mut content = Vec::new();
                std::fs::File::open(&path)
                    .map_err(StoreError::from)?
                    .read_to_end(&mut content)
                    .map_err(StoreError::from)?;
                out.push((rel, content));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Build the canonical archive for a skill directory.
///
/// # Errors
/// Fails on unreadable files or paths outside the directory.
pub fn archive_dir(dir: &std::path::Path) -> Result<Archive, StoreError> {
    if !dir.join("SKILL.md").is_file() {
        return Err(StoreError::Other(format!(
            "{} contains no SKILL.md",
            dir.display()
        )));
    }
    let files = collect_files(dir)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"SKILLPACK1\n");
    bytes.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for (path, content) in &files {
        bytes.extend_from_slice(&(path.len() as u64).to_le_bytes());
        bytes.extend_from_slice(path.as_bytes());
        bytes.extend_from_slice(&(content.len() as u64).to_le_bytes());
        bytes.extend_from_slice(content);
    }
    let file_count = files.len();
    Ok(Archive { bytes, file_count })
}

/// Store archive `bytes` under `root`; returns the hex hash.
///
/// # Errors
/// Propagates cas-kit failures.
pub fn put(root: &std::path::Path, bytes: &[u8]) -> Result<String, StoreError> {
    let store = cas_kit::store::BlobStore::new(root)?;
    Ok(store.put_blob(bytes)?.to_hex())
}

/// Whether `hex` is present in the store at `root`.
#[must_use]
pub fn has(root: &std::path::Path, hex: &str) -> bool {
    let Ok(store) = cas_kit::store::BlobStore::new(root) else {
        return false;
    };
    let Ok(hash) = cas_kit::Hash::from_hex(hex) else {
        return false;
    };
    store.has_blob(&hash)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn skill_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("skillkit-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo\ndescription: demo skill\n---\nbody",
        )
        .unwrap();
        std::fs::write(dir.join("scripts/run.sh"), "echo hi").unwrap();
        dir
    }

    #[test]
    fn archiving_is_deterministic() {
        let a = archive_dir(&skill_dir("a")).unwrap();
        let b = archive_dir(&skill_dir("b")).unwrap();
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.file_count, 2);
    }

    #[test]
    fn content_change_changes_hash() {
        let dir = skill_dir("c");
        let a = archive_dir(&dir).unwrap();
        std::fs::write(dir.join("scripts/run.sh"), "echo changed").unwrap();
        let b = archive_dir(&dir).unwrap();
        assert_ne!(
            cas_kit::hash_bytes(&a.bytes).to_hex(),
            cas_kit::hash_bytes(&b.bytes).to_hex()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_and_has_roundtrip() {
        let dir = skill_dir("d");
        let archive = archive_dir(&dir).unwrap();
        let root = std::env::temp_dir().join(format!("skillkit-cas-{}", std::process::id()));
        let hex = put(&root, &archive.bytes).unwrap();
        assert!(has(&root, &hex));
        assert!(!has(&root, &"0".repeat(64)));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_without_skill_md_is_rejected() {
        let dir = std::env::temp_dir().join(format!("skillkit-empty-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        assert!(archive_dir(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
