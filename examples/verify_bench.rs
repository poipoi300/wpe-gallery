//! Time one cached content-hash pass over a Workshop content directory.
//!
//! `cargo run --release --example verify_bench -- CONTENT_DIR CACHE_FILE`
//! Run twice with the same cache file for cold and warm timings. Nothing is
//! authorized or rendered; the whitelist is not read.
use std::{path::PathBuf, time::Instant};
use wpe_gallery::safety::{self, HashCache};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let (Some(content), Some(cache_file)) = (args.next(), args.next()) else {
        anyhow::bail!("usage: verify_bench CONTENT_DIR CACHE_FILE");
    };
    let mut items: Vec<_> = std::fs::read_dir(&content)?
        .filter_map(|e| {
            let e = e.ok()?;
            let id = e.file_name().into_string().ok()?;
            safety::valid_id(&id).then(|| (id, e.path()))
        })
        .collect();
    items.sort();
    let cache = HashCache::open(cache_file);
    let started = Instant::now();
    let mut failures = 0;
    safety::parallel_map(
        &items,
        |(id, path)| cache.content_hash(id, path),
        |_, result| {
            failures += usize::from(result.is_err());
            true
        },
    );
    let elapsed = started.elapsed();
    cache.flush()?;
    println!(
        "{} items, {failures} failed, {:.2} s",
        items.len(),
        elapsed.as_secs_f64()
    );
    Ok(())
}
