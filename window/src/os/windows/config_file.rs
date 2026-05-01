use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

const CONFIG_FILE_NAME: &str = "wezterm-windows.conf";

static CONFIG: OnceLock<HashMap<String, String>> = OnceLock::new();

fn config_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(CONFIG_FILE_NAME))
}

fn read_config() -> HashMap<String, String> {
    let Some(path) = config_path() else {
        return HashMap::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };

    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

pub(crate) fn get_string(key: &str) -> Option<String> {
    CONFIG.get_or_init(read_config).get(key).cloned()
}

pub(crate) fn get_bool(key: &str, default: bool) -> bool {
    get_string(key)
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

pub(crate) fn get_u32(key: &str, default: u32) -> u32 {
    get_string(key)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

pub(crate) fn get_i16(key: &str, default: i16) -> i16 {
    get_string(key)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
