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

/// 保存配置文件。
pub fn save(name: &str, sessions: &[AudioSession]) -> Result<(), String> {
    ensure_dir().map_err(|e| format!("创建目录失败：{e}"))?;

    let profile = Profile::from_sessions(name, sessions);
    let path = profiles_dir().join(format!("{name}.json"));
    let json = serde_json::to_string_pretty(&profile).map_err(|e| format!("序列化失败：{e}"))?;

    std::fs::write(&path, json).map_err(|e| format!("写入文件失败：{e}"))
}

/// 加载配置文件。
pub fn load(name: &str) -> Result<Profile, String> {
    let path = profiles_dir().join(format!("{name}.json"));
    let json = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败：{e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("解析失败：{e}"))
}

/// 列出所有配置文件名称。
pub fn list() -> Result<Vec<String>, String> {
    ensure_dir().map_err(|e| format!("创建目录失败：{e}"))?;

    let mut names = Vec::new();
    let dir = profiles_dir();
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("读取目录失败：{e}"))?;

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".json") {
            names.push(name.trim_end_matches(".json").to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// 删除配置文件。
pub fn delete(name: &str) -> Result<(), String> {
    let path = profiles_dir().join(format!("{name}.json"));
    std::fs::remove_file(&path).map_err(|e| format!("删除文件失败：{e}"))
}
