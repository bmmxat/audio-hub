//! 按默认输出设备保存并恢复应用音量。
//!
//! 状态和执行都位于 Rust 后端，因此主窗口隐藏或 WebView 被限速时，
//! Windows 默认输出变化仍能立即触发音量切换。

use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    device::{AudioDevice, AudioSession, DeviceDirection},
    wasapi,
};
use crate::unfocused_mute::normalize_app_key;

pub const DEVICE_VOLUME_FOLLOW_EVENT: &str = "device-volume-follow-applied";
const STATE_FILE_NAME: &str = "device-volume-snapshots.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionVolumeSnapshot {
    pub display_name: String,
    pub volume: f32,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceVolumeSnapshot {
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub sessions: HashMap<String, SessionVolumeSnapshot>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedState {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    current_device_id: Option<String>,
    #[serde(default)]
    current_device_name: Option<String>,
    #[serde(default)]
    snapshots: HashMap<String, DeviceVolumeSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceVolumeFollowStatus {
    pub enabled: bool,
    pub current_device_id: Option<String>,
    pub current_device_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceVolumeFollowEvent {
    pub device_id: String,
    pub device_name: String,
    pub snapshot_found: bool,
    pub applied: usize,
}

pub struct DeviceVolumeFollowManager {
    state: Mutex<PersistedState>,
    state_path: PathBuf,
}

impl DeviceVolumeFollowManager {
    pub fn load(app_data_dir: &Path) -> Self {
        let state_path = app_data_dir.join(STATE_FILE_NAME);
        let state = fs::read_to_string(&state_path)
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default();
        Self {
            state: Mutex::new(state),
            state_path,
        }
    }

    pub fn status(&self) -> DeviceVolumeFollowStatus {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        status_from_state(&state)
    }

    pub fn configure(
        &self,
        enabled: bool,
        legacy_snapshots: Option<HashMap<String, DeviceVolumeSnapshot>>,
        auto_muted_keys: &HashSet<String>,
    ) -> Result<DeviceVolumeFollowStatus, String> {
        let default_device = default_output_device()?;
        let sessions = if enabled {
            Some(
                wasapi::enumerate_sessions_for_device(&default_device.device_id)
                    .map_err(|error| format!("无法读取当前应用音量：{error:?}"))?,
            )
        } else {
            None
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| "音量随扬声器状态已损坏。".to_string())?;
        if state.snapshots.is_empty()
            && let Some(legacy_snapshots) = legacy_snapshots
        {
            state.snapshots = legacy_snapshots;
        }
        state.enabled = enabled;
        state.current_device_id = Some(default_device.device_id.clone());
        state.current_device_name = Some(default_device.name.clone());
        if let Some(sessions) = sessions {
            state.snapshots.insert(
                default_device.device_id,
                build_snapshot(&default_device.name, &sessions, auto_muted_keys),
            );
        }
        self.save(&state)?;
        Ok(status_from_state(&state))
    }

    pub fn capture_current(&self, auto_muted_keys: &HashSet<String>) -> Result<(), String> {
        self.capture_device(None, auto_muted_keys)
    }

    pub fn capture_device(
        &self,
        requested_device_id: Option<&str>,
        auto_muted_keys: &HashSet<String>,
    ) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "音量随扬声器状态已损坏。".to_string())?;
        if !state.enabled {
            return Ok(());
        }
        let device_id = requested_device_id
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| state.current_device_id.clone());
        let Some(device_id) = device_id else {
            return Ok(());
        };
        let current_device_name = (state.current_device_id.as_deref() == Some(device_id.as_str()))
            .then(|| state.current_device_name.clone())
            .flatten();
        drop(state);
        let sessions = wasapi::enumerate_sessions_for_device(&device_id)
            .map_err(|error| format!("无法读取当前应用音量：{error:?}"))?;
        let device_name = current_device_name.unwrap_or_else(|| {
            wasapi::enumerate_devices(DeviceDirection::Output)
                .ok()
                .and_then(|devices| {
                    devices
                        .into_iter()
                        .find(|device| device.device_id == device_id)
                        .map(|device| device.name)
                })
                .unwrap_or_default()
        });
        let mut state = self
            .state
            .lock()
            .map_err(|_| "音量随扬声器状态已损坏。".to_string())?;
        if !state.enabled {
            return Ok(());
        }
        merge_snapshot(
            state
                .snapshots
                .entry(device_id)
                .or_insert_with(|| build_snapshot(&device_name, &[], auto_muted_keys)),
            &device_name,
            &sessions,
            auto_muted_keys,
        );
        self.save(&state)
    }

    pub fn apply_current_snapshot(&self) -> Result<usize, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "音量随扬声器状态已损坏。".to_string())?;
        if !state.enabled {
            return Ok(0);
        }
        let Some(device_id) = state.current_device_id.as_deref() else {
            return Ok(0);
        };
        let Some(snapshot) = state.snapshots.get(device_id) else {
            return Ok(0);
        };
        let sessions = wasapi::enumerate_sessions_for_device(device_id)
            .map_err(|error| format!("无法读取当前扬声器会话：{error:?}"))?;
        Ok(apply_snapshot(device_id, &sessions, snapshot))
    }

    pub fn handle_default_output_change(
        &self,
        auto_muted_keys: &HashSet<String>,
    ) -> Result<Option<DeviceVolumeFollowEvent>, String> {
        let default_device = default_output_device()?;
        let sessions = wasapi::enumerate_sessions_for_device(&default_device.device_id)
            .map_err(|error| format!("无法读取切换后的应用音量：{error:?}"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "音量随扬声器状态已损坏。".to_string())?;
        if !state.enabled {
            state.current_device_id = Some(default_device.device_id);
            state.current_device_name = Some(default_device.name);
            self.save(&state)?;
            return Ok(None);
        }
        if state.current_device_id.as_deref() == Some(default_device.device_id.as_str()) {
            return Ok(None);
        }

        state.current_device_id = Some(default_device.device_id.clone());
        state.current_device_name = Some(default_device.name.clone());
        let snapshot = state.snapshots.get(&default_device.device_id).cloned();
        let applied = if let Some(snapshot) = snapshot.as_ref() {
            apply_snapshot(&default_device.device_id, &sessions, snapshot)
        } else {
            state.snapshots.insert(
                default_device.device_id.clone(),
                build_snapshot(&default_device.name, &sessions, auto_muted_keys),
            );
            0
        };
        self.save(&state)?;
        Ok(Some(DeviceVolumeFollowEvent {
            device_id: default_device.device_id,
            device_name: default_device.name,
            snapshot_found: snapshot.is_some(),
            applied,
        }))
    }

    fn save(&self, state: &PersistedState) -> Result<(), String> {
        if let Some(parent) = self.state_path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("无法创建音量快照目录：{error}"))?;
        }
        let contents = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("无法序列化音量快照：{error}"))?;
        fs::write(&self.state_path, contents).map_err(|error| format!("无法保存音量快照：{error}"))
    }
}

fn default_output_device() -> Result<AudioDevice, String> {
    wasapi::enumerate_devices(DeviceDirection::Output)
        .map_err(|error| format!("无法读取默认扬声器：{error:?}"))?
        .into_iter()
        .find(|device| device.is_default)
        .ok_or_else(|| "未检测到 Windows 默认扬声器。".to_string())
}

fn build_snapshot(
    device_name: &str,
    sessions: &[AudioSession],
    auto_muted_keys: &HashSet<String>,
) -> DeviceVolumeSnapshot {
    let sessions = sessions
        .iter()
        .map(|session| {
            let key = session_key(session);
            (
                key.clone(),
                SessionVolumeSnapshot {
                    display_name: session.display_name.clone(),
                    volume: session.volume.clamp(0.0, 1.0),
                    muted: session.muted && !auto_muted_keys.contains(&key),
                },
            )
        })
        .collect();
    DeviceVolumeSnapshot {
        device_name: device_name.to_string(),
        updated_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default(),
        sessions,
    }
}

fn merge_snapshot(
    snapshot: &mut DeviceVolumeSnapshot,
    device_name: &str,
    sessions: &[AudioSession],
    auto_muted_keys: &HashSet<String>,
) {
    let next = build_snapshot(device_name, sessions, auto_muted_keys);
    snapshot.device_name = next.device_name;
    snapshot.updated_at = next.updated_at;
    snapshot.sessions.extend(next.sessions);
}

fn apply_snapshot(
    device_id: &str,
    sessions: &[AudioSession],
    snapshot: &DeviceVolumeSnapshot,
) -> usize {
    let mut applied_pids = HashSet::new();
    sessions
        .iter()
        .filter(|session| applied_pids.insert(session.pid))
        .filter(|session| {
            let Some(saved) = snapshot.sessions.get(&session_key(session)) else {
                return false;
            };
            wasapi::set_session_volume_for_device(device_id, session.pid, saved.volume).is_ok()
                && wasapi::set_session_mute_for_device(device_id, session.pid, saved.muted).is_ok()
        })
        .count()
}

fn session_key(session: &AudioSession) -> String {
    normalize_app_key(
        session
            .process_name
            .as_deref()
            .unwrap_or(&session.display_name),
    )
}

fn status_from_state(state: &PersistedState) -> DeviceVolumeFollowStatus {
    DeviceVolumeFollowStatus {
        enabled: state.enabled,
        current_device_id: state.current_device_id.clone(),
        current_device_name: state.current_device_name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_uses_stable_process_name_and_excludes_temporary_mute() {
        let sessions = vec![AudioSession {
            display_name: "Discord Voice".to_string(),
            process_name: Some("Discord.exe".to_string()),
            device_id: "device-a".to_string(),
            pid: 42,
            volume: 0.65,
            muted: true,
        }];
        let auto_muted = HashSet::from(["discord".to_string()]);
        let snapshot = build_snapshot("Speakers", &sessions, &auto_muted);
        let saved = snapshot.sessions.get("discord").unwrap();
        assert_eq!(saved.volume, 0.65);
        assert!(!saved.muted);
    }

    #[test]
    fn merging_active_sessions_keeps_inactive_application_memory() {
        let mut snapshot = DeviceVolumeSnapshot {
            device_name: "Old Name".to_string(),
            updated_at: 1,
            sessions: HashMap::from([(
                "game".to_string(),
                SessionVolumeSnapshot {
                    display_name: "Game".to_string(),
                    volume: 0.25,
                    muted: false,
                },
            )]),
        };
        let sessions = vec![AudioSession {
            display_name: "Music".to_string(),
            process_name: Some("music.exe".to_string()),
            device_id: "device-b".to_string(),
            pid: 7,
            volume: 0.8,
            muted: false,
        }];
        merge_snapshot(&mut snapshot, "New Name", &sessions, &HashSet::new());
        assert!(snapshot.sessions.contains_key("game"));
        assert_eq!(snapshot.sessions.get("music").unwrap().volume, 0.8);
        assert_eq!(snapshot.device_name, "New Name");
    }
}
