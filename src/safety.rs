// One authoritative whitelist/hash implementation, shared by GUI and launcher.
pub use crate::hash_cache::HashCache;
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
    /// The matching whitelist entry for Safe and Blocked items.
    pub review: Option<Review>,
}
impl Status {
    pub fn unreviewed(reason: impl Into<String>) -> Self {
        Self {
            label: "Unreviewed",
            allowed: false,
            reason: reason.into(),
            review: None,
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
/// With `cache`, an unchanged tree reuses its last hash; that is fast enough
/// for badges but not proof of unchanged content (see `hash_cache`), so
/// authorizing callers pass None and hash the whole tree.
pub fn assess(
    reviews: &Whitelist,
    id: &str,
    directory: &Path,
    cache: Option<&HashCache>,
) -> Status {
    let Some(r) = reviews.get(id) else {
        return Status::unreviewed("No review for this Workshop ID.");
    };
    if r.verdict == Verdict::Blocked {
        return Status {
            label: "Blocked",
            allowed: false,
            reason: format!("Blocked by {}: {}", r.reviewer, r.notes),
            review: Some(r.clone()),
        };
    }
    let hash = match cache {
        Some(cache) => cache.content_hash(id, directory),
        None => content_hash(directory),
    };
    match hash {
        Ok(hash) if hash == r.content_hash => Status {
            label: "Safe",
            allowed: true,
            reason: format!("Reviewed {} by {}. {}", r.reviewed, r.reviewer, r.notes),
            review: Some(r.clone()),
        },
        Ok(_) => Status::unreviewed("Content changed since review. A new review is required."),
        Err(e) => Status::unreviewed(format!("Cannot verify content: {e:#}")),
    }
}
/// Every installed copy of `id` across the given Steam library roots.
fn installed_copies(id: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .map(|root| root.join("steamapps/workshop/content/431960").join(id))
        .filter(|path| path.symlink_metadata().is_ok())
        .collect()
}
/// The single installed copy of `id`. Several copies are ambiguous (the
/// renderer could be pointed at an unreviewed one), so they fail closed.
pub fn resolve_in(id: &str, roots: &[PathBuf]) -> Result<PathBuf> {
    ensure!(valid_id(id), "invalid Workshop ID");
    let mut paths = installed_copies(id, roots);
    ensure!(
        paths.len() == 1,
        "Workshop {id}: expected one installed copy, found {}",
        paths.len()
    );
    Ok(paths.remove(0))
}
pub fn resolve(id: &str) -> Result<PathBuf> {
    resolve_in(id, &crate::library::libraries())
}
/// The status Apply and the launcher will reach for an item the gallery
/// scanned at `directory`: the review must match and `directory` must be the
/// only installed copy, exactly as `verify` requires.
pub fn assess_installed(
    reviews: &Whitelist,
    id: &str,
    directory: &Path,
    roots: &[PathBuf],
    cache: Option<&HashCache>,
) -> Status {
    match resolve_in(id, roots) {
        Ok(path) if path == directory => assess(reviews, id, directory, cache),
        Ok(path) => Status::unreviewed(format!(
            "Installed copy moved to {}; reopen the gallery.",
            path.display()
        )),
        Err(e) => Status::unreviewed(format!("{e:#}")),
    }
}
/// Authorize `id` for running: always a full hash, never the cache.
pub fn verify(id: &str) -> Result<PathBuf> {
    let reviews = read_whitelist(&whitelist_path())?;
    let path = resolve(id)?;
    let status = assess(&reviews, id, &path, None);
    ensure!(status.allowed, "Workshop {id}: {}", status.reason);
    Ok(path.canonicalize()?)
}
/// Run `work` over `items` on a few threads and hand each result to `sink`
/// on the calling thread as it completes. A few readers keep a disk busy
/// without thrashing it. Returning false from `sink` stops the remaining work.
pub fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    work: impl Fn(&T) -> R + Sync,
    mut sink: impl FnMut(usize, R) -> bool,
) {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 4)
        .min(items.len().max(1));
    let (next, stop) = (AtomicUsize::new(0), AtomicBool::new(false));
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (sender, next, stop, work) = (sender.clone(), &next, &stop, &work);
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    if sender.send((index, work(item))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for (index, result) in receiver {
            if !sink(index, result) {
                stop.store(true, Ordering::Relaxed);
                break;
            }
        }
    });
}
// Authoritative format: wpe-gallery-content-v1. Same stream as tools/triage:
// sorted UTF-8 relative paths, D/F tags, big-endian u64 path lengths;
// files add big-endian u64 size and raw bytes. Include empty directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    pub(crate) size: u64,
    pub(crate) modified: std::time::SystemTime,
    pub(crate) directory: bool,
    /// (dev, ino, ctime seconds, ctime nanoseconds).
    #[cfg(unix)]
    pub(crate) identity: (u64, u64, i64, i64),
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
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) metadata: Fingerprint,
}
pub fn content_hash(root: &Path) -> Result<String> {
    Ok(hash_tree(root, snapshot(root)?, |_| {})?.0)
}
/// The lstat view of an item tree: the root plus every entry, sorted by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) root: Fingerprint,
    pub(crate) entries: Vec<Entry>,
}
pub(crate) fn snapshot(root: &Path) -> Result<Snapshot> {
    let root_metadata = fingerprint(&fs::symlink_metadata(root)?)?;
    ensure!(
        root_metadata.directory,
        "item root must be a real directory"
    );
    let mut entries = Vec::new();
    collect(root, root, &mut entries)?;
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    Ok(Snapshot {
        root: root_metadata,
        entries,
    })
}
/// Hash `root` from its snapshot `before`, failing unless the tree still
/// matches it afterwards; returns the hash and that stable snapshot.
pub(crate) fn hash_tree(
    root: &Path,
    before: Snapshot,
    mut after_file: impl FnMut(&Path),
) -> Result<(String, Snapshot)> {
    let mut hash = Sha256::new();
    hash.update(b"wpe-gallery-content-v1\0");
    let mut buffer = vec![0u8; 128 * 1024];
    for entry in &before.entries {
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
    ensure!(snapshot(root)? == before, "item changed while hashing");
    Ok((format!("{:x}", hash.finalize()), before))
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
        let cache = HashCache::in_memory();
        let mut reviews = BTreeMap::from([("123".into(), review(hash.clone(), Verdict::Safe))]);
        assert!(assess(&reviews, "123", d.path(), Some(&cache)).allowed);
        assert!(!assess(&reviews, "456", d.path(), Some(&cache)).allowed);
        fs::write(d.path().join("a"), b"two").unwrap();
        assert!(!assess(&reviews, "123", d.path(), Some(&cache)).allowed);
        fs::write(d.path().join("a"), b"one").unwrap();
        fs::rename(d.path().join("a"), d.path().join("b")).unwrap();
        assert_ne!(hash, content_hash(d.path()).unwrap());
        fs::rename(d.path().join("b"), d.path().join("a")).unwrap();
        fs::create_dir(d.path().join("empty")).unwrap();
        assert_ne!(hash, content_hash(d.path()).unwrap());
        reviews.get_mut("123").unwrap().verdict = Verdict::Blocked;
        assert_eq!(
            assess(&reviews, "123", d.path(), Some(&cache)).label,
            "Blocked"
        );
    }
    #[test]
    fn parallel_map_delivers_each_index_once_and_stops_early() {
        let items: Vec<usize> = (0..100).collect();
        let mut seen = vec![0; items.len()];
        parallel_map(
            &items,
            |n| n * 2,
            |i, doubled| {
                assert_eq!(doubled, i * 2);
                seen[i] += 1;
                true
            },
        );
        assert!(seen.iter().all(|&n| n == 1));
        let mut delivered = 0;
        parallel_map(
            &items,
            |n| *n,
            |_, _| {
                delivered += 1;
                delivered < 5
            },
        );
        assert_eq!(delivered, 5);
    }
    #[test]
    fn duplicate_installs_fail_closed_in_gallery_and_launcher() {
        let d = tempfile::tempdir().unwrap();
        let roots = vec![d.path().join("a"), d.path().join("b")];
        let item = |root: &Path| root.join("steamapps/workshop/content/431960/123");
        fs::create_dir_all(item(&roots[0])).unwrap();
        fs::write(item(&roots[0]).join("a"), b"one").unwrap();
        let reviews = BTreeMap::from([(
            "123".into(),
            review(content_hash(&item(&roots[0])).unwrap(), Verdict::Safe),
        )]);
        assert!(assess_installed(&reviews, "123", &item(&roots[0]), &roots, None).allowed);
        fs::create_dir_all(item(&roots[1])).unwrap();
        let status = assess_installed(&reviews, "123", &item(&roots[0]), &roots, None);
        assert!(!status.allowed);
        assert!(status.reason.contains("found 2"), "{}", status.reason);
        assert!(resolve_in("123", &roots).is_err());
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
            hash_tree(d.path(), snapshot(d.path()).unwrap(), |path| {
                fs::write(d.path().join("replacement"), b"modified").unwrap();
                fs::rename(d.path().join("replacement"), path).unwrap();
            })
            .is_err()
        );
    }
}
