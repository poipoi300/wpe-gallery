use anyhow::{Context, Result};
use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct Output {
    pub name: String,
    pub width: i32,
    pub height: i32,
}
pub trait ApplyBackend {
    fn apply(&self, output: &str, wallpaper: &str) -> Result<()>;
}
pub struct Platform;
impl ApplyBackend for Platform {
    fn apply(&self, output: &str, wallpaper: &str) -> Result<()> {
        anyhow::ensure!(
            cfg!(target_os = "linux"),
            "Apply is currently supported on Linux only"
        );
        crate::library::save_assignment(&crate::library::config_path(), output, wallpaper)?;
        let result = std::process::Command::new("systemctl")
            .args(["--user", "restart", "linux-wallpaperengine.service"])
            .output()
            .context("start systemctl")?;
        anyhow::ensure!(
            result.status.success(),
            "Assignment saved, but service restart failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(())
    }
}
pub fn outputs() -> Vec<Output> {
    #[cfg(target_os = "linux")]
    if let Ok(outputs) = wayland::discover()
        && !outputs.is_empty()
    {
        return outputs;
    }
    let mut outputs = Vec::new();
    if let Ok(result) = std::process::Command::new("niri")
        .args(["msg", "--json", "outputs"])
        .output()
        && let Ok(serde_json::Value::Object(values)) = serde_json::from_slice(&result.stdout)
    {
        for (name, v) in values {
            if let Some(i) = v["current_mode"].as_u64() {
                let mode = &v["modes"][i as usize];
                outputs.push(Output {
                    name,
                    width: mode["width"].as_i64().unwrap_or(0) as i32,
                    height: mode["height"].as_i64().unwrap_or(0) as i32,
                });
            }
        }
    }
    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    outputs
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
                state.outputs.insert(
                    name,
                    Output {
                        name: String::new(),
                        width: 0,
                        height: 0,
                    },
                );
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
        let mut outputs: Vec<_> = state
            .outputs
            .into_values()
            .filter(|o| !o.name.is_empty() && o.width > 0)
            .collect();
        outputs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(outputs)
    }
}
