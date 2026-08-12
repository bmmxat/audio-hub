//! 选定应用失去 Windows 前台焦点后的自动静音管理。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::{
    audio::{device::AudioSession, notifications::SESSION_CHANGED_EVENT, wasapi},
    settings::{self, UnfocusedMuteApplication},
};

pub const UNFOCUSED_MUTE_CHANGED_EVENT: &str = "unfocused-mute-changed";
const RESTORE_FILE_NAME: &str = "unfocused-mute-restore.json";
const FOREGROUND_POLL_INTERVAL: Duration = Duration::from_millis(150);
const SESSION_REFRESH_INTERVAL: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Serialize)]
pub struct UnfocusedMuteStatus {
    pub applications: Vec<UnfocusedMuteApplication>,
    pub auto_muted_keys: Vec<String>,
    pub foreground_key: Option<String>,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RestoreEntry {
    pid: u32,
    key: String,
    #[serde(default)]
    device_id: String,
    was_muted: bool,
    #[serde(default)]
    pending_restore: bool,
}

#[derive(Debug, Default)]
struct RuntimeState {
    applications: BTreeMap<String, String>,
    auto_muted: HashMap<u32, RestoreEntry>,
    pending_restores: Vec<RestoreEntry>,
    foreground_key: Option<String>,
    paused: bool,
}

/// 后台轮询前台窗口并协调 WASAPI 会话静音状态。
pub struct UnfocusedMuteManager {
    state: Arc<Mutex<RuntimeState>>,
    app_data_dir: PathBuf,
    shutdown: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl UnfocusedMuteManager {
    pub fn start(app: AppHandle, app_data_dir: PathBuf) -> Self {
        let saved = settings::load(&app_data_dir);
        let applications = saved
            .unfocused_mute_applications
            .into_iter()
            .filter_map(|application| {
                let key = normalize_app_key(&application.key);
                (!key.is_empty()).then_some((key, application.display_name.trim().to_string()))
            })
            .collect();
        let mut auto_muted = HashMap::new();
        let mut pending_restores = Vec::new();
        for entry in load_restore_entries(&app_data_dir) {
            if entry.pending_restore {
                pending_restores.push(entry);
            } else {
                auto_muted.insert(entry.pid, entry);
            }
        }
        let state = Arc::new(Mutex::new(RuntimeState {
            applications,
            auto_muted,
            pending_restores,
            foreground_key: None,
            paused: false,
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_dir = app_data_dir.clone();
        let worker = thread::Builder::new()
            .name("unfocused-mute-monitor".to_string())
            .spawn(move || {
                let mut previous_foreground_pid = u32::MAX;
                let mut foreground_key = None;
                let mut last_session_refresh = Instant::now() - SESSION_REFRESH_INTERVAL;
                while !worker_shutdown.load(Ordering::Acquire) {
                    let foreground_pid = foreground_process_id();
                    let foreground_changed = foreground_pid != previous_foreground_pid;
                    if foreground_changed {
                        previous_foreground_pid = foreground_pid;
                        foreground_key = wasapi::process_name(foreground_pid)
                            .map(|name| normalize_app_key(&name));
                    }
                    if (foreground_changed
                        || last_session_refresh.elapsed() >= SESSION_REFRESH_INTERVAL)
                        && reconcile(&worker_state, &worker_dir, foreground_key.clone())
                    {
                        let _ = app.emit(SESSION_CHANGED_EVENT, ());
                        let _ = app.emit(UNFOCUSED_MUTE_CHANGED_EVENT, ());
                    }
                    if foreground_changed
                        || last_session_refresh.elapsed() >= SESSION_REFRESH_INTERVAL
                    {
                        last_session_refresh = Instant::now();
                    }
                    thread::sleep(FOREGROUND_POLL_INTERVAL);
                }
            })
            .ok();

        Self {
            state,
            app_data_dir,
            shutdown,
            worker: Mutex::new(worker),
        }
    }

    pub fn status(&self) -> UnfocusedMuteStatus {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        status_from_state(&state)
    }

    /// 返回由本功能临时静音前的原始静音状态，供设备音量快照避开临时状态。
    pub fn auto_muted_baselines(&self) -> HashMap<String, bool> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut baselines = HashMap::new();
        for entry in state.auto_muted.values() {
            baselines
                .entry(entry.key.clone())
                .and_modify(|muted| *muted |= entry.was_muted)
                .or_insert(entry.was_muted);
        }
        baselines
    }

    /// 在设备或会话拓扑变化后立即重新绑定，不等待后台轮询周期。
    pub fn reconcile_now(&self) -> bool {
        reconcile(&self.state, &self.app_data_dir, foreground_process_key())
    }

    pub fn set_application(
        &self,
        key: String,
        display_name: String,
        enabled: bool,
    ) -> Result<UnfocusedMuteStatus, String> {
        let display_name = display_name.trim().to_string();
        let key = normalize_app_key(&key);
        if key.is_empty() || key == "system sounds" || key == "系统音效" {
            return Err("请选择一个有效的应用音频会话".to_string());
        }

        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut next_applications = state.applications.clone();
        if enabled {
            next_applications.insert(key, display_name);
        } else {
            next_applications.remove(&key);
        }

        let mut app_settings = settings::load(&self.app_data_dir);
        app_settings.unfocused_mute_applications = next_applications
            .iter()
            .map(|(key, display_name)| UnfocusedMuteApplication {
                key: key.clone(),
                display_name: display_name.clone(),
            })
            .collect();
        settings::save(&self.app_data_dir, &app_settings)?;
        state.applications = next_applications;
        drop(state);

        // 立即应用加入/移除结果，避免等待下一次轮询。
        reconcile(&self.state, &self.app_data_dir, foreground_process_key());
        Ok(self.status())
    }

    /// 暂停或恢复自动静音。暂停只影响当前运行周期，重启后默认恢复运行。
    pub fn toggle_paused(&self) -> UnfocusedMuteStatus {
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.paused = !state.paused;
        }
        reconcile(&self.state, &self.app_data_dir, foreground_process_key());
        self.status()
    }

    /// 程序退出前恢复所有由本功能改变的静音状态。
    pub fn restore_all(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut worker) = self.worker.lock()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
        restore_all(&self.state, &self.app_data_dir);
    }
}

impl Drop for UnfocusedMuteManager {
    fn drop(&mut self) {
        self.restore_all();
    }
}

pub fn normalize_app_key(value: &str) -> String {
    let normalized = value.trim().to_lowercase();
    normalized
        .strip_suffix(".exe")
        .unwrap_or(&normalized)
        .to_string()
}

fn foreground_process_key() -> Option<String> {
    wasapi::process_name(foreground_process_id()).map(|name| normalize_app_key(&name))
}

fn foreground_process_id() -> u32 {
    let window = unsafe { GetForegroundWindow() };
    if window.0.is_null() {
        return 0;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
    }
    pid
}

fn reconcile(
    state: &Arc<Mutex<RuntimeState>>,
    app_data_dir: &Path,
    foreground_key: Option<String>,
) -> bool {
    let tracked_keys: BTreeSet<String> = {
        let runtime = state.lock().unwrap_or_else(|error| error.into_inner());
        runtime
            .applications
            .keys()
            .cloned()
            .chain(runtime.auto_muted.values().map(|entry| entry.key.clone()))
            .collect()
    };
    let Ok(mut sessions) = wasapi::enumerate_sessions() else {
        return false;
    };
    resolve_effective_output_sessions(&mut sessions, &tracked_keys);
    let sessions_by_pid: HashMap<u32, (&AudioSession, String)> = sessions
        .iter()
        .filter(|session| session.pid != 0)
        .map(|session| (session.pid, (session, session_app_key(session))))
        .collect();
    let mut runtime = state.lock().unwrap_or_else(|error| error.into_inner());
    let mut changed = false;
    runtime.foreground_key = foreground_key.clone();

    let tracked_pids: Vec<u32> = runtime.auto_muted.keys().copied().collect();
    for pid in tracked_pids {
        let Some(entry) = runtime.auto_muted.get(&pid).cloned() else {
            continue;
        };
        let current = sessions_by_pid.get(&pid);
        let still_same_application = current.is_some_and(|(_, key)| *key == entry.key);
        let still_same_device = current.is_some_and(|(session, _)| {
            entry.device_id.is_empty() || session.device_id == entry.device_id
        });
        let current_muted = match restore_entry_session_state(&entry) {
            Ok(state) => state,
            Err(_) if !still_same_application || !still_same_device => None,
            Err(_) => {
                // 读取端点失败时保留恢复记录，下一轮继续尝试。
                continue;
            }
        };
        let should_stay_auto_muted = still_same_application
            && !runtime.paused
            && runtime.applications.contains_key(&entry.key)
            && foreground_key.as_deref() != Some(entry.key.as_str());

        if should_stay_auto_muted && still_same_device && current_muted.is_some() {
            if current_muted == Some(false) && set_restore_entry_mute(&entry, true).is_ok() {
                changed = true;
            }
            continue;
        }

        if should_stay_auto_muted
            && !still_same_device
            && let Some((target_session, _)) = current
        {
            // 安全交接：先静音新端点，再把旧端点放入待恢复队列。
            // 旧端点必须保持静音，直到 Windows 报告其会话不再活动。
            if !target_session.muted && set_session_mute_on_device(target_session, true).is_err() {
                continue;
            }

            let target_pending_index = runtime.pending_restores.iter().position(|pending| {
                pending.pid == pid
                    && pending.key == entry.key
                    && pending.device_id == target_session.device_id
            });
            let target_baseline = target_pending_index
                .map(|index| runtime.pending_restores.remove(index).was_muted)
                .unwrap_or(entry.was_muted);

            if !entry.device_id.is_empty()
                && entry.device_id != target_session.device_id
                && !runtime.pending_restores.iter().any(|pending| {
                    pending.pid == entry.pid
                        && pending.key == entry.key
                        && pending.device_id == entry.device_id
                })
            {
                let mut pending = entry.clone();
                pending.pending_restore = true;
                runtime.pending_restores.push(pending);
            }

            runtime.auto_muted.insert(
                pid,
                RestoreEntry {
                    pid,
                    key: entry.key,
                    device_id: target_session.device_id.clone(),
                    was_muted: target_baseline,
                    pending_restore: false,
                },
            );
            changed = true;
            continue;
        }

        if let Some(current_muted) = current_muted {
            let already_restored = current_muted == entry.was_muted;
            if !already_restored && set_restore_entry_mute(&entry, entry.was_muted).is_err() {
                // 保留恢复记录，下一轮继续尝试，避免瞬时驱动失败后丢失原状态。
                continue;
            }
        }
        runtime.auto_muted.remove(&pid);
        changed = true;
    }

    for session in &sessions {
        if session.pid == 0 {
            continue;
        }
        let key = session_app_key(session);
        let should_mute = !runtime.paused
            && runtime.applications.contains_key(&key)
            && foreground_key.as_deref() != Some(key.as_str());
        if !should_mute || runtime.auto_muted.contains_key(&session.pid) {
            continue;
        }
        if session.muted || set_session_mute_on_device(session, true).is_ok() {
            let pending_index = runtime.pending_restores.iter().position(|pending| {
                pending.pid == session.pid
                    && pending.key == key
                    && pending.device_id == session.device_id
            });
            let was_muted = pending_index
                .map(|index| runtime.pending_restores.remove(index).was_muted)
                .unwrap_or(session.muted);
            runtime.auto_muted.insert(
                session.pid,
                RestoreEntry {
                    pid: session.pid,
                    key,
                    device_id: session.device_id.clone(),
                    was_muted,
                    pending_restore: false,
                },
            );
            changed = true;
        }
    }

    let mut pending_index = 0;
    while pending_index < runtime.pending_restores.len() {
        let entry = runtime.pending_restores[pending_index].clone();
        let covered_by_current = runtime.auto_muted.get(&entry.pid).is_some_and(|current| {
            current.key == entry.key && current.device_id == entry.device_id
        });
        if covered_by_current {
            runtime.pending_restores.remove(pending_index);
            changed = true;
            continue;
        }

        let should_keep_muted = !runtime.paused
            && runtime.applications.contains_key(&entry.key)
            && foreground_key.as_deref() != Some(entry.key.as_str());
        let action = if !should_keep_muted {
            restore_pending_entry(&entry)
        } else {
            pending_restore_action(&entry)
        };
        match action {
            PendingRestoreAction::RestoredOrGone => {
                runtime.pending_restores.remove(pending_index);
                changed = true;
            }
            PendingRestoreAction::Keep => pending_index += 1,
        }
    }

    if changed {
        let _ = save_runtime_entries(app_data_dir, &runtime);
    }
    changed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRestoreAction {
    Keep,
    RestoredOrGone,
}

fn restore_pending_entry(entry: &RestoreEntry) -> PendingRestoreAction {
    match restore_entry_session_state(entry) {
        Ok(Some(current_muted)) => {
            if current_muted == entry.was_muted
                || set_restore_entry_mute(entry, entry.was_muted).is_ok()
            {
                PendingRestoreAction::RestoredOrGone
            } else {
                PendingRestoreAction::Keep
            }
        }
        Ok(None) => PendingRestoreAction::RestoredOrGone,
        Err(_) => PendingRestoreAction::Keep,
    }
}

fn pending_restore_action(entry: &RestoreEntry) -> PendingRestoreAction {
    let current_muted = match restore_entry_session_state(entry) {
        Ok(Some(muted)) => muted,
        Ok(None) => return PendingRestoreAction::RestoredOrGone,
        Err(_) => return PendingRestoreAction::Keep,
    };

    // 原始状态本来就是静音时无需等待；保持静音即已恢复到基线。
    if entry.was_muted {
        return if current_muted || set_restore_entry_mute(entry, true).is_ok() {
            PendingRestoreAction::RestoredOrGone
        } else {
            PendingRestoreAction::Keep
        };
    }

    match wasapi::session_active_on_device(&entry.device_id, entry.pid) {
        Ok(Some(true)) => {
            // 旧流仍可能向该端点输出，继续保持静音。
            if !current_muted {
                let _ = set_restore_entry_mute(entry, true);
            }
            PendingRestoreAction::Keep
        }
        Ok(Some(false)) => {
            if !current_muted || set_restore_entry_mute(entry, false).is_ok() {
                PendingRestoreAction::RestoredOrGone
            } else {
                PendingRestoreAction::Keep
            }
        }
        Ok(None) => PendingRestoreAction::RestoredOrGone,
        Err(_) => PendingRestoreAction::Keep,
    }
}

/// 路由切换时，新旧端点上的会话可能同时存在；应用已经静音后，两者还可能都被
/// Windows 标记为非活动。此时不能依靠会话活跃状态判断实际目标端点，而应优先使用
/// Windows 保存的 per-app 路由；没有独立路由时使用当前默认输出。
fn resolve_effective_output_sessions(
    sessions: &mut [AudioSession],
    tracked_keys: &BTreeSet<String>,
) {
    let default_device_id = wasapi::get_default_device_id().ok();
    let mut sessions_by_device: HashMap<String, Vec<AudioSession>> = HashMap::new();

    for session in sessions {
        if session.pid == 0 {
            continue;
        }
        let key = session_app_key(session);
        if !tracked_keys.contains(&key) {
            continue;
        }

        let target_device_id = match wasapi::get_app_output_device(session.pid) {
            Ok(Some(device_id)) if !device_id.is_empty() => Some(device_id),
            Ok(_) => default_device_id.clone(),
            Err(_) => None,
        };
        let Some(target_device_id) = target_device_id else {
            continue;
        };
        if target_device_id == session.device_id {
            continue;
        }

        if !sessions_by_device.contains_key(&target_device_id) {
            let Ok(device_sessions) = wasapi::enumerate_sessions_for_device(&target_device_id)
            else {
                continue;
            };
            sessions_by_device.insert(target_device_id.clone(), device_sessions);
        }
        let Some(effective_session) =
            sessions_by_device
                .get(&target_device_id)
                .and_then(|device_sessions| {
                    find_application_session(device_sessions, session.pid, &key)
                })
        else {
            // 目标端点尚未生成会话时保留当前会话，下一轮继续迁移。
            continue;
        };
        *session = effective_session.clone();
    }
}

fn find_application_session<'a>(
    sessions: &'a [AudioSession],
    pid: u32,
    key: &str,
) -> Option<&'a AudioSession> {
    sessions
        .iter()
        .find(|session| session.pid == pid && session_app_key(session) == key)
}

fn restore_all(state: &Arc<Mutex<RuntimeState>>, app_data_dir: &Path) {
    let mut runtime = state.lock().unwrap_or_else(|error| error.into_inner());
    runtime
        .auto_muted
        .retain(|_, entry| restore_pending_entry(entry) == PendingRestoreAction::Keep);
    runtime
        .pending_restores
        .retain(|entry| restore_pending_entry(entry) == PendingRestoreAction::Keep);
    let _ = save_runtime_entries(app_data_dir, &runtime);
}

fn set_session_mute_on_device(session: &AudioSession, muted: bool) -> windows::core::Result<()> {
    if session.device_id.is_empty() {
        wasapi::set_session_mute(session.pid, muted)
    } else {
        wasapi::set_session_mute_for_device(&session.device_id, session.pid, muted)
    }
}

fn set_restore_entry_mute(entry: &RestoreEntry, muted: bool) -> windows::core::Result<()> {
    if entry.device_id.is_empty() {
        // 兼容 v0.3.1 及更早版本创建的恢复记录。
        wasapi::set_session_mute(entry.pid, muted)
    } else {
        wasapi::set_session_mute_for_device(&entry.device_id, entry.pid, muted)
    }
}

fn restore_entry_session_state(entry: &RestoreEntry) -> windows::core::Result<Option<bool>> {
    let sessions = if entry.device_id.is_empty() {
        wasapi::enumerate_sessions()?
    } else {
        wasapi::enumerate_sessions_for_device(&entry.device_id)?
    };
    Ok(sessions
        .iter()
        .find(|session| restore_entry_matches_session(entry, session))
        .map(|session| session.muted))
}

fn restore_entry_matches_session(entry: &RestoreEntry, session: &AudioSession) -> bool {
    entry.pid == session.pid
        && entry.key == session_app_key(session)
        && (entry.device_id.is_empty() || entry.device_id == session.device_id)
}

fn session_app_key(session: &AudioSession) -> String {
    normalize_app_key(
        session
            .process_name
            .as_deref()
            .unwrap_or(&session.display_name),
    )
}

fn status_from_state(state: &RuntimeState) -> UnfocusedMuteStatus {
    let applications = state
        .applications
        .iter()
        .map(|(key, display_name)| UnfocusedMuteApplication {
            key: key.clone(),
            display_name: display_name.clone(),
        })
        .collect();
    let auto_muted_keys = state
        .auto_muted
        .values()
        .map(|entry| entry.key.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    UnfocusedMuteStatus {
        applications,
        auto_muted_keys,
        foreground_key: state.foreground_key.clone(),
        paused: state.paused,
    }
}

fn restore_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(RESTORE_FILE_NAME)
}

fn load_restore_entries(app_data_dir: &Path) -> Vec<RestoreEntry> {
    fs::read_to_string(restore_path(app_data_dir))
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

fn save_restore_entries<'a>(
    app_data_dir: &Path,
    entries: impl Iterator<Item = &'a RestoreEntry>,
) -> Result<(), String> {
    fs::create_dir_all(app_data_dir).map_err(|error| format!("无法创建配置目录：{error}"))?;
    let mut entries: Vec<_> = entries.cloned().collect();
    entries.sort_by(|left, right| {
        (left.pid, left.pending_restore, left.device_id.as_str()).cmp(&(
            right.pid,
            right.pending_restore,
            right.device_id.as_str(),
        ))
    });
    let contents = serde_json::to_vec_pretty(&entries)
        .map_err(|error| format!("无法序列化未聚焦静音恢复状态：{error}"))?;
    fs::write(restore_path(app_data_dir), contents)
        .map_err(|error| format!("无法保存未聚焦静音恢复状态：{error}"))
}

fn save_runtime_entries(app_data_dir: &Path, state: &RuntimeState) -> Result<(), String> {
    save_restore_entries(
        app_data_dir,
        state
            .auto_muted
            .values()
            .chain(state.pending_restores.iter()),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        RestoreEntry, find_application_session, normalize_app_key, restore_entry_matches_session,
        session_app_key,
    };
    use crate::audio::device::AudioSession;

    #[test]
    fn normalizes_process_and_display_names_to_the_same_key() {
        assert_eq!(normalize_app_key(" Discord.EXE "), "discord");
        assert_eq!(normalize_app_key("Discord"), "discord");
        assert_eq!(normalize_app_key("网易云音乐"), "网易云音乐");
    }

    #[test]
    fn session_matching_prefers_the_real_process_name() {
        let session = AudioSession {
            display_name: "Example Player".to_string(),
            process_name: Some("player-core.exe".to_string()),
            device_id: "device".to_string(),
            pid: 42,
            volume: 0.8,
            muted: false,
        };
        assert_eq!(session_app_key(&session), "player-core");
    }

    #[test]
    fn restore_entry_matches_the_original_output_device() {
        let entry = RestoreEntry {
            pid: 42,
            key: "player-core".to_string(),
            device_id: "headphones".to_string(),
            was_muted: false,
            pending_restore: false,
        };
        let headphones = AudioSession {
            display_name: "Example Player".to_string(),
            process_name: Some("player-core.exe".to_string()),
            device_id: "headphones".to_string(),
            pid: 42,
            volume: 0.8,
            muted: true,
        };
        let speakers = AudioSession {
            device_id: "speakers".to_string(),
            ..headphones.clone()
        };
        assert!(restore_entry_matches_session(&entry, &headphones));
        assert!(!restore_entry_matches_session(&entry, &speakers));
    }

    #[test]
    fn legacy_restore_entry_matches_without_a_device_id() {
        let entry = RestoreEntry {
            pid: 42,
            key: "player-core".to_string(),
            device_id: String::new(),
            was_muted: false,
            pending_restore: false,
        };
        let session = AudioSession {
            display_name: "Example Player".to_string(),
            process_name: Some("player-core.exe".to_string()),
            device_id: "headphones".to_string(),
            pid: 42,
            volume: 0.8,
            muted: true,
        };
        assert!(restore_entry_matches_session(&entry, &session));
    }

    #[test]
    fn legacy_restore_entry_defaults_to_current_endpoint_record() {
        let entry: RestoreEntry = serde_json::from_str(
            r#"{"pid":42,"key":"player-core","device_id":"device","was_muted":false}"#,
        )
        .unwrap();

        assert!(!entry.pending_restore);
    }

    #[test]
    fn finds_the_same_application_session_after_output_migration() {
        let sessions = vec![
            AudioSession {
                display_name: "Example Player".to_string(),
                process_name: Some("player-core.exe".to_string()),
                device_id: "new-output".to_string(),
                pid: 42,
                volume: 0.8,
                muted: false,
            },
            AudioSession {
                display_name: "Other Player".to_string(),
                process_name: Some("other.exe".to_string()),
                device_id: "new-output".to_string(),
                pid: 43,
                volume: 0.5,
                muted: false,
            },
        ];

        let migrated = find_application_session(&sessions, 42, "player-core").unwrap();
        assert_eq!(migrated.device_id, "new-output");
        assert!(find_application_session(&sessions, 42, "other").is_none());
    }
}
