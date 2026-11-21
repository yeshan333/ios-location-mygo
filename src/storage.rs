use crate::coords::validate_coords;
use crate::i18n::Locale;
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Favorite {
    pub id: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub locale: Option<Locale>,
    pub dark_mode: Option<bool>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            locale: None,
            dark_mode: None,
        }
    }
}

pub fn app_data_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("iOS Location");
        let _ = fs::create_dir_all(&dir);
        return dir;
    }
    std::env::temp_dir().join("ios-location")
}

pub fn tile_cache_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join("iOS Location")
            .join("tiles");
        let _ = fs::create_dir_all(&dir);
        return dir;
    }
    std::env::temp_dir().join("ios-location-tiles")
}

pub fn load_settings_from(dir: &PathBuf) -> AppSettings {
    let path = dir.join("settings.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_settings_to(dir: &PathBuf, settings: &AppSettings) -> Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join("settings.json");
    let raw = serde_json::to_string_pretty(settings)?;
    fs::write(path, raw)?;
    Ok(())
}

pub fn list_favorites_from(dir: &PathBuf) -> Vec<Favorite> {
    let path = dir.join("favorites.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<Favorite>>(&raw).ok())
        .unwrap_or_default()
}

pub fn save_favorites_to(dir: &PathBuf, favs: &[Favorite]) -> Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join("favorites.json");
    let raw = serde_json::to_string_pretty(favs)?;
    fs::write(path, raw)?;
    Ok(())
}

pub fn add_favorite_to(dir: &PathBuf, name: &str, lat: f64, lng: f64) -> Result<Vec<Favorite>> {
    validate_coords(lat, lng).map_err(|e| anyhow::anyhow!(e))?;
    let mut clean: String = name.trim().chars().take(40).collect();
    if clean.is_empty() {
        clean = format!("{lat:.4}, {lng:.4}");
    }
    let mut favs = list_favorites_from(dir);
    let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    favs.insert(
        0,
        Favorite {
            id: format!("{now_nanos:x}"),
            name: clean,
            latitude: lat,
            longitude: lng,
            created_at: Utc::now().to_rfc3339(),
        },
    );
    favs.truncate(100);
    save_favorites_to(dir, &favs)?;
    Ok(favs)
}

pub fn delete_favorite_from(dir: &PathBuf, id: &str) -> Result<Vec<Favorite>> {
    let mut favs = list_favorites_from(dir);
    favs.retain(|f| f.id != id);
    save_favorites_to(dir, &favs)?;
    Ok(favs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_favorites_crud() {
        let dir = std::env::temp_dir().join(format!(
            "ios-loc-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(1)
        ));
        let favs = add_favorite_to(&dir, "东京塔", 35.65858, 139.74543).unwrap();
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].name, "东京塔");
        let after = delete_favorite_from(&dir, &favs[0].id).unwrap();
        assert!(after.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
