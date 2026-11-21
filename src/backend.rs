use anyhow::{Context, Result};
use serde::Serialize;
#[derive(Clone, Debug, Default, Serialize)]
pub struct Output {
    /// Connector name (`DP-3`); compositors may renumber it across boots and driver updates.
    pub name: String,
    /// Stable identity, `make model serial` as niri names outputs; None when the display reports
    /// no make and model or another connected display shares it.
    pub identity: Option<String>,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub width: i32,
    pub height: i32,
}
impl Output {
    /// Short display name for the UI: the model, or the connector when the display reports none
    /// (wlroots compositors send the placeholder "Unknown").
    pub fn label(&self) -> &str {
        Some(self.model.trim())
            .filter(|m| !m.is_empty() && *m != "Unknown")
            .unwrap_or(&self.name)
    }
}
/// Fills each output's identity; duplicates (two identical displays without serials) get none,
/// so an identity never names two outputs.
fn assign_identities(outputs: &mut [Output]) {
    // niri and wlroots report a missing make or model as "Unknown".
    let missing = |s: &str| s.trim().is_empty() || s.trim() == "Unknown";
    for o in outputs.iter_mut() {
        o.identity = (!(missing(&o.make) && missing(&o.model))).then(|| {
            [&o.make, &o.model, &o.serial]
                .into_iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        });
    }
    let identities: Vec<_> = outputs.iter().map(|o| o.identity.clone()).collect();
    for o in outputs.iter_mut() {
        if identities.iter().filter(|i| **i == o.identity).count() > 1 {
            o.identity = None;
        }
    }
}
/// niri's view of the outputs: connector, make, model, serial and the current mode.
fn niri_outputs() -> Option<Vec<Output>> {
    let result = std::process::Command::new("niri")
        .args(["msg", "--json", "outputs"])
        .output()
        .ok()?;
    let Ok(serde_json::Value::Object(values)) = serde_json::from_slice(&result.stdout) else {
        return None;
    };
    let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_owned();
    let outputs = values
        .into_iter()
        .map(|(name, v)| {
            let mode = v["current_mode"].as_u64().map(|i| &v["modes"][i as usize]);
            Output {
                name,
                make: text(&v["make"]),
                model: text(&v["model"]),
                // niri names a display without a serial "make model Unknown".
                serial: v["serial"].as_str().unwrap_or("Unknown").to_owned(),
                width: mode.map_or(0, |m| m["width"].as_i64().unwrap_or(0) as i32),
                height: mode.map_or(0, |m| m["height"].as_i64().unwrap_or(0) as i32),
                ..Default::default()
            }
        })
        .collect();
    Some(outputs)
}
pub trait ApplyBackend {
    fn apply(&self, output: &Output, wallpaper: &str) -> Result<()>;
}
pub struct Platform;
impl ApplyBackend for Platform {
    fn apply(&self, output: &Output, wallpaper: &str) -> Result<()> {
        anyhow::ensure!(
            cfg!(target_os = "linux"),
            "Apply is currently supported on Linux only"
        );
        crate::safety::verify(wallpaper)?;
        crate::library::save_assignment(&crate::library::config_path(), output, wallpaper)?;
        // Reload swaps renderers without a bare-background gap; units without
        // ExecReload, or not running, are restarted instead.
        let result = std::process::Command::new("systemctl")
            .args([
                "--user",
                "reload-or-restart",
                "linux-wallpaperengine.service",
            ])
            .output()
            .context("start systemctl")?;
        anyhow::ensure!(
            result.status.success(),
            "Assignment saved, but service reload failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(())
    }
}
pub fn outputs() -> Vec<Output> {
    discover().unwrap_or_default()
}
/// The enabled outputs, or None when neither Wayland nor niri could be queried.
pub fn discover() -> Option<Vec<Output>> {
    let niri = niri_outputs();
    #[cfg(target_os = "linux")]
    let wayland = wayland::discover().ok();
    #[cfg(not(target_os = "linux"))]
    let wayland: Option<Vec<Output>> = None;
    if niri.is_none() && wayland.is_none() {
        return None;
    }
    let (niri, wayland) = (niri.unwrap_or_default(), wayland.unwrap_or_default());
    let mut outputs = if wayland.is_empty() {
        niri.clone()
    } else {
        // wl_output carries no serial; take it from niri when niri runs the session.
        wayland
            .into_iter()
            .map(|mut o| {
                if let Some(n) = niri.iter().find(|n| n.name == o.name) {
                    o.serial = n.serial.clone();
                }
                o
            })
            .collect::<Vec<_>>()
    };
    // Disabled niri outputs count for duplicates, so enabling the other of two identical
    // displays cannot hand it this one's identity.
    let disabled = niri
        .iter()
        .filter(|n| !outputs.iter().any(|o| o.name == n.name))
        .map(|n| Output {
            width: 0,
            ..n.clone()
        })
        .collect::<Vec<_>>();
    outputs.extend(disabled);
    assign_identities(&mut outputs);
    outputs.retain(|o| o.width > 0);
    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    Some(outputs)
}
#[cfg(target_os = "linux")]
mod wayland {
    use super::*;
    use std::collections::BTreeMap;
    use wayland_client::{
        Connection, Dispatch, QueueHandle,
        protocol::{wl_output, wl_registry},
    };
    #[derive(Default)]
    struct State {
        outputs: BTreeMap<u32, Output>,
    }
    impl Dispatch<wl_registry::WlRegistry, ()> for State {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            q: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
                && interface == "wl_output"
                && version >= 4
            {
                state.outputs.insert(name, Output::default());
                registry.bind::<wl_output::WlOutput, _, _>(name, 4, q, name);
            }
        }
    }
    impl Dispatch<wl_output::WlOutput, u32> for State {
        fn event(
            state: &mut Self,
            _: &wl_output::WlOutput,
            event: wl_output::Event,
            id: &u32,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let out = state.outputs.get_mut(id).unwrap();
            match event {
                wl_output::Event::Name { name } => out.name = name,
                wl_output::Event::Geometry { make, model, .. } => {
                    out.make = make;
                    out.model = model;
                }
                wl_output::Event::Mode {
                    flags,
                    width,
                    height,
                    ..
                } if flags
                    .into_result()
                    .is_ok_and(|f| f.contains(wl_output::Mode::Current)) =>
                {
                    out.width = width;
                    out.height = height;
                }
                _ => {}
            }
        }
    }
    pub fn discover() -> Result<Vec<Output>> {
        let conn = Connection::connect_to_env()?;
        let mut queue = conn.new_event_queue();
        conn.display().get_registry(&queue.handle(), ());
        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;
        Ok(state
            .outputs
            .into_values()
            .filter(|o| !o.name.is_empty() && o.width > 0)
            .collect())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn output(name: &str, make: &str, model: &str, serial: &str) -> Output {
        Output {
            name: name.into(),
            make: make.into(),
            model: model.into(),
            serial: serial.into(),
            ..Default::default()
        }
    }
    #[test]
    fn identities_follow_the_display_not_the_connector() {
        let mut outputs = vec![
            output("DP-6", "Acme Corporation", "Vista 34", "AC1234567"),
            output("DP-4", "", "", ""),
            output("DP-5", "Unknown", "Unknown", "Unknown"),
            output("HDMI-A-1", "Dell", "U2720Q", "Unknown"),
            output("HDMI-A-2", "Dell", "U2720Q", "Unknown"),
        ];
        assign_identities(&mut outputs);
        let identities: Vec<_> = outputs.iter().map(|o| o.identity.as_deref()).collect();
        assert_eq!(
            identities,
            [
                Some("Acme Corporation Vista 34 AC1234567"),
                None,
                None,
                None,
                None
            ]
        );
        assert_eq!(outputs[0].label(), "Vista 34");
        assert_eq!(
            output("HEADLESS-1", "headless", "Unknown", "").label(),
            "HEADLESS-1"
        );
    }
}
