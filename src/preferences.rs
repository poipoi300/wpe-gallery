use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThumbnailSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ThumbnailSize {
    pub fn width(self) -> f32 {
        match self {
            Self::Small => 165.,
            Self::Medium => 215.,
            Self::Large => 280.,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub thumbnail_size: ThumbnailSize,
    pub artwork_lighting: bool,
    pub sidebar_width: f32,
}

pub const MIN_SIDEBAR: f32 = 268.;
pub const MAX_SIDEBAR: f32 = 600.;
impl Default for Preferences {
    fn default() -> Self {
        Self {
            thumbnail_size: ThumbnailSize::Medium,
            artwork_lighting: true,
            sidebar_width: 344.,
        }
    }
}

impl Preferences {
    pub fn path() -> PathBuf {
        directories::BaseDirs::new()
            .expect("home directory")
            .config_dir()
            .join("wpe-gallery/preferences.json")
    }

    pub fn read(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let mut value: Self = serde_json::from_slice(&bytes)?;
                value.sidebar_width = value.sidebar_width.clamp(MIN_SIDEBAR, MAX_SIDEBAR);
                Ok(value)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn update(path: &Path, change: impl FnOnce(&mut Self)) -> Result<()> {
        let dir = path.parent().expect("preferences directory");
        fs::create_dir_all(dir)?;
        // Other gallery windows may save a different setting at the same time.
        let lock = fs::File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        lock.lock()?;
        let mut prefs = Self::read(path)?;
        change(&mut prefs);
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer_pretty(&mut temp, &prefs)?;
        temp.as_file().sync_all()?;
        temp.persist(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrades_partial_preferences_and_restores_a_custom_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wpe-gallery/preferences.json");
        assert_eq!(Preferences::read(&path).unwrap(), Preferences::default());
        let prefs = Preferences {
            thumbnail_size: ThumbnailSize::Large,
            artwork_lighting: false,
            sidebar_width: 416.,
        };
        Preferences::update(&path, |p| p.thumbnail_size = prefs.thumbnail_size).unwrap();
        Preferences::update(&path, |p| p.artwork_lighting = prefs.artwork_lighting).unwrap();
        Preferences::update(&path, |p| p.sidebar_width = prefs.sidebar_width).unwrap();
        assert_eq!(Preferences::read(&path).unwrap(), prefs);
        fs::write(&path, br#"{"sidebar_width":9999}"#).unwrap();
        assert_eq!(
            Preferences::read(&path).unwrap(),
            Preferences {
                sidebar_width: MAX_SIDEBAR,
                ..Default::default()
            }
        );
    }
}
