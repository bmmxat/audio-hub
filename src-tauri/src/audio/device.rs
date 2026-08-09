use serde::{Deserialize, Serialize};

/// 设备数据流方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceDirection {
    /// 输出设备（扬声器、耳机等）
    Output,
    /// 输入设备（麦克风等）
    Input,
}

/// 音频设备信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    /// 用户友好名称，例如 "扬声器 (EDIFIER M230)"
    pub name: String,
    /// WASAPI 端点 ID（GUID 格式）
    pub device_id: String,
    /// 是否为当前默认设备
    pub is_default: bool,
}

/// Windows 音频端点的主音量状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DeviceVolumeState {
    /// 主音量（0.0 ~ 1.0）
    pub volume: f32,
    /// 端点是否静音
    pub muted: bool,
}

impl std::fmt::Display for AudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let marker = if self.is_default { " ★" } else { "" };
        write!(f, "{}{}", self.name, marker)
    }
}

/// 音频会话信息（对应 Windows 音量混合器中的每个应用条目）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSession {
    /// 显示名称，例如 "Chrome"、"Discord"、"Steam"
    pub display_name: String,
    /// ToolHelp 快照读取到的实际进程名，用于需要与前台窗口匹配的功能。
    #[serde(default)]
    pub process_name: Option<String>,
    /// 进程 ID
    pub pid: u32,
    /// 音量（0.0 ~ 1.0）
    pub volume: f32,
    /// 是否静音
    pub muted: bool,
}

impl std::fmt::Display for AudioSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mute_marker = if self.muted { " 🔇" } else { "" };
        let vol_pct = (self.volume * 100.0) as u32;
        write!(
            f,
            "{} (PID: {}) 音量: {}%{}",
            self.display_name, self.pid, vol_pct, mute_marker
        )
    }
}
