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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RestoreEntry {
    pid: u32,
    key: String,
    was_muted: bool,
}

#[derive(Debug, Default)]
struct RuntimeState {
    applications: BTreeMap<String, String>,
    auto_muted: HashMap<u32, RestoreEntry>,
    foreground_key: Option<String>,
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
        let auto_muted = load_restore_entries(&app_data_dir)
            .into_iter()
            .map(|entry| (entry.pid, entry))
            .collect();
        let state = Arc::new(Mutex::new(RuntimeState {
            applications,
            auto_muted,
            foreground_key: None,
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
    let Ok(sessions) = wasapi::enumerate_sessions() else {
        return false;
    };
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
        let should_remain_muted = still_same_application
            && runtime.applications.contains_key(&entry.key)
            && foreground_key.as_deref() != Some(entry.key.as_str());

        if should_remain_muted {
            if let Some((session, _)) = current
                && !session.muted
                && wasapi::set_session_mute(pid, true).is_ok()
            {
                changed = true;
            }
            continue;
        }

        if still_same_application {
            let already_restored =
                current.is_some_and(|(session, _)| session.muted == entry.was_muted);
            if !already_restored && wasapi::set_session_mute(pid, entry.was_muted).is_err() {
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
        let should_mute = runtime.applications.contains_key(&key)
            && foreground_key.as_deref() != Some(key.as_str());
        if !should_mute || runtime.auto_muted.contains_key(&session.pid) {
            continue;
        }
        if session.muted || wasapi::set_session_mute(session.pid, true).is_ok() {
            runtime.auto_muted.insert(
                session.pid,
                RestoreEntry {
                    pid: session.pid,
                    key,
                    was_muted: session.muted,
                },
            );
            changed = true;
        }
    }

    if changed {
        let _ = save_restore_entries(app_data_dir, runtime.auto_muted.values());
    }
    changed
}

fn restore_all(state: &Arc<Mutex<RuntimeState>>, app_data_dir: &Path) {
    let sessions = wasapi::enumerate_sessions().unwrap_or_default();
    let sessions_by_pid: HashMap<u32, (String, bool)> = sessions
        .iter()
        .map(|session| (session.pid, (session_app_key(session), session.muted)))
        .collect();
    let mut runtime = state.lock().unwrap_or_else(|error| error.into_inner());
    let entries: Vec<_> = runtime.auto_muted.values().cloned().collect();
    for entry in entries {
        let restored = match sessions_by_pid.get(&entry.pid) {
            Some((key, current_muted)) if key == &entry.key => {
                *current_muted == entry.was_muted
                    || wasapi::set_session_mute(entry.pid, entry.was_muted).is_ok()
            }
            // 会话已结束或 PID 已被其他应用复用，不应改动新的会话。
            _ => true,
        };
        if restored {
            runtime.auto_muted.remove(&entry.pid);
        }
    }
    let _ = save_restore_entries(app_data_dir, runtime.auto_muted.values());
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
    entries.sort_by_key(|entry| entry.pid);
    let contents = serde_json::to_vec_pretty(&entries)
        .map_err(|error| format!("无法序列化未聚焦静音恢复状态：{error}"))?;
    fs::write(restore_path(app_data_dir), contents)
        .map_err(|error| format!("无法保存未聚焦静音恢复状态：{error}"))
}

#[cfg(test)]
mod tests {
    use super::{normalize_app_key, session_app_key};
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
            pid: 42,
            volume: 0.8,
            muted: false,
        };
        assert_eq!(session_app_key(&session), "player-core");
    }
}
