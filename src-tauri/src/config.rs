use anyhow::Result;
use std::path::{Path, PathBuf};

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("config.json")
}

pub fn read_api_key(data_dir: &Path) -> Option<String> {
    let bytes = std::fs::read(config_path(data_dir)).ok()?;
    let map: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    map["api_key"].as_str().map(|s| s.to_string())
}

pub fn write_api_key(data_dir: &Path, key: &str) -> Result<()> {
    let map = serde_json::json!({ "api_key": key });
    std::fs::write(config_path(data_dir), map.to_string())?;
    Ok(())
}
