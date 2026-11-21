use crate::backend::Output;
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
    pub directory: PathBuf,
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
const MAX_PROJECT_BYTES: u64 = 4 * 1024 * 1024;
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
    let file = fs::File::open(path.join("project.json"))?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "project.json must be a regular file"
    );
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(file, MAX_PROJECT_BYTES + 1),
        &mut bytes,
    )?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_PROJECT_BYTES,
        "project.json exceeds 4 MiB"
    );
    let v: serde_json::Value = serde_json::from_slice(&bytes)?;
    // Previews are only read from regular files inside the item directory,
    // whether named by project.json or found by the conventional names.
    let root = path.canonicalize()?;
    let contained = |name: &str| {
        let p = path.join(name).canonicalize().ok()?;
        (p.starts_with(&root) && p.is_file()).then_some(p)
    };
    let preview = v["preview"].as_str().and_then(contained).or_else(|| {
        ["preview.gif", "preview.jpg", "preview.png"]
            .into_iter()
            .find_map(contained)
    });
    Ok(Wallpaper {
        title: v["title"].as_str().unwrap_or(&id).into(),
        directory: path.to_path_buf(),
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
    /// Keyed by connector name: displays without an identity, and every display in files
    /// written before identities existed.
    pub outputs: BTreeMap<String, Assignment>,
    /// Keyed by display identity (`backend::Output::identity`), whatever connector it is on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub displays: BTreeMap<String, Assignment>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            outputs: BTreeMap::new(),
            displays: BTreeMap::new(),
        }
    }
}
impl Config {
    /// The output's entry: by its identity, else by its connector.
    pub fn get(&self, output: &Output) -> Option<&Assignment> {
        output
            .identity
            .as_ref()
            .and_then(|i| self.displays.get(i))
            .or_else(|| self.outputs.get(&output.name))
    }
    /// Stores the output's entry under its identity when it has one, else its connector.
    pub fn insert(&mut self, output: &Output, assignment: Assignment) {
        match &output.identity {
            Some(identity) => self.displays.insert(identity.clone(), assignment),
            None => self.outputs.insert(output.name.clone(), assignment),
        };
    }
    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty() && self.displays.is_empty()
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
/// Assigns wallpaper `id` to `output`, keeping its properties when the wallpaper is unchanged. A
/// display with an identity drops its connector entry so the two never disagree.
pub fn save_assignment(path: &Path, output: &Output, id: &str) -> Result<()> {
    anyhow::ensure!(
        !output.name.is_empty() && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
        "invalid assignment"
    );
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    let mut config = load_config(path)?;
    let old = config.get(output).filter(|a| a.wallpaper_id == id).cloned();
    if output.identity.is_some() {
        config.outputs.remove(&output.name);
    }
    config.insert(
        output,
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
    #[cfg(unix)]
    #[test]
    fn previews_stay_inside_the_item_directory() {
        let d = tempfile::tempdir().unwrap();
        let item = d.path().join("123");
        fs::create_dir(&item).unwrap();
        fs::write(d.path().join("outside.png"), b"x").unwrap();
        std::os::unix::fs::symlink(d.path().join("outside.png"), item.join("preview.png")).unwrap();
        fs::write(
            item.join("project.json"),
            br#"{"preview":"../outside.png"}"#,
        )
        .unwrap();
        assert_eq!(read(&item).unwrap().preview, None);
        fs::write(item.join("preview.jpg"), b"x").unwrap();
        assert_eq!(
            read(&item).unwrap().preview,
            Some(item.join("preview.jpg").canonicalize().unwrap())
        );
    }
    fn connector(name: &str) -> Output {
        Output {
            name: name.into(),
            ..Default::default()
        }
    }
    #[test]
    fn saving_a_display_replaces_its_connector_entry() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("assignments.json");
        save_assignment(&p, &connector("DP-3"), "123").unwrap();
        save_assignment(&p, &connector("DP-6"), "456").unwrap();
        let mut config = load_config(&p).unwrap();
        config
            .outputs
            .get_mut("DP-6")
            .unwrap()
            .properties
            .insert("color".into(), serde_json::json!("1 0 0"));
        fs::write(&p, serde_json::to_vec(&config).unwrap()).unwrap();
        let display = Output {
            identity: Some("Make Model Serial".into()),
            ..connector("DP-6")
        };
        save_assignment(&p, &display, "456").unwrap();
        let config = load_config(&p).unwrap();
        assert_eq!(config.outputs.keys().collect::<Vec<_>>(), ["DP-3"]);
        assert_eq!(
            config.displays["Make Model Serial"].properties["color"],
            "1 0 0"
        );
    }
    #[test]
    fn assignments_preserve_other_outputs_and_properties() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("assignments.json");
        save_assignment(&p, &connector("DP-1"), "123").unwrap();
        save_assignment(&p, &connector("DP-2"), "456").unwrap();
        let mut config = load_config(&p).unwrap();
        config
            .outputs
            .get_mut("DP-1")
            .unwrap()
            .properties
            .insert("color".into(), serde_json::json!("1 0 0"));
        fs::write(&p, serde_json::to_vec(&config).unwrap()).unwrap();
        save_assignment(&p, &connector("DP-1"), "123").unwrap();
        assert_eq!(
            load_config(&p).unwrap().outputs["DP-1"].properties["color"],
            "1 0 0"
        );
        save_assignment(&p, &connector("DP-1"), "789").unwrap();
        assert!(
            load_config(&p).unwrap().outputs["DP-1"]
                .properties
                .is_empty()
        );
        save_assignment(&p, &connector("DP-1"), "123").unwrap();
        assert_eq!(load_config(&p).unwrap().outputs["DP-1"].wallpaper_id, "123");
        fs::write(&p, b"{broken").unwrap();
        assert!(save_assignment(&p, &connector("DP-1"), "789").is_err());
        assert_eq!(fs::read(&p).unwrap(), b"{broken");
    }
}
