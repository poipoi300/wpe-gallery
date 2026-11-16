// One authoritative whitelist/hash implementation, shared by GUI and launcher.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Safe,
    Blocked,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
// Review producers may attach metadata (for example `type`). Serde ignores
// those fields; the authorization fields below remain required and validated.
pub struct Review {
    pub verdict: Verdict,
    pub reviewed: String,
    pub reviewer: String,
    pub notes: String,
    pub content_hash: String,
}
pub type Whitelist = BTreeMap<String, Review>;
#[derive(Clone, Debug)]
pub struct Status {
    pub label: &'static str,
    pub allowed: bool,
    pub reason: String,
}
impl Status {
    pub fn unreviewed(reason: impl Into<String>) -> Self {
        Self {
            label: "Unreviewed",
            allowed: false,
            reason: reason.into(),
        }
    }
}
pub fn whitelist_path() -> PathBuf {
    directories::BaseDirs::new()
        .expect("home directory")
        .config_dir()
        .join("wpe-gallery/whitelist.json")
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}
pub fn read_whitelist(path: &Path) -> Result<Whitelist> {
    let reviews: Whitelist = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("Read {}", path.display()))?,
    )?;
    for (id, r) in &reviews {
        ensure!(valid_id(id), "invalid Workshop ID in whitelist");
        ensure!(
            r.reviewed.len() == 10
                && r.reviewed.as_bytes()[4] == b'-'
                && r.reviewed.as_bytes()[7] == b'-'
                && r.reviewed
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()),
            "invalid review date for {id}; use YYYY-MM-DD"
        );
        ensure!(!r.reviewer.trim().is_empty(), "missing reviewer for {id}");
        ensure!(
            r.content_hash.len() == 64
                && r.content_hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "invalid SHA-256 for {id}"
        );
    }
    Ok(reviews)
}
pub fn assess(reviews: &Whitelist, id: &str, directory: &Path) -> Status {
    let Some(r) = reviews.get(id) else {
        return Status::unreviewed("No review for this Workshop ID.");
    };
    if r.verdict == Verdict::Blocked {
        return Status {
            label: "Blocked",
            allowed: false,
            reason: format!("Blocked by {}: {}", r.reviewer, r.notes),
        };
    }
    match content_hash(directory) {
        Ok(hash) if hash == r.content_hash => Status {
            label: "Safe",
            allowed: true,
            reason: format!("Reviewed {} by {}. {}", r.reviewed, r.reviewer, r.notes),
        },
        Ok(_) => Status::unreviewed("Content changed since review. A new review is required."),
        Err(e) => Status::unreviewed(format!("Cannot verify content: {e:#}")),
    }
}
pub fn resolve(id: &str) -> Result<PathBuf> {
    ensure!(valid_id(id), "invalid Workshop ID");
    let mut paths = Vec::new();
    for root in crate::library::libraries() {
        let path = root.join("steamapps/workshop/content/431960").join(id);
        if path.exists() {
            paths.push(path);
        }
    }
    ensure!(
        paths.len() == 1,
        "Workshop {id}: expected one installed copy, found {}",
        paths.len()
    );
    Ok(paths.remove(0))
}
pub fn verify(id: &str) -> Result<PathBuf> {
    let reviews = read_whitelist(&whitelist_path())?;
    let path = resolve(id)?;
    let status = assess(&reviews, id, &path);
    ensure!(status.allowed, "Workshop {id}: {}", status.reason);
    Ok(path.canonicalize()?)
}
// Authoritative format: wpe-gallery-content-v1. Same stream as tools/triage:
// sorted UTF-8 relative paths, D/F tags, big-endian u64 path lengths;
// files add big-endian u64 size and raw bytes. Include empty directories.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    size: u64,
    modified: std::time::SystemTime,
    directory: bool,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
fn fingerprint(metadata: &fs::Metadata) -> Result<Fingerprint> {
    ensure!(
        !metadata.file_type().is_symlink() && (metadata.is_file() || metadata.is_dir()),
        "links and special files are not accepted"
    );
    Ok(Fingerprint {
        size: metadata.len(),
        modified: metadata.modified()?,
        directory: metadata.is_dir(),
        #[cfg(unix)]
        identity: {
            use std::os::unix::fs::MetadataExt;
            (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            )
        },
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    name: String,
    path: PathBuf,
    metadata: Fingerprint,
}
pub fn content_hash(root: &Path) -> Result<String> {
    hash_with_hook(root, |_| {})
}
fn hash_with_hook(root: &Path, mut after_file: impl FnMut(&Path)) -> Result<String> {
    let original_root = fingerprint(&fs::symlink_metadata(root)?)?;
    ensure!(
        original_root.directory,
        "item root must be a real directory"
    );
    let mut entries = Vec::new();
    collect(root, root, &mut entries)?;
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let mut hash = Sha256::new();
    hash.update(b"wpe-gallery-content-v1\0");
    let mut buffer = vec![0u8; 128 * 1024];
    for entry in &entries {
        hash.update(if entry.metadata.directory { b"D" } else { b"F" });
        hash.update((entry.name.len() as u64).to_be_bytes());
        hash.update(entry.name.as_bytes());
        if !entry.metadata.directory {
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            let mut file = options.open(&entry.path)?;
            ensure!(
                fingerprint(&file.metadata()?)? == entry.metadata,
                "file changed while opening: {}",
                entry.name
            );
            hash.update(entry.metadata.size.to_be_bytes());
            let mut count = 0u64;
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                hash.update(&buffer[..n]);
            }
            ensure!(
                count == entry.metadata.size && fingerprint(&file.metadata()?)? == entry.metadata,
                "file changed while hashing: {}",
                entry.name
            );
            after_file(&entry.path);
        }
    }
    let mut after = Vec::new();
    collect(root, root, &mut after)?;
    after.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    ensure!(
        original_root == fingerprint(&fs::symlink_metadata(root)?)? && entries == after,
        "item changed while hashing"
    );
    Ok(format!("{:x}", hash.finalize()))
}
fn collect(root: &Path, directory: &Path, entries: &mut Vec<Entry>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let metadata = fingerprint(&fs::symlink_metadata(&path)?)?;
        let name = path
            .strip_prefix(root)?
            .components()
            .map(|c| c.as_os_str().to_str().context("non-UTF-8 item path"))
            .collect::<Result<Vec<_>>>()?
            .join("/");
        let is_dir = metadata.directory;
        entries.push(Entry {
            name,
            path: path.clone(),
            metadata,
        });
        if is_dir {
            collect(root, &path, entries)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn review(hash: String, verdict: Verdict) -> Review {
        Review {
            verdict,
            content_hash: hash,
            reviewed: "2026-11-16".into(),
            reviewer: "fixture reviewer".into(),
            notes: "synthetic test, not a real review".into(),
        }
    }
    #[test]
    fn hash_tracks_contents_names_empty_directories_and_verdicts() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a"), b"one").unwrap();
        let hash = content_hash(d.path()).unwrap();
        let mut reviews = BTreeMap::from([("123".into(), review(hash.clone(), Verdict::Safe))]);
        assert!(assess(&reviews, "123", d.path()).allowed);
        assert!(!assess(&reviews, "456", d.path()).allowed);
        fs::write(d.path().join("a"), b"two").unwrap();
        assert!(!assess(&reviews, "123", d.path()).allowed);
        fs::write(d.path().join("a"), b"one").unwrap();
        fs::rename(d.path().join("a"), d.path().join("b")).unwrap();
        assert_ne!(hash, content_hash(d.path()).unwrap());
        fs::rename(d.path().join("b"), d.path().join("a")).unwrap();
        fs::create_dir(d.path().join("empty")).unwrap();
        assert_ne!(hash, content_hash(d.path()).unwrap());
        reviews.get_mut("123").unwrap().verdict = Verdict::Blocked;
        assert_eq!(assess(&reviews, "123", d.path()).label, "Blocked");
    }
    #[cfg(unix)]
    #[test]
    fn links_fail_closed() {
        let d = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc/passwd", d.path().join("linked")).unwrap();
        assert!(content_hash(d.path()).is_err());
    }
    #[test]
    fn atomic_replacement_during_verification_fails_closed() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a"), b"reviewed").unwrap();
        assert!(
            hash_with_hook(d.path(), |path| {
                fs::write(d.path().join("replacement"), b"modified").unwrap();
                fs::rename(d.path().join("replacement"), path).unwrap();
            })
            .is_err()
        );
    }
}
