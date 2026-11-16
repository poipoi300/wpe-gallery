use crate::{
    library::{self, Assignment, Config},
    safety,
};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn previous_path() -> PathBuf {
    library::config_path().with_file_name("last-safe-assignments.json")
}
fn legacy() -> Result<Config> {
    let output = std::env::var("WPE_OUTPUT")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| crate::backend::outputs().first().map(|o| o.name.clone()))
        .context("no enabled output")?;
    Ok(Config {
        version: 1,
        outputs: BTreeMap::from([(
            output,
            Assignment {
                wallpaper_id: std::env::var("WPE_BG").unwrap_or_default(),
                ..Default::default()
            },
        )]),
    })
}
fn choose<F>(
    requested: &Config,
    previous: &Config,
    mut verify: F,
) -> (Config, BTreeMap<String, PathBuf>)
where
    F: FnMut(&str) -> Result<PathBuf>,
{
    let mut selected = Config::default();
    let mut paths = BTreeMap::new();
    for (output, wanted) in &requested.outputs {
        if output.is_empty() || output.starts_with('-') {
            eprintln!("wallpaperengine-start: invalid output name");
            continue;
        }
        for (index, assignment) in [Some(wanted), previous.outputs.get(output)]
            .into_iter()
            .flatten()
            .enumerate()
        {
            if !assignment.properties.is_empty() {
                eprintln!("wallpaperengine-start: {output}: property overrides not supported");
                continue;
            }
            match verify(&assignment.wallpaper_id) {
                Ok(path) => {
                    if index > 0 {
                        eprintln!(
                            "wallpaperengine-start: {output}: falling back to last safe assignment {}",
                            assignment.wallpaper_id
                        );
                    }
                    selected.outputs.insert(output.clone(), assignment.clone());
                    paths.insert(output.clone(), path);
                    break;
                }
                Err(e) => eprintln!(
                    "wallpaperengine-start: {output}: rejected {}: {e:#}",
                    assignment.wallpaper_id
                ),
            }
        }
    }
    (selected, paths)
}
pub fn arguments(record: bool) -> Result<Vec<String>> {
    let previous = library::load_config(&previous_path()).unwrap_or_else(|e| {
        eprintln!("wallpaperengine-start: cannot load last-safe assignments: {e:#}");
        Config::default()
    });
    let requested = if library::config_path().exists() {
        library::load_config(&library::config_path())
    } else {
        legacy()
    };
    let requested = requested.unwrap_or_else(|e| {
        eprintln!(
            "wallpaperengine-start: cannot load requested assignments: {e:#}; trying last safe"
        );
        previous.clone()
    });
    let (selected, paths) = choose(&requested, &previous, safety::verify);
    ensure!(
        !selected.outputs.is_empty(),
        "no reviewed-safe, hash-matching assignment; renderer not started"
    );
    let fps = std::env::var("WPE_FPS").unwrap_or("30".into());
    ensure!(
        fps.parse::<u32>().is_ok_and(|n| (1..=1000).contains(&n)),
        "WPE_FPS must be 1..1000"
    );
    let mut args = vec![
        "/usr/bin/linux-wallpaperengine".into(),
        "--silent".into(),
        "--fps".into(),
        fps,
        "--fullscreen-pause-only-active".into(),
        "--scaling".into(),
        "fill".into(),
    ];
    for (output, path) in paths {
        args.extend([
            "--screen-root".into(),
            output,
            "--bg".into(),
            path.to_string_lossy().into_owned(),
            "--scaling".into(),
            "fill".into(),
        ]);
    }
    if record {
        let path = previous_path();
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temp, &selected)?;
        temp.as_file().sync_all()?;
        temp.persist(path)?;
    }
    Ok(args)
}
pub fn launch() -> Result<()> {
    let args = arguments(true)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(std::process::Command::new(&args[0])
            .args(&args[1..])
            .exec()
            .into())
    }
    #[cfg(not(unix))]
    {
        anyhow::bail!("Linux renderer launch is unsupported on this platform")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config(id: &str) -> Config {
        Config {
            version: 1,
            outputs: BTreeMap::from([(
                "TEST-1".into(),
                Assignment {
                    wallpaper_id: id.into(),
                    ..Default::default()
                },
            )]),
        }
    }
    #[test]
    fn revalidates_fallback_and_stops_if_both_unsafe() {
        let previous = config("123");
        let requested = config("456");
        let (selected, _) = choose(&requested, &previous, |id| {
            ensure!(id == "123", "unreviewed");
            Ok(PathBuf::from("/fixture/123"))
        });
        assert_eq!(selected.outputs["TEST-1"].wallpaper_id, "123");
        let (selected, _) = choose(&requested, &previous, |_| {
            anyhow::bail!("changed after review")
        });
        assert!(selected.outputs.is_empty());
    }
}
