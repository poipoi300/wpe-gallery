use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize)]
pub struct Wallpaper {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub description: String,
    pub properties: serde_json::Value,
    pub preview: Option<PathBuf>,
}
pub fn libraries() -> Vec<PathBuf> {
    let home = directories::BaseDirs::new()
        .expect("home directory")
        .home_dir()
        .to_path_buf();
    let mut roots = vec![
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join("Library/Application Support/Steam"),
    ];
    if let Some(p) = std::env::var_os("PROGRAMFILES(X86)") {
        roots.push(PathBuf::from(p).join("Steam"));
    }
    let mut all = roots.clone();
    for root in roots {
        if let Ok(text) = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            all.extend(vdf_paths(&text).into_iter().map(PathBuf::from));
        }
    }
    let mut unique = BTreeMap::new();
    for root in all {
        if let Ok(path) = root.canonicalize() {
            unique.insert(path.clone(), path);
        }
    }
    unique.into_values().collect()
}
// Steam's quoted VDF tokens: braces and // comments are structural, escapes are retained.
pub fn vdf_paths(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
        } else if c == '"' {
            let mut value = String::new();
            while let Some(c) = chars.next() {
                if c == '"' {
                    break;
                }
                if c == '\\' {
                    if let Some(c) = chars.next() {
                        value.push(c);
                    }
                } else {
                    value.push(c);
                }
            }
            tokens.push(value);
        }
    }
    tokens
        .windows(2)
        .filter(|t| t[0] == "path")
        .map(|t| t[1].clone())
        .collect()
}
pub fn scan() -> (Vec<Wallpaper>, Vec<String>) {
    let mut items = BTreeMap::new();
    let mut warnings = Vec::new();
    for root in libraries() {
        let folder = root.join("steamapps/workshop/content/431960");
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            match read(&path) {
                Ok(item) => {
                    items.entry(item.id.clone()).or_insert(item);
                }
                Err(e) => warnings.push(format!("{}: {e}", path.display())),
            }
        }
    }
    let mut items: Vec<_> = items.into_values().collect();
    items.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    (items, warnings)
}
fn read(path: &Path) -> Result<Wallpaper> {
    let id = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    anyhow::ensure!(
        !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
        "invalid workshop ID"
    );
    let v: serde_json::Value = serde_json::from_slice(&fs::read(path.join("project.json"))?)?;
    let preview = v["preview"]
        .as_str()
        .and_then(|p| {
            let p = path.join(p).canonicalize().ok()?;
            p.starts_with(path.canonicalize().ok()?).then_some(p)
        })
        .or_else(|| {
            ["preview.gif", "preview.jpg", "preview.png"]
                .iter()
                .map(|p| path.join(p))
                .find(|p| p.is_file())
        });
    Ok(Wallpaper {
        title: v["title"].as_str().unwrap_or(&id).into(),
        id,
        kind: v["type"].as_str().unwrap_or("unknown").into(),
        tags: v["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        description: v["description"].as_str().unwrap_or("").into(),
        properties: v["general"]["properties"].clone(),
        preview,
    })
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub wallpaper_id: String,
    #[serde(default)]
    pub properties: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub outputs: BTreeMap<String, Assignment>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            outputs: BTreeMap::new(),
        }
    }
}
pub fn config_path() -> PathBuf {
    directories::BaseDirs::new()
        .expect("home directory")
        .config_dir()
        .join("linux-wallpaperengine/assignments.json")
}
pub fn load_config(path: &Path) -> Result<Config> {
    let c: Config = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(
        c.version == 1,
        "unsupported assignments version {}",
        c.version
    );
    Ok(c)
}
pub fn save_assignment(path: &Path, output: &str, id: &str) -> Result<()> {
    anyhow::ensure!(
        !output.is_empty() && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
        "invalid assignment"
    );
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    let mut config = load_config(path)?;
    let old = config
        .outputs
        .remove(output)
        .filter(|a| a.wallpaper_id == id);
    config.outputs.insert(
        output.into(),
        old.unwrap_or(Assignment {
            wallpaper_id: id.into(),
            ..Default::default()
        }),
    );
    use std::io::Write;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, &config)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steam_paths_and_escapes() {
        assert_eq!(
            vdf_paths(
                r#""libraryfolders" { // "path" "wrong"
          "0" { "path" "/games/Steam Library" } "1" { "path" "D:\\Steam" } }"#
            ),
            vec!["/games/Steam Library", "D:\\Steam"]
        );
    }
    #[test]
    fn assignments_preserve_other_outputs_and_properties() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("assignments.json");
        save_assignment(&p, "DP-1", "123").unwrap();
        save_assignment(&p, "DP-2", "456").unwrap();
        let mut config = load_config(&p).unwrap();
        config
            .outputs
            .get_mut("DP-1")
            .unwrap()
            .properties
            .insert("color".into(), serde_json::json!("1 0 0"));
        fs::write(&p, serde_json::to_vec(&config).unwrap()).unwrap();
        save_assignment(&p, "DP-1", "123").unwrap();
        assert_eq!(
            load_config(&p).unwrap().outputs["DP-1"].properties["color"],
            "1 0 0"
        );
        save_assignment(&p, "DP-1", "789").unwrap();
        assert!(
            load_config(&p).unwrap().outputs["DP-1"]
                .properties
                .is_empty()
        );
        save_assignment(&p, "DP-1", "123").unwrap();
        assert_eq!(load_config(&p).unwrap().outputs["DP-1"].wallpaper_id, "123");
        fs::write(&p, b"{broken").unwrap();
        assert!(save_assignment(&p, "DP-1", "789").is_err());
        assert_eq!(fs::read(&p).unwrap(), b"{broken");
    }
}
