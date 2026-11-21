use crate::{
    backend::Output,
    library::{self, Assignment, Config},
    safety,
};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, fs, path::PathBuf};
#[cfg(target_os = "linux")]
mod supervisor;

fn previous_path() -> PathBuf {
    library::config_path().with_file_name("last-safe-assignments.json")
}
/// The output WPE_OUTPUT names: a display identity before a connector name, since a virtual
/// output may be named like a display; the first enabled output when unset.
fn legacy_output(outputs: &[Output], wanted: Option<String>) -> Option<Output> {
    let Some(name) = wanted else {
        return outputs.first().cloned();
    };
    let found = outputs
        .iter()
        .find(|o| o.identity.as_deref() == Some(&name))
        .or_else(|| outputs.iter().find(|o| o.name == name));
    Some(found.cloned().unwrap_or(Output {
        name,
        ..Default::default()
    }))
}
fn legacy(outputs: &[Output]) -> Result<Config> {
    let wanted = std::env::var("WPE_OUTPUT").ok().filter(|s| !s.is_empty());
    let output = legacy_output(outputs, wanted).context("no enabled output")?;
    let mut config = Config::default();
    config.insert(
        &output,
        Assignment {
            wallpaper_id: std::env::var("WPE_BG").unwrap_or_default(),
            ..Default::default()
        },
    );
    Ok(config)
}
/// Picks a verified wallpaper per enabled output and the connector each renders on.
///
/// Each output takes its identity entry, else its connector entry; when that wallpaper fails
/// verification only its last-safe entries (identity, then connector) are tried. The paths are
/// keyed by connector.
fn choose<F>(
    requested: &Config,
    previous: &Config,
    outputs: &[Output],
    mut verify: F,
) -> (Config, BTreeMap<String, PathBuf>)
where
    F: FnMut(&str) -> Result<PathBuf>,
{
    let mut selected = Config::default();
    let mut paths = BTreeMap::new();
    let present = |key: &String| outputs.iter().any(|o| o.identity.as_ref() == Some(key));
    for key in requested.displays.keys().filter(|k| !present(k)) {
        eprintln!("wallpaperengine-start: {key}: not connected");
    }
    for key in requested.outputs.keys() {
        if !outputs.iter().any(|o| &o.name == key) {
            eprintln!("wallpaperengine-start: {key}: not connected");
        }
    }
    for output in outputs {
        let Some(wanted) = requested.get(output) else {
            continue;
        };
        let connector = &output.name;
        if connector.is_empty() || connector.starts_with('-') {
            eprintln!("wallpaperengine-start: invalid output name");
            continue;
        }
        let by_identity = output
            .identity
            .as_ref()
            .and_then(|i| previous.displays.get(i));
        let fallbacks = [by_identity, previous.outputs.get(connector)];
        for (index, assignment) in std::iter::once(wanted)
            .chain(fallbacks.into_iter().flatten())
            .enumerate()
        {
            if !assignment.properties.is_empty() {
                eprintln!("wallpaperengine-start: {connector}: property overrides not supported");
                continue;
            }
            match verify(&assignment.wallpaper_id) {
                Ok(path) => {
                    if index > 0 {
                        eprintln!(
                            "wallpaperengine-start: {connector}: falling back to last safe assignment {}",
                            assignment.wallpaper_id
                        );
                    }
                    selected.insert(output, assignment.clone());
                    paths.insert(connector.clone(), path);
                    break;
                }
                Err(e) => eprintln!(
                    "wallpaperengine-start: {connector}: rejected {}: {e:#}",
                    assignment.wallpaper_id
                ),
            }
        }
    }
    (selected, paths)
}
/// Keeps the last-safe entries of displays and connectors absent this launch.
fn keep_absent(selected: &mut Config, previous: &Config, outputs: &[Output]) {
    for (key, assignment) in &previous.displays {
        if !outputs.iter().any(|o| o.identity.as_ref() == Some(key)) {
            selected
                .displays
                .entry(key.clone())
                .or_insert_with(|| assignment.clone());
        }
    }
    for (key, assignment) in &previous.outputs {
        if !outputs.iter().any(|o| &o.name == key) {
            selected
                .outputs
                .entry(key.clone())
                .or_insert_with(|| assignment.clone());
        }
    }
}
pub fn arguments(record: bool) -> Result<Vec<String>> {
    let previous = library::load_config(&previous_path()).unwrap_or_else(|e| {
        eprintln!("wallpaperengine-start: cannot load last-safe assignments: {e:#}");
        Config::default()
    });
    let discovered = crate::backend::discover();
    let requested = if library::config_path().exists() {
        library::load_config(&library::config_path())
    } else {
        legacy(discovered.as_deref().unwrap_or_default())
    };
    let requested = requested.unwrap_or_else(|e| {
        eprintln!(
            "wallpaperengine-start: cannot load requested assignments: {e:#}; trying last safe"
        );
        previous.clone()
    });
    let outputs = discovered.unwrap_or_else(|| {
        // Without discovery (no compositor reachable) only connector entries can be placed,
        // as before identities existed; the renderer itself rejects unknown connectors.
        eprintln!(
            "wallpaperengine-start: cannot list outputs; using assignment keys as connectors"
        );
        requested
            .outputs
            .keys()
            .map(|key| Output {
                name: key.clone(),
                ..Default::default()
            })
            .collect()
    });
    ensure!(!outputs.is_empty(), "no enabled output");
    let (mut selected, paths) = choose(&requested, &previous, &outputs, safety::verify);
    ensure!(
        !selected.is_empty(),
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
        keep_absent(&mut selected, &previous, &outputs);
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
/// Runs the renderer under the make-before-break supervisor (see `supervisor`).
pub fn launch() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        supervisor::launch()
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Linux renderer launch is unsupported on this platform")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str) -> Assignment {
        Assignment {
            wallpaper_id: id.into(),
            ..Default::default()
        }
    }
    fn by_connector(key: &str, id: &str) -> Config {
        Config {
            outputs: BTreeMap::from([(key.into(), entry(id))]),
            ..Default::default()
        }
    }
    fn by_display(key: &str, id: &str) -> Config {
        Config {
            displays: BTreeMap::from([(key.into(), entry(id))]),
            ..Default::default()
        }
    }
    fn output(name: &str, identity: Option<&str>) -> Output {
        Output {
            name: name.into(),
            identity: identity.map(Into::into),
            ..Default::default()
        }
    }
    fn at(id: &str) -> Result<PathBuf> {
        Ok(PathBuf::from(format!("/fixture/{id}")))
    }
    const DISPLAY: &str = "Acme Vista AC1";
    #[test]
    fn revalidates_fallback_and_stops_if_both_unsafe() {
        let outputs = [output("TEST-1", None)];
        let (requested, previous) = (by_connector("TEST-1", "456"), by_connector("TEST-1", "123"));
        let (selected, _) = choose(&requested, &previous, &outputs, |id| {
            ensure!(id == "123", "unreviewed");
            at(id)
        });
        assert_eq!(selected.outputs["TEST-1"].wallpaper_id, "123");
        let (selected, _) = choose(&requested, &previous, &outputs, |_| {
            anyhow::bail!("changed after review")
        });
        assert!(selected.is_empty());
    }
    #[test]
    fn display_entries_follow_the_display_to_its_new_connector() {
        let outputs = [output("DP-6", Some(DISPLAY))];
        let (selected, paths) = choose(
            &by_display(DISPLAY, "123"),
            &Config::default(),
            &outputs,
            at,
        );
        assert_eq!(paths["DP-6"], PathBuf::from("/fixture/123"));
        assert_eq!(selected.displays.keys().collect::<Vec<_>>(), [DISPLAY]);
    }
    #[test]
    fn display_entry_wins_over_a_stale_connector_entry() {
        let outputs = [output("DP-6", Some(DISPLAY))];
        let mut requested = by_connector("DP-6", "111");
        requested.displays = by_display(DISPLAY, "222").displays;
        requested
            .outputs
            .extend(by_connector("DP-3", "333").outputs);
        let (_, paths) = choose(&requested, &Config::default(), &outputs, at);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths["DP-6"], PathBuf::from("/fixture/222"));
        // A rejected display entry falls back to last safe, never to the stale connector entry.
        let (selected, paths) = choose(&requested, &Config::default(), &outputs, |id| {
            ensure!(id != "222", "unreviewed");
            at(id)
        });
        assert!(paths.is_empty() && selected.is_empty());
    }
    #[test]
    fn a_rejected_display_backup_still_tries_the_connector_backup() {
        let outputs = [output("DP-6", Some(DISPLAY))];
        let mut previous = by_display(DISPLAY, "789");
        previous.outputs = by_connector("DP-6", "123").outputs;
        let (selected, _) = choose(&by_display(DISPLAY, "456"), &previous, &outputs, |id| {
            ensure!(id == "123", "unreviewed");
            at(id)
        });
        assert_eq!(selected.displays[DISPLAY].wallpaper_id, "123");
    }
    #[test]
    fn a_connector_named_like_a_display_never_takes_its_entry() {
        // A niri virtual output may be named anything; without an identity it only matches
        // connector entries.
        let outputs = [output(DISPLAY, None)];
        let (selected, paths) = choose(
            &by_display(DISPLAY, "123"),
            &Config::default(),
            &outputs,
            at,
        );
        assert!(paths.is_empty() && selected.is_empty());
    }
    #[test]
    fn recording_keeps_absent_displays_backups() {
        let outputs = [output("DP-6", Some(DISPLAY))];
        let mut selected = by_display(DISPLAY, "123");
        let mut previous = by_display(DISPLAY, "999");
        previous
            .displays
            .extend(by_display("Dell U2720Q X", "456").displays);
        previous.outputs = by_connector("DP-3", "789").outputs;
        keep_absent(&mut selected, &previous, &outputs);
        assert_eq!(selected.displays[DISPLAY].wallpaper_id, "123");
        assert_eq!(selected.displays["Dell U2720Q X"].wallpaper_id, "456");
        assert_eq!(selected.outputs["DP-3"].wallpaper_id, "789");
    }
    #[test]
    fn wpe_output_prefers_a_display_identity_over_a_connector_name() {
        let outputs = [output(DISPLAY, None), output("DP-6", Some(DISPLAY))];
        let pick = |name: &str| legacy_output(&outputs, Some(name.into())).unwrap().name;
        assert_eq!(pick(DISPLAY), "DP-6");
        assert_eq!(pick("DP-6"), "DP-6");
        assert_eq!(legacy_output(&outputs, None).unwrap().name, DISPLAY);
    }
}
