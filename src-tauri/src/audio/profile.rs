//! 音频配置文件——保存/恢复应用音量快照。
//!
//! 配置文件存储在 `%APPDATA%/audio-hub/profiles/<name>.json`。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::device::AudioSession;

/// 单个应用的音量快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEntry {
    pub pid: u32,
    pub display_name: String,
    pub volume: f32,
    pub muted: bool,
}

/// 配置文件——一组应用音量快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub entries: Vec<ProfileEntry>,
}

impl Profile {
    /// 从当前会话列表创建配置文件。
    pub fn from_sessions(name: &str, sessions: &[AudioSession]) -> Self {
        Self {
            name: name.to_string(),
            entries: sessions
                .iter()
                .map(|s| ProfileEntry {
                    pid: s.pid,
                    display_name: s.display_name.clone(),
                    volume: s.volume,
                    muted: s.muted,
                })
                .collect(),
        }
    }
}

/// 获取 Profile 存储目录。
fn profiles_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE").unwrap_or_default();
            PathBuf::from(home).join("AppData").join("Roaming")
        });
    base.join("audio-hub").join("profiles")
}

/// 确保存储目录存在。
fn ensure_dir() -> std::io::Result<()> {
    std::fs::create_dir_all(profiles_dir())
}

/// 校验 Profile 名称，确保最终路径不会逃逸存储目录。
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.trim() != name {
        return Err("配置名称不能为空，且首尾不能包含空格".to_string());
    }
    if name.chars().count() > 64 {
        return Err("配置名称不能超过 64 个字符".to_string());
    }
    if name.chars().any(|ch| {
        ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) || name.ends_with('.')
        || name.ends_with(' ')
    {
        return Err("配置名称包含 Windows 文件名不允许的字符".to_string());
    }

    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .and_then(|suffix| suffix.parse::<u8>().ok())
            .is_some_and(|number| (1..=9).contains(&number));
    if reserved {
        return Err("配置名称不能使用 Windows 保留设备名".to_string());
    }

    Ok(())
}

fn profile_path(name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    Ok(profiles_dir().join(format!("{name}.json")))
}

/// 保存配置文件。
pub fn save(name: &str, sessions: &[AudioSession]) -> Result<(), String> {
    ensure_dir().map_err(|e| format!("创建目录失败：{e}"))?;

    let profile = Profile::from_sessions(name, sessions);
    let path = profile_path(name)?;
    let json = serde_json::to_string_pretty(&profile).map_err(|e| format!("序列化失败：{e}"))?;

    std::fs::write(&path, json).map_err(|e| format!("写入文件失败：{e}"))
}

/// 加载配置文件。
pub fn load(name: &str) -> Result<Profile, String> {
    let path = profile_path(name)?;
    let json = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败：{e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("解析失败：{e}"))
}

/// 列出所有配置文件名称。
pub fn list() -> Result<Vec<String>, String> {
    ensure_dir().map_err(|e| format!("创建目录失败：{e}"))?;

    let mut names = Vec::new();
    let dir = profiles_dir();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("读取目录失败：{e}"))?;

    for entry in entries.filter_map(|e| e.ok()) {
        if !entry.file_type().is_ok_and(|file_type| file_type.is_file()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".json") {
            let profile_name = name.trim_end_matches(".json");
            if validate_name(profile_name).is_ok() {
                names.push(profile_name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// 删除配置文件。
pub fn delete(name: &str) -> Result<(), String> {
    let path = profile_path(name)?;
    std::fs::remove_file(&path).map_err(|e| format!("删除文件失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::{Profile, validate_name};

    #[test]
    fn accepts_normal_profile_names() {
        assert!(validate_name("默认配置").is_ok());
        assert!(validate_name("Game Mode 1").is_ok());
    }

    #[test]
    fn rejects_path_traversal_and_reserved_names() {
        for name in ["../escape", r"..\escape", "C:\\escape", "CON", "LPT1.json"] {
            assert!(validate_name(name).is_err(), "{name} 应被拒绝");
        }
    }

    #[test]
    fn ignores_legacy_eq_binding() {
        let profile: Profile = serde_json::from_str(
            r#"{"name":"旧配置","entries":[],"eq_preset":{"device_id":"old","device_name":"旧设备","preset_name":"旧音色"}}"#,
        )
        .unwrap();
        assert_eq!(profile.name, "旧配置");
        assert!(profile.entries.is_empty());
    }
}
