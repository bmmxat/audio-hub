//! 系统托盘：常驻图标、未聚焦静音快捷控制、默认输出/麦克风切换与退出。

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use tauri::{
    AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};

use crate::audio::{
    device::DeviceDirection,
    notifications::{DEVICES_CHANGED_EVENT, SESSION_CHANGED_EVENT},
    process_loopback::ProcessCaptureManager,
    simple_route::SimpleRouteManager,
    wasapi,
};
use crate::plugins::voicemeeter::VoicemeeterManager;
use crate::unfocused_mute::{UNFOCUSED_MUTE_CHANGED_EVENT, UnfocusedMuteManager};

const TRAY_ICON_ID: &str = "audio-hub-tray";
const TRAY_OPEN_ID: &str = "tray-open";
const TRAY_UNFOCUSED_MUTE_ID: &str = "tray-unfocused-mute";
const TRAY_DEVICE_MENU_ID: &str = "tray-devices";
const TRAY_DEVICE_PREFIX: &str = "tray-device:";
const TRAY_DEVICE_PLACEHOLDER_ID: &str = "tray-device:none";
const TRAY_INPUT_MENU_ID: &str = "tray-input-devices";
const TRAY_INPUT_PREFIX: &str = "tray-input:";
const TRAY_INPUT_PLACEHOLDER_ID: &str = "tray-input:none";
const TRAY_QUIT_ID: &str = "tray-quit";

/// 保存托盘图标实例并记录可用状态，防止图标被提前释放。
#[derive(Default)]
pub struct TrayState {
    icon: Mutex<Option<TrayIcon<tauri::Wry>>>,
    available: AtomicBool,
}

/// 托盘是否已成功创建（决定最小化按钮隐藏窗口还是正常最小化）。
pub fn is_available(app: &AppHandle) -> bool {
    app.state::<TrayState>().available.load(Ordering::Relaxed)
}

/// 创建托盘图标与初始菜单。
pub fn build(app: &AppHandle) -> Result<(), String> {
    let menu = build_menu(app).map_err(|error| format!("无法创建托盘菜单：{error}"))?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "无法加载托盘图标。".to_string())?;

    let tray = TrayIconBuilder::with_id(TRAY_ICON_ID)
        .icon(icon)
        .menu(&menu)
        .tooltip("Audio Hub")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            handle_menu_event(app, event.id().as_ref());
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|error| format!("无法创建系统托盘图标：{error}"))?;

    let state = app.state::<TrayState>();
    let mut icon_slot = state
        .icon
        .lock()
        .map_err(|_| "托盘状态已损坏。".to_string())?;
    *icon_slot = Some(tray);
    state.available.store(true, Ordering::Release);
    Ok(())
}

/// 根据当前音频状态重建托盘菜单。
pub fn refresh(app: &AppHandle) {
    if !is_available(app) {
        return;
    }
    let Ok(menu) = build_menu(app) else {
        return;
    };
    let Some(tray) = app
        .state::<TrayState>()
        .icon
        .lock()
        .ok()
        .and_then(|icon| icon.clone())
    else {
        return;
    };
    if let Err(error) = tray.set_menu(Some(menu)) {
        eprintln!("刷新托盘菜单失败：{error}");
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let open_item = MenuItem::with_id(app, TRAY_OPEN_ID, "打开 Audio Hub", true, None::<&str>)?;

    let unfocused_mute = app.state::<UnfocusedMuteManager>().status();
    let unfocused_mute_count = unfocused_mute.applications.len();
    let unfocused_mute_text = if unfocused_mute_count == 0 {
        "未聚焦静音：未配置".to_string()
    } else if unfocused_mute.paused {
        format!("未聚焦静音：已暂停（{unfocused_mute_count} 个应用）")
    } else {
        format!("未聚焦静音：运行中（{unfocused_mute_count} 个应用）")
    };
    let unfocused_mute_item = MenuItem::with_id(
        app,
        TRAY_UNFOCUSED_MUTE_ID,
        unfocused_mute_text,
        unfocused_mute_count > 0,
        None::<&str>,
    )?;

    let output_menu = build_device_menu(
        app,
        DeviceDirection::Output,
        TRAY_DEVICE_MENU_ID,
        "默认输出设备",
        TRAY_DEVICE_PREFIX,
        TRAY_DEVICE_PLACEHOLDER_ID,
    )?;
    let input_menu = build_device_menu(
        app,
        DeviceDirection::Input,
        TRAY_INPUT_MENU_ID,
        "默认麦克风",
        TRAY_INPUT_PREFIX,
        TRAY_INPUT_PLACEHOLDER_ID,
    )?;

    let quit_item = MenuItem::with_id(app, TRAY_QUIT_ID, "退出 Audio Hub", true, None::<&str>)?;

    let menu = Menu::new(app)?;
    menu.append_items(&[
        &open_item,
        &PredefinedMenuItem::separator(app)?,
        &unfocused_mute_item,
        &output_menu,
        &input_menu,
        &PredefinedMenuItem::separator(app)?,
        &quit_item,
    ])?;
    Ok(menu)
}

fn build_device_menu(
    app: &AppHandle,
    direction: DeviceDirection,
    menu_id: &str,
    title: &str,
    prefix: &str,
    placeholder_id: &str,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let submenu = Submenu::with_id(app, menu_id, title, true)?;
    let devices = match wasapi::enumerate_devices(direction) {
        Ok(devices) => devices,
        Err(error) => {
            let item = MenuItem::with_id(
                app,
                placeholder_id,
                format!("读取设备失败：{error}"),
                false,
                None::<&str>,
            )?;
            submenu.append(&item)?;
            return Ok(submenu);
        }
    };

    if devices.is_empty() {
        let item = MenuItem::with_id(app, placeholder_id, "未检测到输出设备", false, None::<&str>)?;
        submenu.append(&item)?;
        return Ok(submenu);
    }

    for device in devices {
        let marker = if device.is_default { " ✓" } else { "" };
        let item = MenuItem::with_id(
            app,
            format!("{prefix}{}", device.device_id),
            format!("{}{}", device.name, marker),
            true,
            None::<&str>,
        )?;
        submenu.append(&item)?;
    }
    Ok(submenu)
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        TRAY_OPEN_ID => show_main_window(app),
        TRAY_UNFOCUSED_MUTE_ID => toggle_unfocused_mute(app),
        TRAY_QUIT_ID => quit(app),
        _ if id.starts_with(TRAY_DEVICE_PREFIX) && id != TRAY_DEVICE_PLACEHOLDER_ID => {
            let device_id = id.trim_start_matches(TRAY_DEVICE_PREFIX);
            switch_default_device(app, device_id);
        }
        _ if id.starts_with(TRAY_INPUT_PREFIX) && id != TRAY_INPUT_PLACEHOLDER_ID => {
            let device_id = id.trim_start_matches(TRAY_INPUT_PREFIX);
            switch_default_device(app, device_id);
        }
        _ => {}
    }
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_unfocused_mute(app: &AppHandle) {
    app.state::<UnfocusedMuteManager>().toggle_paused();
    let _ = app.emit(SESSION_CHANGED_EVENT, ());
    let _ = app.emit(UNFOCUSED_MUTE_CHANGED_EVENT, ());
    refresh_after_menu_event(app);
}

fn switch_default_device(app: &AppHandle, device_id: &str) {
    let app = app.clone();
    let device_id = device_id.to_string();
    std::thread::spawn(move || match wasapi::set_default_device(&device_id) {
        Ok(()) => {
            let _ = app.emit(DEVICES_CHANGED_EVENT, ());
        }
        Err(error) => {
            eprintln!("从托盘切换默认设备失败：{error:?}");
        }
    });
}

/// 等原生菜单事件返回后再替换菜单，避免当前菜单项在点击过程中失效。
fn refresh_after_menu_event(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(80));
        refresh(&app);
    });
}

fn quit(app: &AppHandle) {
    let capture = app.state::<ProcessCaptureManager>();
    if capture.status().active {
        let _ = capture.stop();
    }
    app.state::<UnfocusedMuteManager>().restore_all();
    if let Err(error) = app
        .state::<SimpleRouteManager>()
        .stop_all(&app.state::<VoicemeeterManager>())
    {
        eprintln!("退出时恢复简易流转失败：{error}");
    }
    app.exit(0);
}
