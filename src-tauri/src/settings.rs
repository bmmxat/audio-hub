//! 应用设置持久化（%APPDATA%/audio-hub/settings.json）。

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// 关闭按钮（右上角 X）的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    /// 最小化到系统托盘，窗口转入后台。
    #[default]
    Minimize,
    /// 退出程序。
    Quit,
}

/// 需要在未聚焦时自动静音的应用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnfocusedMuteApplication {
    /// 归一化后的稳定应用标识。
    pub key: String,
    /// 最近一次从音频会话读取到的显示名称。
    pub display_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub close_behavior: CloseBehavior,
    /// 用户是否已完成首次选择；false 时每次关闭都询问。
    #[serde(default)]
    pub close_behavior_chosen: bool,
    /// 失去前台焦点后需要自动静音的应用列表。
    #[serde(default)]
    pub unfocused_mute_applications: Vec<UnfocusedMuteApplication>,
}

/// 返回给前端的状态。
#[derive(Debug, Clone, Serialize)]
pub struct CloseBehaviorState {
    pub behavior: CloseBehavior,
    pub chosen: bool,
}

pub fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(SETTINGS_FILE_NAME)
}

pub fn load(app_data_dir: &Path) -> AppSettings {
    fs::read_to_string(settings_path(app_data_dir))
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

pub fn save(app_data_dir: &Path, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app_data_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建配置目录：{error}"))?;
    }
    let contents = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("无法序列化应用设置：{error}"))?;
    fs::write(&path, contents).map_err(|error| format!("无法保存应用设置：{error}"))
}
