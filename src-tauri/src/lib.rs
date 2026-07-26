mod audio;

use audio::{
    device::{AudioDevice, AudioSession, DeviceDirection},
    eq::{SessionEqConfig, SessionEqManager},
    notifications::AudioNotificationWatcher,
    process_loopback::{
        ProcessCaptureManager, ProcessCaptureResult, ProcessCaptureStatus, ProcessLoopbackSupport,
    },
    profile::{self, Profile},
    wasapi,
};
use tauri::Manager;

/// 获取默认输出设备的端点 ID。
#[tauri::command]
fn get_default_device_id() -> Result<String, String> {
    wasapi::get_default_device_id().map_err(|e| format!("{:?}", e))
}

/// 获取默认输出设备的友好名称。
#[tauri::command]
fn get_default_device_name() -> Result<String, String> {
    wasapi::get_default_device_friendly_name().map_err(|e| format!("{:?}", e))
}

/// 枚举指定方向的所有音频设备。
#[tauri::command]
fn enumerate_devices(direction: DeviceDirection) -> Result<Vec<AudioDevice>, String> {
    wasapi::enumerate_devices(direction).map_err(|e| format!("{:?}", e))
}

/// 枚举所有输出设备上的音频会话（跨设备去重）。
#[tauri::command]
fn enumerate_sessions() -> Result<Vec<AudioSession>, String> {
    wasapi::enumerate_sessions().map_err(|e| format!("{:?}", e))
}

/// 设置指定 PID 应用的音量（0.0 ~ 1.0）。
#[tauri::command]
fn set_session_volume(pid: u32, volume: f32) -> Result<(), String> {
    wasapi::set_session_volume(pid, volume).map_err(|e| format!("{:?}", e))
}

/// 设置指定 PID 应用的静音状态。
#[tauri::command]
fn set_session_mute(pid: u32, muted: bool) -> Result<(), String> {
    wasapi::set_session_mute(pid, muted).map_err(|e| format!("{:?}", e))
}

/// 将指定端点设置为默认设备（Win11 可能不生效）。
#[tauri::command]
fn set_default_device(device_id: String) -> Result<(), String> {
    wasapi::set_default_device(&device_id).map_err(|e| format!("{:?}", e))
}

/// 设置应用输出设备（per-app 路由）。
#[tauri::command]
fn set_app_output_device(pid: u32, device_id: String) -> Result<(), String> {
    wasapi::set_app_output_device(pid, &device_id).map_err(|e| format!("{:?}", e))
}

/// 打开 Windows 声音设置面板（降级方案）。
#[tauri::command]
fn open_sound_settings() {
    wasapi::open_sound_settings();
}

/// Windows 原生音频通知是否已成功启用。
#[tauri::command]
fn audio_notifications_available(watcher: tauri::State<'_, AudioNotificationWatcher>) -> bool {
    watcher.is_available()
}

#[tauri::command]
fn process_loopback_support() -> ProcessLoopbackSupport {
    ProcessCaptureManager::support()
}

#[tauri::command]
fn process_capture_status(
    manager: tauri::State<'_, ProcessCaptureManager>,
) -> ProcessCaptureStatus {
    manager.status()
}

#[tauri::command]
fn start_process_capture(
    pid: u32,
    app: tauri::AppHandle,
    manager: tauri::State<'_, ProcessCaptureManager>,
    eq_manager: tauri::State<'_, SessionEqManager>,
) -> Result<ProcessCaptureStatus, String> {
    let output_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定录制目录: {error}"))?
        .join("captures");
    manager.start(pid, &output_dir, eq_manager.get(pid))
}

#[tauri::command]
fn get_session_eq(pid: u32, manager: tauri::State<'_, SessionEqManager>) -> SessionEqConfig {
    manager.get(pid)
}

#[tauri::command]
fn set_session_eq(
    pid: u32,
    config: SessionEqConfig,
    manager: tauri::State<'_, SessionEqManager>,
) -> Result<SessionEqConfig, String> {
    manager.set(pid, config)
}

#[tauri::command]
fn reset_session_eq(pid: u32, manager: tauri::State<'_, SessionEqManager>) -> SessionEqConfig {
    manager.reset(pid)
}

#[tauri::command]
fn stop_process_capture(
    manager: tauri::State<'_, ProcessCaptureManager>,
) -> Result<ProcessCaptureResult, String> {
    manager.stop()
}

#[tauri::command]
fn reveal_capture_file(path: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(path);
    if !path.is_file() {
        return Err("录制文件不存在".to_string());
    }
    std::process::Command::new("explorer.exe")
        .arg("/select,")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开录制文件位置: {error}"))
}

// ── 窗口控制 ──────────────────────────────────────────
#[tauri::command]
fn win_minimize(window: tauri::Window) {
    let _ = window.minimize();
}
#[tauri::command]
fn win_toggle_maximize(window: tauri::Window) {
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}
#[tauri::command]
fn win_close(window: tauri::Window, capture_manager: tauri::State<'_, ProcessCaptureManager>) {
    if capture_manager.status().active {
        let _ = capture_manager.stop();
    }
    let _ = window.close();
}

// ── Profile 命令 ─────────────────────────────────────
// 注意：前台通过 enumerate_sessions 获取当前会话列表后，
// 在 JS 层传递给保存命令，而非在 Rust 层重复调用。

/// 保存当前音量配置。
#[tauri::command]
fn save_profile(name: String, sessions: Vec<AudioSession>) -> Result<(), String> {
    profile::save(&name, &sessions)
}

/// 加载指定配置。
#[tauri::command]
fn load_profile(name: String) -> Result<Profile, String> {
    profile::load(&name)
}

/// 列出所有配置名称。
#[tauri::command]
fn list_profiles() -> Result<Vec<String>, String> {
    profile::list()
}

/// 删除指定配置。
#[tauri::command]
fn delete_profile(name: String) -> Result<(), String> {
    profile::delete(&name)
}

/// 应用配置——将保存的音量/静音恢复到当前活跃会话。
#[tauri::command]
fn apply_profile(name: String) -> Result<(), String> {
    let p = profile::load(&name)?;
    wasapi::apply_profile(&p).map_err(|e| format!("{e:?}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(AudioNotificationWatcher::start(app.handle().clone()));
            app.manage(ProcessCaptureManager::default());
            app.manage(SessionEqManager::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_default_device_id,
            get_default_device_name,
            enumerate_devices,
            enumerate_sessions,
            set_session_volume,
            set_session_mute,
            set_default_device,
            set_app_output_device,
            open_sound_settings,
            audio_notifications_available,
            process_loopback_support,
            process_capture_status,
            start_process_capture,
            stop_process_capture,
            reveal_capture_file,
            get_session_eq,
            set_session_eq,
            reset_session_eq,
            win_minimize,
            win_toggle_maximize,
            win_close,
            save_profile,
            load_profile,
            list_profiles,
            delete_profile,
            apply_profile,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Audio Hub 失败");
}
