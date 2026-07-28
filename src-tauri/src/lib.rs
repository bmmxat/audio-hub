mod audio;
mod autostart;
mod plugins;

use audio::{
    device::{AudioDevice, AudioSession, DeviceDirection, DeviceVolumeState},
    notifications::AudioNotificationWatcher,
    process_loopback::{
        ProcessCaptureManager, ProcessCaptureResult, ProcessCaptureStatus, ProcessLoopbackSupport,
    },
    profile::{self, Profile},
    wasapi,
};
use plugins::equalizer_apo::{
    self, EqPresetCatalog, EqualizerApoStatus, GlobalEqConfig, MicrophoneConfig,
    MicrophoneConfigState,
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

/// 获取指定输出或输入设备的 Windows 主音量。
#[tauri::command]
fn get_device_volume(device_id: String) -> Result<DeviceVolumeState, String> {
    wasapi::get_device_volume(&device_id).map_err(|error| format!("{error:?}"))
}

/// 设置指定输出或输入设备的 Windows 主音量。
#[tauri::command]
fn set_device_volume(device_id: String, volume: f32) -> Result<DeviceVolumeState, String> {
    wasapi::set_device_volume(&device_id, volume).map_err(|error| format!("{error:?}"))
}

/// 设置指定输出或输入设备的静音状态。
#[tauri::command]
fn set_device_mute(device_id: String, muted: bool) -> Result<DeviceVolumeState, String> {
    wasapi::set_device_mute(&device_id, muted).map_err(|error| format!("{error:?}"))
}

/// 当前程序是否已注册为登录 Windows 后自动启动。
#[tauri::command]
fn get_autostart_enabled() -> Result<bool, String> {
    autostart::is_enabled()
}

/// 启用或关闭当前用户的登录自启动。
#[tauri::command]
fn set_autostart_enabled(enabled: bool) -> Result<bool, String> {
    autostart::set_enabled(enabled)
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

#[tauri::command]
fn equalizer_apo_status(app: tauri::AppHandle) -> Result<EqualizerApoStatus, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    Ok(equalizer_apo::status(Some(&app_data_dir)))
}

#[tauri::command]
fn equalizer_apo_enabled_devices(device_ids: Vec<String>) -> Vec<String> {
    equalizer_apo::enabled_device_ids(device_ids)
}

#[tauri::command]
fn choose_rnnoise_plugin_directory(
    app: tauri::AppHandle,
) -> Result<Option<EqualizerApoStatus>, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::choose_rnnoise_plugin_directory(&app_data_dir)
}

#[tauri::command]
fn get_global_eq(device_id: String, app: tauri::AppHandle) -> Result<GlobalEqConfig, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::get_endpoint_eq(&app_data_dir, &device_id)
}

#[tauri::command]
fn set_global_eq(
    device_id: String,
    device_name: String,
    config: GlobalEqConfig,
    app: tauri::AppHandle,
) -> Result<GlobalEqConfig, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::set_endpoint_eq(&app_data_dir, &device_id, &device_name, config)
}

#[tauri::command]
fn get_microphone_processing(
    device_id: String,
    app: tauri::AppHandle,
) -> Result<MicrophoneConfigState, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::get_microphone_config(&app_data_dir, &device_id)
}

#[tauri::command]
fn set_microphone_processing(
    device_id: String,
    device_name: String,
    config: MicrophoneConfig,
    app: tauri::AppHandle,
) -> Result<MicrophoneConfig, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::set_microphone_config(&app_data_dir, &device_id, &device_name, config)
}

#[tauri::command]
fn list_global_eq_presets(
    device_id: String,
    app: tauri::AppHandle,
) -> Result<EqPresetCatalog, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::list_presets(&app_data_dir, &device_id)
}

#[tauri::command]
fn get_global_eq_preset(
    device_id: String,
    preset_name: String,
    app: tauri::AppHandle,
) -> Result<GlobalEqConfig, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::get_preset(&app_data_dir, &device_id, &preset_name)
}

#[tauri::command]
fn save_global_eq_preset(
    device_id: String,
    device_name: String,
    preset_name: String,
    config: GlobalEqConfig,
    app: tauri::AppHandle,
) -> Result<GlobalEqConfig, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::save_preset(
        &app_data_dir,
        &device_id,
        &device_name,
        &preset_name,
        config,
    )
}

#[tauri::command]
fn activate_global_eq_preset(
    device_id: String,
    preset_name: String,
    app: tauri::AppHandle,
) -> Result<GlobalEqConfig, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::activate_preset(&app_data_dir, &device_id, &preset_name)
}

#[tauri::command]
fn delete_global_eq_preset(
    device_id: String,
    preset_name: String,
    app: tauri::AppHandle,
) -> Result<EqPresetCatalog, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::delete_preset(&app_data_dir, &device_id, &preset_name)
}

#[tauri::command]
fn connect_equalizer_apo(app: tauri::AppHandle) -> Result<EqualizerApoStatus, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::connect(&app_data_dir)
}

#[tauri::command]
fn disconnect_equalizer_apo(app: tauri::AppHandle) -> Result<EqualizerApoStatus, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::disconnect(&app_data_dir)
}

#[tauri::command]
fn open_equalizer_apo_download() -> Result<(), String> {
    equalizer_apo::open_download_page()
}

#[tauri::command]
fn open_equalizer_apo_configurator() -> Result<(), String> {
    equalizer_apo::open_configurator()
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
) -> Result<ProcessCaptureStatus, String> {
    let output_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定录制目录: {error}"))?
        .join("captures");
    manager.start(pid, &output_dir)
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_default_device_id,
            get_default_device_name,
            enumerate_devices,
            enumerate_sessions,
            set_session_volume,
            set_session_mute,
            get_device_volume,
            set_device_volume,
            set_device_mute,
            get_autostart_enabled,
            set_autostart_enabled,
            set_default_device,
            set_app_output_device,
            open_sound_settings,
            equalizer_apo_status,
            equalizer_apo_enabled_devices,
            choose_rnnoise_plugin_directory,
            get_global_eq,
            set_global_eq,
            get_microphone_processing,
            set_microphone_processing,
            list_global_eq_presets,
            get_global_eq_preset,
            save_global_eq_preset,
            activate_global_eq_preset,
            delete_global_eq_preset,
            connect_equalizer_apo,
            disconnect_equalizer_apo,
            open_equalizer_apo_download,
            open_equalizer_apo_configurator,
            audio_notifications_available,
            process_loopback_support,
            process_capture_status,
            start_process_capture,
            stop_process_capture,
            reveal_capture_file,
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
