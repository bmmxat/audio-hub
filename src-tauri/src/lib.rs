mod audio;
mod autostart;
mod plugins;
mod settings;
mod tray;
mod unfocused_mute;

use audio::{
    device::{AudioDevice, AudioSession, DeviceDirection, DeviceVolumeState},
    device_volume_follow::{
        DeviceVolumeFollowManager, DeviceVolumeFollowStatus, DeviceVolumeSnapshot,
    },
    notifications::{AudioNotificationWatcher, DEVICES_CHANGED_EVENT, SESSION_CHANGED_EVENT},
    process_loopback::{
        ProcessCaptureManager, ProcessCaptureResult, ProcessCaptureStatus, ProcessLoopbackSupport,
    },
    profile::{self, Profile},
    simple_route::{SimpleRouteManager, SimpleRouteStatus},
    wasapi,
};
use plugins::equalizer_apo::{
    self, ApoProcessingState, EqPresetCatalog, EqualizerApoStatus, GlobalEqConfig,
    MicrophoneConfig, MicrophoneConfigState,
};
use plugins::voicemeeter::{self, VoicemeeterConfiguration, VoicemeeterManager, VoicemeeterStatus};
use tauri::{Emitter, Listener, Manager};
use unfocused_mute::{UNFOCUSED_MUTE_CHANGED_EVENT, UnfocusedMuteManager, UnfocusedMuteStatus};

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
fn set_session_volume(pid: u32, volume: f32, device_id: Option<String>) -> Result<(), String> {
    match device_id.as_deref().filter(|value| !value.is_empty()) {
        Some(device_id) => wasapi::set_session_volume_for_device(device_id, pid, volume),
        None => wasapi::set_session_volume(pid, volume),
    }
    .map_err(|e| format!("{:?}", e))
}

/// 设置指定 PID 应用的静音状态。
#[tauri::command]
fn set_session_mute(pid: u32, muted: bool, device_id: Option<String>) -> Result<(), String> {
    match device_id.as_deref().filter(|value| !value.is_empty()) {
        Some(device_id) => wasapi::set_session_mute_for_device(device_id, pid, muted),
        None => wasapi::set_session_mute(pid, muted),
    }
    .map_err(|e| format!("{:?}", e))
}

#[tauri::command]
fn get_device_volume_follow_status(
    manager: tauri::State<'_, DeviceVolumeFollowManager>,
) -> DeviceVolumeFollowStatus {
    manager.status()
}

#[tauri::command]
fn configure_device_volume_follow(
    enabled: bool,
    legacy_snapshots: Option<std::collections::HashMap<String, DeviceVolumeSnapshot>>,
    manager: tauri::State<'_, DeviceVolumeFollowManager>,
    unfocused_mute: tauri::State<'_, UnfocusedMuteManager>,
) -> Result<DeviceVolumeFollowStatus, String> {
    let auto_muted_keys = unfocused_mute
        .status()
        .auto_muted_keys
        .into_iter()
        .collect();
    manager.configure(enabled, legacy_snapshots, &auto_muted_keys)
}

#[tauri::command]
fn capture_device_volume_snapshot(
    device_id: Option<String>,
    manager: tauri::State<'_, DeviceVolumeFollowManager>,
    unfocused_mute: tauri::State<'_, UnfocusedMuteManager>,
) -> Result<(), String> {
    let auto_muted_keys = unfocused_mute
        .status()
        .auto_muted_keys
        .into_iter()
        .collect();
    manager.capture_device(device_id.as_deref(), &auto_muted_keys)
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

/// 使用系统默认浏览器打开 Audio Hub 的 GitHub 项目主页。
#[tauri::command]
fn open_project_homepage() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg("https://github.com/bmmxat/audio-hub")
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开项目主页：{error}"))
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
fn equalizer_apo_processing_state(
    output_device_id: Option<String>,
    input_device_id: Option<String>,
    app: tauri::AppHandle,
) -> Result<ApoProcessingState, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定插件配置目录：{error}"))?;
    equalizer_apo::processing_state(
        &app_data_dir,
        output_device_id.as_deref(),
        input_device_id.as_deref(),
    )
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

#[tauri::command]
fn voicemeeter_status(manager: tauri::State<'_, VoicemeeterManager>) -> VoicemeeterStatus {
    manager.status()
}

#[tauri::command]
fn start_voicemeeter(
    manager: tauri::State<'_, VoicemeeterManager>,
) -> Result<VoicemeeterStatus, String> {
    manager.start()
}

#[tauri::command]
fn show_voicemeeter(
    manager: tauri::State<'_, VoicemeeterManager>,
) -> Result<VoicemeeterStatus, String> {
    manager.show()
}

#[tauri::command]
fn restart_voicemeeter_audio_engine(
    manager: tauri::State<'_, VoicemeeterManager>,
) -> Result<VoicemeeterStatus, String> {
    manager.restart_audio_engine()
}

#[tauri::command]
fn apply_voicemeeter_configuration(
    configuration: VoicemeeterConfiguration,
    manager: tauri::State<'_, VoicemeeterManager>,
    simple_route: tauri::State<'_, SimpleRouteManager>,
) -> Result<VoicemeeterStatus, String> {
    if simple_route.status().active {
        return Err("简易流转正在运行，请先停止全部简易流转再修改高级路由。".to_string());
    }
    manager.apply(configuration)
}

#[tauri::command]
fn open_voicemeeter_download() -> Result<(), String> {
    voicemeeter::open_download_page()
}

#[tauri::command]
fn simple_route_status(manager: tauri::State<'_, SimpleRouteManager>) -> SimpleRouteStatus {
    manager.status()
}

#[tauri::command]
fn prepare_simple_route(
    manager: tauri::State<'_, SimpleRouteManager>,
    voicemeeter: tauri::State<'_, VoicemeeterManager>,
) -> Result<SimpleRouteStatus, String> {
    manager.prepare(&voicemeeter)
}

#[tauri::command]
fn enable_simple_route_application(
    pid: u32,
    key: String,
    display_name: String,
    manager: tauri::State<'_, SimpleRouteManager>,
    voicemeeter: tauri::State<'_, VoicemeeterManager>,
) -> Result<SimpleRouteStatus, String> {
    manager.enable_application(pid, key, display_name, &voicemeeter)
}

#[tauri::command]
fn disable_simple_route_application(
    key: String,
    current_pid: Option<u32>,
    manager: tauri::State<'_, SimpleRouteManager>,
) -> Result<SimpleRouteStatus, String> {
    manager.disable_application(&key, current_pid)
}

#[tauri::command]
fn stop_all_simple_routes(
    manager: tauri::State<'_, SimpleRouteManager>,
    voicemeeter: tauri::State<'_, VoicemeeterManager>,
) -> Result<SimpleRouteStatus, String> {
    manager.stop_all(&voicemeeter)
}

#[tauri::command]
fn shutdown_voicemeeter(
    manager: tauri::State<'_, VoicemeeterManager>,
    simple_route: tauri::State<'_, SimpleRouteManager>,
) -> Result<VoicemeeterStatus, String> {
    simple_route.stop_all(&manager)?;
    manager.shutdown()
}

#[tauri::command]
fn sync_simple_route_monitor(
    manager: tauri::State<'_, SimpleRouteManager>,
    voicemeeter: tauri::State<'_, VoicemeeterManager>,
) -> Result<SimpleRouteStatus, String> {
    manager.sync_monitor_to_default(&voicemeeter)
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
fn win_hide_to_tray(window: tauri::Window, app: tauri::AppHandle) {
    if tray::is_available(&app) {
        let _ = window.hide();
    } else {
        let _ = window.minimize();
    }
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
fn win_close(
    window: tauri::Window,
    capture_manager: tauri::State<'_, ProcessCaptureManager>,
    unfocused_mute_manager: tauri::State<'_, UnfocusedMuteManager>,
    simple_route_manager: tauri::State<'_, SimpleRouteManager>,
    voicemeeter_manager: tauri::State<'_, VoicemeeterManager>,
) {
    if capture_manager.status().active {
        let _ = capture_manager.stop();
    }
    unfocused_mute_manager.restore_all();
    if let Err(error) = simple_route_manager.stop_all(&voicemeeter_manager) {
        eprintln!("退出时恢复简易流转失败：{error}");
    }
    let _ = window.close();
}

/// 关闭按钮行为（右上角 X）：由前端在首次询问后决定隐藏到托盘或退出。
#[tauri::command]
fn get_close_behavior(app: tauri::AppHandle) -> Result<settings::CloseBehaviorState, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定配置目录：{error}"))?;
    let current = settings::load(&app_data_dir);
    Ok(settings::CloseBehaviorState {
        behavior: current.close_behavior,
        chosen: current.close_behavior_chosen,
    })
}

#[tauri::command]
fn set_close_behavior(
    behavior: settings::CloseBehavior,
    app: tauri::AppHandle,
) -> Result<settings::CloseBehaviorState, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定配置目录：{error}"))?;
    let mut current = settings::load(&app_data_dir);
    current.close_behavior = behavior;
    current.close_behavior_chosen = true;
    settings::save(&app_data_dir, &current)?;
    Ok(settings::CloseBehaviorState {
        behavior: current.close_behavior,
        chosen: current.close_behavior_chosen,
    })
}

/// 获取未聚焦自动静音列表及当前运行状态。
#[tauri::command]
fn get_unfocused_mute_status(
    manager: tauri::State<'_, UnfocusedMuteManager>,
) -> UnfocusedMuteStatus {
    manager.status()
}

/// 添加或移除一个未聚焦自动静音应用。
#[tauri::command]
fn set_unfocused_mute_application(
    key: String,
    display_name: String,
    enabled: bool,
    manager: tauri::State<'_, UnfocusedMuteManager>,
    app: tauri::AppHandle,
) -> Result<UnfocusedMuteStatus, String> {
    let status = manager.set_application(key, display_name, enabled)?;
    let _ = app.emit(SESSION_CHANGED_EVENT, ());
    let _ = app.emit(UNFOCUSED_MUTE_CHANGED_EVENT, ());
    tray::refresh(&app);
    Ok(status)
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
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .setup(|app| {
            app.manage(ProcessCaptureManager::default());
            app.manage(VoicemeeterManager::default());
            app.manage(tray::TrayState::default());
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("无法确定简易流转配置目录：{error}"))?;
            app.manage(SimpleRouteManager::load(&app_data_dir));
            app.manage(DeviceVolumeFollowManager::load(&app_data_dir));
            app.manage(UnfocusedMuteManager::start(
                app.handle().clone(),
                app_data_dir,
            ));
            app.manage(AudioNotificationWatcher::start(app.handle().clone()));

            if let Err(error) = tray::build(app.handle()) {
                eprintln!("系统托盘初始化失败，最小化将退回任务栏窗口：{error}");
            }

            let tray_app = app.handle().clone();
            app.listen(DEVICES_CHANGED_EVENT, move |_| {
                tray::refresh(&tray_app);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                window.state::<UnfocusedMuteManager>().restore_all();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_default_device_id,
            get_default_device_name,
            enumerate_devices,
            enumerate_sessions,
            set_session_volume,
            set_session_mute,
            get_device_volume_follow_status,
            configure_device_volume_follow,
            capture_device_volume_snapshot,
            get_device_volume,
            set_device_volume,
            set_device_mute,
            get_autostart_enabled,
            set_autostart_enabled,
            set_default_device,
            set_app_output_device,
            open_sound_settings,
            open_project_homepage,
            equalizer_apo_status,
            equalizer_apo_enabled_devices,
            equalizer_apo_processing_state,
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
            voicemeeter_status,
            start_voicemeeter,
            show_voicemeeter,
            restart_voicemeeter_audio_engine,
            shutdown_voicemeeter,
            apply_voicemeeter_configuration,
            open_voicemeeter_download,
            simple_route_status,
            prepare_simple_route,
            enable_simple_route_application,
            disable_simple_route_application,
            stop_all_simple_routes,
            sync_simple_route_monitor,
            audio_notifications_available,
            process_loopback_support,
            process_capture_status,
            start_process_capture,
            stop_process_capture,
            reveal_capture_file,
            win_minimize,
            win_hide_to_tray,
            win_toggle_maximize,
            win_close,
            get_close_behavior,
            set_close_behavior,
            get_unfocused_mute_status,
            set_unfocused_mute_application,
            save_profile,
            load_profile,
            list_profiles,
            delete_profile,
            apply_profile,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Audio Hub 失败");
}
