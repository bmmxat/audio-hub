use windows::{
    core::*,
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::*,
        System::{
            Com::{
                StructuredStorage::PROPVARIANT, *,
            },
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32,
                TH32CS_SNAPPROCESS,
            },
        },
        UI::Shell::PropertiesSystem::IPropertyStore,
    },
};

use super::device::{AudioDevice, AudioSession, DeviceDirection};

// ── COM 生命周期封装 ──────────────────────────────────────────

/// 初始化 COM，执行闭包，然后反初始化 COM。
///
/// # Safety
///
/// 闭包内的 COM 对象必须在 `CoUninitialize` 执行前全部 Drop。
/// 此函数利用作用域边界保证正确的 Drop 顺序。
unsafe fn with_com<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = f();

    unsafe {
        CoUninitialize();
    }

    result
}

// ── 内部辅助函数 ──────────────────────────────────────────────

/// 创建 `IMMDeviceEnumerator` 实例。
fn create_device_enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

/// 获取默认渲染设备（`IMMDevice`）。
fn get_default_render_device() -> Result<IMMDevice> {
    let enumerator = create_device_enumerator()?;
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
}

/// 获取默认渲染设备的端点 ID。
fn get_default_render_device_id() -> Result<String> {
    let device = get_default_render_device()?;
    let id = unsafe { device.GetId()? };
    Ok(unsafe { id.to_string()? })
}

/// 获取默认捕获设备（麦克风）的端点 ID。
fn get_default_capture_device_id() -> Result<String> {
    let enumerator = create_device_enumerator()?;
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) }?;
    let id = unsafe { device.GetId()? };
    Ok(unsafe { id.to_string()? })
}

/// 根据数据流方向获取对应的默认设备 ID。
#[allow(non_upper_case_globals)]
fn get_default_device_id_for_flow(dataflow: EDataFlow) -> Option<String> {
    match dataflow {
        eRender => get_default_render_device_id().ok(),
        eCapture => get_default_capture_device_id().ok(),
        _ => None,
    }
}

/// 从 `IMMDevice` 提取友好名称。
///
/// 调用链：`IMMDevice → OpenPropertyStore → GetValue(PKEY_Device_FriendlyName)`
fn get_device_friendly_name(device: &IMMDevice) -> Result<String> {
    let property_store: IPropertyStore =
        unsafe { device.OpenPropertyStore(STGM_READ)? };

    let prop_variant: PROPVARIANT =
        unsafe { property_store.GetValue(&PKEY_Device_FriendlyName)? };

    let pwsz_val =
        unsafe { prop_variant.Anonymous.Anonymous.Anonymous.pwszVal };

    if pwsz_val.is_null() {
        Err(Error::from_hresult(HRESULT::from_win32(0x8000_FFFF)))
    } else {
        Ok(unsafe { pwsz_val.to_string()? })
    }
}

/// 从 `IMMDevice` 获取端点 ID 字符串。
fn get_device_id(device: &IMMDevice) -> Result<String> {
    let id = unsafe { device.GetId()? };
    Ok(unsafe { id.to_string()? })
}

// ── 公开 API ──────────────────────────────────────────────────

/// 获取默认渲染设备的端点 ID 字符串（基于 GUID 的标识符）。
pub fn get_default_device_id() -> Result<String> {
    unsafe {
        with_com(|| get_default_render_device_id())
    }
}

/// 获取默认渲染设备的用户友好名称（例如 "扬声器 (EDIFIER M230)"）。
pub fn get_default_device_friendly_name() -> Result<String> {
    unsafe {
        with_com(|| {
            let device = get_default_render_device()?;
            get_device_friendly_name(&device)
        })
    }
}

/// 枚举指定方向的所有音频设备。
///
/// # 参数
///
/// * `direction` - 设备方向：`Output`（输出/扬声器）或 `Input`（输入/麦克风）
///
/// # 返回
///
/// 返回 `Vec<AudioDevice>`，其中 `is_default` 标记了当前系统默认设备。
pub fn enumerate_devices(direction: DeviceDirection) -> Result<Vec<AudioDevice>> {
    let dataflow = match direction {
        DeviceDirection::Output => eRender,
        DeviceDirection::Input => eCapture,
    };

    unsafe { enumerate_devices_impl(dataflow) }
}

/// 实际枚举逻辑。
unsafe fn enumerate_devices_impl(dataflow: EDataFlow) -> Result<Vec<AudioDevice>> {
    unsafe {
        with_com(|| {
            let enumerator = create_device_enumerator()?;

            // 根据数据流方向获取对应的默认设备 ID
            let default_id = get_default_device_id_for_flow(dataflow);

            // 枚举指定方向的所有活跃设备
            let collection = enumerator.EnumAudioEndpoints(
                dataflow,
                DEVICE_STATE_ACTIVE,
            )?;

            let count = collection.GetCount()?;
            let mut devices = Vec::with_capacity(count as usize);

            for i in 0..count {
                let device = collection.Item(i)?;

                // 获取设备 ID
                let device_id = get_device_id(&device)?;

                // 获取友好名称；失败时回退到设备 ID
                let name = get_device_friendly_name(&device)
                    .unwrap_or_else(|_| device_id.clone());

                let is_default = default_id.as_ref() == Some(&device_id);

                devices.push(AudioDevice {
                    name,
                    device_id,
                    is_default,
                });
            }

            Ok(devices)
        })
    }
}

// ── 会话管理（Phase 4）───────────────────────────────────────

/// 从 PID 获取进程的可执行文件名（例如 "League of Legends.exe"）。
///
/// 使用 `CreateToolhelp32Snapshot` 遍历进程快照，无需打开进程句柄——
/// 因此对受保护进程（反作弊、管理员权限）同样有效。
fn get_process_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }

    unsafe {
        // 创建系统进程快照（不需要打开目标进程句柄）
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;

        let mut entry = PROCESSENTRY32 {
            dwSize: std::mem::size_of::<PROCESSENTRY32>() as u32,
            ..Default::default()
        };

        // 遍历快照，查找匹配的 PID
        if Process32First(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32ProcessID == pid {
                    // szExeFile 是 ANSI 编码的 [i8; 260]，找到 \0 截断
                    let end = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let bytes: Vec<u8> =
                        entry.szExeFile[..end].iter().map(|&c| c as u8).collect();
                    let name = String::from_utf8_lossy(&bytes).to_string();
                    let _ = windows::Win32::Foundation::CloseHandle(snapshot);
                    return Some(name);
                }
                if Process32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        None
    }
}

/// 枚举所有输出设备上的音频会话。
///
/// 遍历每个活跃的渲染设备，汇总其上所有音频会话。
/// 这对应 Windows 音量混合器中显示的应用列表（跨所有设备）。
///
/// # 返回
///
/// 返回 `Vec<AudioSession>`，包含每个音频会话的显示名称、PID、音量和静音状态。
pub fn enumerate_sessions() -> Result<Vec<AudioSession>> {
    unsafe {
        with_com(|| {
            let enumerator = create_device_enumerator()?;

            // 枚举所有活跃的输出设备
            let devices = enumerator.EnumAudioEndpoints(
                eRender,
                DEVICE_STATE_ACTIVE,
            )?;

            let device_count = devices.GetCount()?;
            let mut all_sessions = Vec::new();

            for di in 0..device_count {
                let device = devices.Item(di)?;

                // 激活 IAudioSessionManager2；某些设备可能没有活跃会话
                let session_manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None)
                {
                    Ok(mgr) => mgr,
                    Err(_) => continue,
                };

                let session_enumerator = match session_manager.GetSessionEnumerator()
                {
                    Ok(se) => se,
                    Err(_) => continue,
                };

                let count = session_enumerator.GetCount()?;

                for si in 0..count {
                    let session_ctrl = match session_enumerator.GetSession(si) {
                        Ok(sc) => sc,
                        Err(_) => continue,
                    };

                    // 获取 PID
                    let pid = session_ctrl
                        .cast::<IAudioSessionControl2>()
                        .and_then(|ctrl2| ctrl2.GetProcessId())
                        .unwrap_or(0);

                    // 获取显示名称，优先级：
                    // 1. IAudioSessionControl::GetDisplayName()
                    // 2. ToolHelp 快照反查进程名（无权限限制）
                    // 3. 系统音效特殊标记
                    // 以上都无法获取 → 跳过
                    let raw_name = session_ctrl.GetDisplayName().ok();
                    let display_name = raw_name
                        .and_then(|pwstr| pwstr.to_string().ok())
                        .filter(|s| !s.is_empty() && !s.starts_with('@'))
                        .or_else(|| get_process_name(pid))
                        .or_else(|| {
                            if pid == 0 {
                                Some("系统音效".to_string())
                            } else {
                                None
                            }
                        });

                    let display_name = match display_name {
                        Some(name) => name,
                        None => continue,
                    };

                    // 获取音量和静音状态
                    let (volume, muted) = session_ctrl
                        .cast::<ISimpleAudioVolume>()
                        .map(|vol| {
                            let v = vol.GetMasterVolume().unwrap_or(1.0);
                            let m = vol.GetMute().unwrap_or(BOOL(0));
                            (v, m.as_bool())
                        })
                        .unwrap_or((1.0, false));

                    all_sessions.push(AudioSession {
                        display_name,
                        pid,
                        volume,
                        muted,
                    });
                }
            }

            // 去重：同一 (PID, 显示名称) 只保留一个。
            // 系统音效 (PID 0) 在每个设备上各有一份，只需保留一份。
            let mut seen = std::collections::HashSet::new();
            all_sessions.retain(|s| seen.insert((s.pid, s.display_name.clone())));

            Ok(all_sessions)
        })
    }
}

// ── 音量控制（Phase 5）───────────────────────────────────

/// 设置指定 PID 在所有输出设备上的音量。
///
/// 遍历所有活跃会话，对匹配 PID 的会话调用 `ISimpleAudioVolume::SetMasterVolume`。
/// 同一 PID 可能在多个设备上有会话——全部同步调整。
///
/// # 参数
///
/// * `pid`  - 目标进程 ID
/// * `volume` - 音量值（0.0 ~ 1.0）
pub fn set_session_volume(pid: u32, volume: f32) -> Result<()> {
    let volume = volume.clamp(0.0, 1.0);

    unsafe {
        with_com(|| {
            for_each_session_by_pid(pid, |vol| {
                vol.SetMasterVolume(volume, std::ptr::null()).ok();
            });
            Ok(())
        })
    }
}

/// 设置指定 PID 在所有输出设备上的静音状态。
///
/// 遍历所有活跃会话，对匹配 PID 的会话调用 `ISimpleAudioVolume::SetMute`。
///
/// # 参数
///
/// * `pid`   - 目标进程 ID
/// * `muted` - true=静音，false=取消静音
pub fn set_session_mute(pid: u32, muted: bool) -> Result<()> {
    unsafe {
        with_com(|| {
            for_each_session_by_pid(pid, |vol| {
                vol.SetMute(muted, std::ptr::null()).ok();
            });
            Ok(())
        })
    }
}

/// 遍历所有输出设备上的会话，对匹配 PID 的会话执行回调。
unsafe fn for_each_session_by_pid(target_pid: u32, mut f: impl FnMut(&ISimpleAudioVolume)) {
    let Some(enumerator) = create_device_enumerator().ok() else {
        return;
    };

    let Some(devices) =
        (unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }).ok()
    else {
        return;
    };

    let Some(device_count) = (unsafe { devices.GetCount() }).ok() else {
        return;
    };

    for di in 0..device_count {
        let Some(device) = (unsafe { devices.Item(di) }).ok() else {
            continue;
        };

        let Some(session_manager) =
            (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }).ok()
        else {
            continue;
        };

        let Some(session_enumerator) =
            (unsafe { session_manager.GetSessionEnumerator() }).ok()
        else {
            continue;
        };

        let Some(count) = (unsafe { session_enumerator.GetCount() }).ok() else {
            continue;
        };

        for si in 0..count {
            let Some(session_ctrl) = (unsafe { session_enumerator.GetSession(si) }).ok() else {
                continue;
            };

            let pid = session_ctrl
                .cast::<IAudioSessionControl2>()
                .and_then(|ctrl2| unsafe { ctrl2.GetProcessId() })
                .unwrap_or(0);

            if pid != target_pid {
                continue;
            }

            if let Ok(vol) = session_ctrl.cast::<ISimpleAudioVolume>() {
                f(&vol);
            }
        }
    }
}

// ── 默认设备切换（Phase 5+）───────────────────────────────

/// 将指定端点设置为默认设备（自动判断输出/输入方向）。
pub fn set_default_device(device_id: &str) -> Result<()> {
    // 从设备 ID 判断方向：{0.0.0.xxx} 开头的是输出，{0.0.1.xxx} 是输入
    let is_input = device_id.contains("{0.0.1.");
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let r = super::policy_config::set_default_endpoint(device_id, is_input);
        CoUninitialize();
        r
    }
}

/// 设置特定应用的输出设备（per-app 路由，Win10 1803+）。
pub fn set_app_output_device(pid: u32, device_id: &str) -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let r = super::policy_config::set_app_output_device(pid, device_id);
        CoUninitialize();
        r
    }
}

/// 打开 Windows 声音设置面板（`ms-settings:sound`）。
pub fn open_sound_settings() {
    std::process::Command::new("cmd")
        .args(["/c", "start", "ms-settings:sound"])
        .spawn()
        .ok();
}

// ── Profile 应用（Phase 6）────────────────────────────────

/// 将 Profile 中的音量/静音应用到当前活跃会话。
///
/// 匹配策略：先按 PID 精确匹配，PID 不存在时按 display_name 匹配
///（处理应用重启后 PID 变化的情况）。
pub fn apply_profile(profile: &super::profile::Profile) {
    for entry in &profile.entries {
        // 先尝试用 PID 直接匹配（最快、最精确）
        if apply_to_pid(entry.pid, entry.volume, entry.muted) {
            continue;
        }
        // PID 不存在——可能应用已重启，改用名称匹配
        if let Some(pid) = find_pid_by_name(&entry.display_name) {
            apply_to_pid(pid, entry.volume, entry.muted);
        }
    }
}

/// 设置指定 PID 的音量和静音。
fn apply_to_pid(pid: u32, volume: f32, muted: bool) -> bool {
    if pid == 0 {
        return false;
    }
    // 先设静音，再设音量（静音状态独立于音量值）
    let _ = set_session_mute(pid, muted);
    let _ = set_session_volume(pid, volume);
    true
}

/// 在所有活跃会话中按 display_name 查找 PID。
fn find_pid_by_name(name: &str) -> Option<u32> {
    let sessions = enumerate_sessions().ok()?;
    sessions
        .iter()
        .find(|s| s.display_name.eq_ignore_ascii_case(name))
        .map(|s| s.pid)
}

