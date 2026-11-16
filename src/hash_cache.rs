// Verified-hash cache for gallery badges: skips rereading an item whose
// lstat tree is unchanged since it was last hashed.
//
// Each record holds a digest of every entry's size, mtime, dev, inode and
// ctime, root included. Userland can restore mtime (utimensat) but not ctime,
// which the kernel stamps on write(2), truncate, rename and metadata changes;
// so a same-size edit with mtime set back still changes the fingerprint.
// Two limits narrow the cases where ctime lags behind content:
// - Only filesystems whose ctime the kernel owns are trusted (ntfs-3g, for
//   example, lets users set it through an xattr).
// - A tree is cached only when every ctime predates hashing by
//   `racy_margin`: with coarse timestamps a write in the same tick as the
//   read keeps ctime.
// ctime is still not a content guarantee: stores through a shared writable
// mmap to an already-dirty page change bytes without touching ctime (always
// on tmpfs, within the writeback window elsewhere), and stepping the clock
// back can replay old ctimes. The cache therefore only feeds the gallery's
// badges. Apply and the launcher always hash the full tree (`safety::verify`),
// so a stale record can mislabel a tile but never authorize content.
use crate::safety::{Snapshot, hash_tree, snapshot};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

const VERSION: u32 = 1;
/// Pending records are written at most this long after they were computed.
const SAVE_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Record {
    path: String,
    fingerprint: String,
    content_hash: String,
}
#[derive(Deserialize, Serialize)]
struct CacheFile {
    version: u32,
    items: BTreeMap<String, Record>,
}
#[derive(Default)]
struct State {
    records: BTreeMap<String, Record>,
    unsaved: BTreeMap<String, Record>,
    removed: BTreeSet<String>,
    last_save: Option<Instant>,
}
pub struct HashCache {
    path: Option<PathBuf>,
    racy_margin: Duration,
    state: Mutex<State>,
    /// Serializes this process's load-merge-write cycles.
    writing: Mutex<()>,
    #[cfg(test)]
    full_hashes: std::sync::atomic::AtomicUsize,
}
pub fn cache_path() -> PathBuf {
    directories::BaseDirs::new()
        .expect("home directory")
        .cache_dir()
        .join("wpe-gallery/hashes.v1")
}
/// Records from `path`; a missing, corrupt or other-version file yields none.
fn load(path: &Path) -> BTreeMap<String, Record> {
    let Ok(bytes) = fs::read(path) else {
        return BTreeMap::new();
    };
    match serde_json::from_slice::<CacheFile>(&bytes) {
        Ok(file) if file.version == VERSION => file.items,
        Ok(_) => BTreeMap::new(),
        Err(e) => {
            log::warn!("Ignoring hash cache {}: {e}", path.display());
            BTreeMap::new()
        }
    }
}
fn write_atomic(path: &Path, items: BTreeMap<String, Record>) -> Result<()> {
    let directory = path.parent().context("cache path has no parent")?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(directory)?;
    // NamedTempFile is created with mode 0600.
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    temp.write_all(&serde_json::to_vec(&CacheFile {
        version: VERSION,
        items,
    })?)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
impl HashCache {
    /// Load the cache at `path`, ignoring unreadable contents.
    pub fn open(path: PathBuf) -> Self {
        let records = load(&path);
        Self::with(Some(path), records)
    }
    /// A cache that is never persisted.
    pub fn in_memory() -> Self {
        Self::with(None, BTreeMap::new())
    }
    fn with(path: Option<PathBuf>, records: BTreeMap<String, Record>) -> Self {
        Self {
            path,
            racy_margin: Duration::from_secs(2),
            state: Mutex::new(State {
                records,
                last_save: Some(Instant::now()),
                ..Default::default()
            }),
            writing: Mutex::new(()),
            #[cfg(test)]
            full_hashes: Default::default(),
        }
    }
    /// The process-wide cache at [`cache_path`].
    pub fn shared() -> &'static Self {
        static SHARED: OnceLock<HashCache> = OnceLock::new();
        SHARED.get_or_init(|| Self::open(cache_path()))
    }
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// `safety::content_hash(directory)`, reusing the cached result when the
    /// tree's fingerprint is unchanged since it was hashed.
    pub fn content_hash(&self, id: &str, directory: &Path) -> Result<String> {
        let location = directory
            .canonicalize()
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned));
        let current = snapshot(directory)?;
        let key = fingerprint(directory, &current);
        if let (Some(location), Some(key)) = (&location, &key)
            && let Some(record) = self.state().records.get(id)
            && record.path == *location
            && record.fingerprint == *key
        {
            return Ok(record.content_hash.clone());
        }
        let started = SystemTime::now();
        // The full hash keeps its own TOCTOU checks and fails unless the tree
        // still matches `current` afterwards, so `key` describes what was read.
        let (hash, stable) = hash_tree(directory, current, |_| {})?;
        #[cfg(test)]
        self.full_hashes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let settled = started
            .checked_sub(self.racy_margin)
            .is_some_and(|limit| ctimes_before(&stable, limit));
        if let (Some(path), Some(fingerprint), true) = (location, key, settled) {
            let record = Record {
                path,
                fingerprint,
                content_hash: hash.clone(),
            };
            let mut state = self.state();
            state.records.insert(id.into(), record.clone());
            state.removed.remove(id);
            state.unsaved.insert(id.into(), record);
            let due = state.last_save.is_none_or(|t| t.elapsed() >= SAVE_INTERVAL);
            drop(state);
            if due && let Err(e) = self.flush() {
                log::warn!("Cannot save hash cache: {e:#}");
            }
        }
        Ok(hash)
    }
    /// Forget records whose ID fails `keep`, e.g. uninstalled items.
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        let mut state = self.state();
        let gone: Vec<_> = state
            .records
            .keys()
            .filter(|id| !keep(id))
            .cloned()
            .collect();
        for id in gone {
            state.records.remove(&id);
            state.unsaved.remove(&id);
            state.removed.insert(id);
        }
    }
    /// Merge pending changes into the file on disk, keeping entries another
    /// process wrote meanwhile. Concurrent writers can only drop records,
    /// which costs a rehash. Failure leaves verification unaffected.
    pub fn flush(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let (unsaved, removed) = {
            let mut state = self.state();
            if state.unsaved.is_empty() && state.removed.is_empty() {
                return Ok(());
            }
            state.last_save = Some(Instant::now());
            (state.unsaved.clone(), state.removed.clone())
        };
        let mut items = load(path);
        items.retain(|id, _| !removed.contains(id));
        items.extend(unsaved.clone());
        write_atomic(path, items).with_context(|| format!("Write {}", path.display()))?;
        let mut state = self.state();
        state
            .unsaved
            .retain(|id, record| unsaved.get(id) != Some(record));
        state.removed.retain(|id| !removed.contains(id));
        Ok(())
    }
}
/// Whether every entry's ctime, root included, is before `limit`.
fn ctimes_before(snapshot: &Snapshot, limit: SystemTime) -> bool {
    #[cfg(unix)]
    {
        let Ok(limit) = limit.duration_since(SystemTime::UNIX_EPOCH) else {
            return false;
        };
        let limit = (limit.as_secs() as i64, limit.subsec_nanos() as i64);
        std::iter::once(&snapshot.root)
            .chain(snapshot.entries.iter().map(|e| &e.metadata))
            .all(|m| (m.identity.2, m.identity.3) < limit)
    }
    #[cfg(not(unix))]
    {
        let _ = (snapshot, limit);
        false
    }
}
fn nanos(time: SystemTime) -> i128 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => after.as_nanos() as i128,
        Err(before) => -(before.duration().as_nanos() as i128),
    }
}
/// Digest of the tree's lstat fingerprint, or None when it cannot prove the
/// content unchanged: no ctime (non-Unix), an untrusted filesystem, or an
/// entry on another device.
fn fingerprint(directory: &Path, snapshot: &Snapshot) -> Option<String> {
    #[cfg(unix)]
    {
        use sha2::{Digest, Sha256};
        let device = snapshot.root.identity.0;
        if !trusted_filesystem(directory, snapshot)
            || snapshot
                .entries
                .iter()
                .any(|e| e.metadata.identity.0 != device)
        {
            return None;
        }
        let mut hash = Sha256::new();
        hash.update(b"wpe-gallery-fingerprint-v1\0");
        let entries = std::iter::once(("", &snapshot.root)).chain(
            snapshot
                .entries
                .iter()
                .map(|e| (e.name.as_str(), &e.metadata)),
        );
        for (name, m) in entries {
            hash.update(if m.directory { b"D" } else { b"F" });
            hash.update((name.len() as u64).to_be_bytes());
            hash.update(name.as_bytes());
            hash.update(m.size.to_be_bytes());
            hash.update(nanos(m.modified).to_be_bytes());
            let (dev, ino, ctime, ctime_nsec) = m.identity;
            hash.update(dev.to_be_bytes());
            hash.update(ino.to_be_bytes());
            hash.update(ctime.to_be_bytes());
            hash.update(ctime_nsec.to_be_bytes());
        }
        Some(format!("{:x}", hash.finalize()))
    }
    #[cfg(not(unix))]
    {
        let _ = (directory, snapshot);
        None
    }
}
/// Whether the item root (the same inode the snapshot saw) lives on a
/// filesystem where only the kernel sets ctime.
#[cfg(unix)]
fn trusted_filesystem(directory: &Path, snapshot: &Snapshot) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::{fs::MetadataExt, fs::OpenOptionsExt, io::AsRawFd};
        const TRUSTED: [u32; 6] = [
            0xEF53,     // ext2/3/4
            0x9123683E, // btrfs
            0x58465342, // xfs
            0xF2F52010, // f2fs
            0x01021994, // tmpfs
            0xCA451A4E, // bcachefs
        ];
        let Ok(root) = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(directory)
        else {
            return false;
        };
        let Ok(metadata) = root.metadata() else {
            return false;
        };
        if (metadata.dev(), metadata.ino()) != (snapshot.root.identity.0, snapshot.root.identity.1)
        {
            return false;
        }
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: `root` is an open descriptor and `stat` is a valid buffer.
        if unsafe { libc::fstatfs(root.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            return false;
        }
        // SAFETY: fstatfs succeeded and initialized `stat`.
        let kind = unsafe { stat.assume_init() }.f_type as u32;
        TRUSTED.contains(&kind)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (directory, snapshot);
        false
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    fn settled(path: Option<PathBuf>) -> HashCache {
        let mut cache = match path {
            Some(path) => HashCache::open(path),
            None => HashCache::in_memory(),
        };
        cache.racy_margin = Duration::ZERO;
        cache
    }
    fn full_hashes(cache: &HashCache) -> usize {
        cache.full_hashes.load(Ordering::Relaxed)
    }
    /// Let the coarse ctime clock move past the last change.
    fn tick() {
        std::thread::sleep(Duration::from_millis(30));
    }
    /// A settled item, or None where the temp directory's filesystem is not
    /// trusted (the cache then always rehashes, which other tests cover).
    fn item() -> Option<tempfile::TempDir> {
        let d = tempfile::tempdir().unwrap();
        if fingerprint(d.path(), &snapshot(d.path()).unwrap()).is_none() {
            eprintln!("skipping: temp directory filesystem not trusted for ctime");
            return None;
        }
        fs::create_dir(d.path().join("sub")).unwrap();
        fs::write(d.path().join("sub/a"), b"one").unwrap();
        fs::write(d.path().join("b"), b"two").unwrap();
        tick();
        Some(d)
    }
    #[test]
    fn hit_avoids_rehash_and_persists() {
        let Some(d) = item() else { return };
        let file = tempfile::tempdir().unwrap();
        let path = file.path().join("nested/hashes.v1");
        let cache = settled(Some(path.clone()));
        let expected = crate::safety::content_hash(d.path()).unwrap();
        assert_eq!(cache.content_hash("1", d.path()).unwrap(), expected);
        assert_eq!(cache.content_hash("1", d.path()).unwrap(), expected);
        assert_eq!(full_hashes(&cache), 1);
        // Another ID or another path for the same tree is a miss.
        cache.content_hash("2", d.path()).unwrap();
        assert_eq!(full_hashes(&cache), 2);
        cache.flush().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let reopened = settled(Some(path));
        assert_eq!(reopened.content_hash("1", d.path()).unwrap(), expected);
        assert_eq!(full_hashes(&reopened), 0);
    }
    #[test]
    fn retained_ids_survive_flush_and_others_are_dropped() {
        let Some(d) = item() else { return };
        let file = tempfile::tempdir().unwrap();
        let path = file.path().join("hashes.v1");
        let cache = settled(Some(path.clone()));
        cache.content_hash("1", d.path()).unwrap();
        cache.content_hash("2", d.path()).unwrap();
        cache.flush().unwrap();
        cache.retain(|id| id == "1");
        cache.flush().unwrap();
        assert_eq!(load(&path).keys().collect::<Vec<_>>(), ["1"]);
    }
    #[test]
    fn same_size_and_restored_mtime_is_detected_by_ctime() {
        let Some(d) = item() else { return };
        let cache = settled(None);
        let file = d.path().join("sub/a");
        let mtime = fs::metadata(&file).unwrap().modified().unwrap();
        let original = cache.content_hash("1", d.path()).unwrap();
        tick();
        fs::write(&file, b"eno").unwrap();
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), mtime);
        let changed = cache.content_hash("1", d.path()).unwrap();
        assert_ne!(changed, original);
        assert_eq!(changed, crate::safety::content_hash(d.path()).unwrap());
        assert_eq!(full_hashes(&cache), 2);
    }
    #[test]
    fn added_or_removed_entries_invalidate() {
        let Some(d) = item() else { return };
        let cache = settled(None);
        let original = cache.content_hash("1", d.path()).unwrap();
        tick();
        fs::write(d.path().join("sub/new"), b"").unwrap();
        assert_ne!(cache.content_hash("1", d.path()).unwrap(), original);
        tick();
        fs::remove_file(d.path().join("sub/new")).unwrap();
        fs::remove_file(d.path().join("b")).unwrap();
        let removed = cache.content_hash("1", d.path()).unwrap();
        assert_ne!(removed, original);
        assert_eq!(removed, crate::safety::content_hash(d.path()).unwrap());
        assert_eq!(full_hashes(&cache), 3);
    }
    #[test]
    fn untrusted_or_mixed_trees_are_never_cached() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a"), b"one").unwrap();
        let mut tree = snapshot(d.path()).unwrap();
        #[cfg(unix)]
        {
            tree.entries[0].metadata.identity.0 ^= 1;
        }
        assert_eq!(fingerprint(d.path(), &tree), None);
    }
    #[test]
    fn recent_changes_are_not_cached() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a"), b"fresh").unwrap();
        let cache = HashCache::in_memory();
        cache.content_hash("1", d.path()).unwrap();
        cache.content_hash("1", d.path()).unwrap();
        assert_eq!(full_hashes(&cache), 2);
    }
    #[test]
    fn corrupt_or_old_cache_is_ignored() {
        let Some(d) = item() else { return };
        let file = tempfile::tempdir().unwrap();
        let path = file.path().join("hashes.v1");
        let expected = crate::safety::content_hash(d.path()).unwrap();
        for contents in [&b"\0garbage{"[..], br#"{"version":0,"items":{}}"#] {
            fs::write(&path, contents).unwrap();
            let cache = settled(Some(path.clone()));
            assert_eq!(cache.content_hash("1", d.path()).unwrap(), expected);
            assert_eq!(full_hashes(&cache), 1);
            cache.flush().unwrap();
        }
        // A forged record for the right ID and path but a stale fingerprint
        // is ignored.
        let mut items = load(&path);
        let record = items.get_mut("1").unwrap();
        record.content_hash = "0".repeat(64);
        record.fingerprint = "f".repeat(64);
        write_atomic(&path, items).unwrap();
        let cache = settled(Some(path));
        assert_eq!(cache.content_hash("1", d.path()).unwrap(), expected);
    }
    #[test]
    fn unwritable_cache_still_hashes() {
        let Some(d) = item() else { return };
        let file = tempfile::NamedTempFile::new().unwrap();
        // A regular file where the cache directory should be.
        let cache = settled(Some(file.path().join("hashes.v1")));
        let expected = crate::safety::content_hash(d.path()).unwrap();
        assert_eq!(cache.content_hash("1", d.path()).unwrap(), expected);
        assert!(cache.flush().is_err());
        assert_eq!(cache.content_hash("1", d.path()).unwrap(), expected);
    }
}
