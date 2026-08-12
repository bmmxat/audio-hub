use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Foundation::{CloseHandle, RPC_E_CHANGED_MODE},
        Media::Audio::{Endpoints::IAudioEndpointVolume, *},
        System::{
            Com::{StructuredStorage::PROPVARIANT, *},
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
        },
        UI::Shell::PropertiesSystem::IPropertyStore,
    },
    core::*,
};

use super::device::{AudioDevice, AudioSession, DeviceDirection, DeviceVolumeState};

// ── COM 生命周期封装 ──────────────────────────────────────────

/// 当前线程的 COM apartment 守卫。
///
/// 仅在 `CoInitializeEx` 成功（包括 `S_FALSE`）后创建，
/// 从而确保 `CoUninitialize` 始终与成功的初始化调用配对。
struct ComApartment {
    should_uninitialize: bool,
}

impl ComApartment {
    fn initialize(mode: COINIT) -> Result<Self> {
        let result = unsafe { CoInitializeEx(None, mode) };
        if result == RPC_E_CHANGED_MODE {
            // 线程已由宿主初始化为另一种 apartment；沿用现有模式，
            // 但不能替宿主调用 CoUninitialize。
            return Ok(Self {
                should_uninitialize: false,
            });
        }
        result.ok()?;
        Ok(Self {
            should_uninitialize: true,
        })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.should_uninitialize {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

/// 在 MTA 中执行闭包。闭包创建的 COM 对象会先于 apartment 守卫释放。
fn with_com<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let _apartment = ComApartment::initialize(COINIT_MULTITHREADED)?;
    f()
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
    let property_store: IPropertyStore = unsafe { device.OpenPropertyStore(STGM_READ)? };

    let prop_variant: PROPVARIANT = unsafe { property_store.GetValue(&PKEY_Device_FriendlyName)? };

    let pwsz_val = unsafe { prop_variant.Anonymous.Anonymous.Anonymous.pwszVal };

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

/// 根据端点 ID 获取设备。
fn get_device_by_id(enumerator: &IMMDeviceEnumerator, device_id: &str) -> Result<IMMDevice> {
    let wide_id: Vec<u16> = OsStr::new(device_id).encode_wide().chain(Some(0)).collect();
    unsafe { enumerator.GetDevice(PCWSTR(wide_id.as_ptr())) }
}

/// 激活设备级主音量接口。
fn get_endpoint_volume(device: &IMMDevice) -> Result<IAudioEndpointVolume> {
    unsafe { device.Activate(CLSCTX_ALL, None) }
}

// ── 公开 API ──────────────────────────────────────────────────

/// 获取默认渲染设备的端点 ID 字符串（基于 GUID 的标识符）。
pub fn get_default_device_id() -> Result<String> {
    with_com(get_default_render_device_id)
}

/// 获取默认渲染设备的用户友好名称（例如 "扬声器 (EDIFIER M230)"）。
pub fn get_default_device_friendly_name() -> Result<String> {
    with_com(|| {
        let device = get_default_render_device()?;
        get_device_friendly_name(&device)
    })
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

    enumerate_devices_impl(dataflow)
}

/// 获取指定输出或输入端点的 Windows 主音量。
pub fn get_device_volume(device_id: &str) -> Result<DeviceVolumeState> {
    with_com(|| {
        let enumerator = create_device_enumerator()?;
        let device = get_device_by_id(&enumerator, device_id)?;
        let endpoint_volume = get_endpoint_volume(&device)?;
        let volume = unsafe { endpoint_volume.GetMasterVolumeLevelScalar()? };
        let muted = unsafe { endpoint_volume.GetMute()? }.as_bool();
        Ok(DeviceVolumeState { volume, muted })
    })
}

/// 设置指定输出或输入端点的 Windows 主音量。
pub fn set_device_volume(device_id: &str, volume: f32) -> Result<DeviceVolumeState> {
    if !volume.is_finite() {
        return Err(Error::new(
            HRESULT::from_win32(87),
            "设备音量必须是有效数字",
        ));
    }
    let volume = volume.clamp(0.0, 1.0);
    with_com(|| {
        let enumerator = create_device_enumerator()?;
        let device = get_device_by_id(&enumerator, device_id)?;
        let endpoint_volume = get_endpoint_volume(&device)?;
        unsafe {
            endpoint_volume.SetMasterVolumeLevelScalar(
                volume,
                &super::notifications::AUDIO_HUB_EVENT_CONTEXT,
            )?;
            if volume > 0.0 && endpoint_volume.GetMute()?.as_bool() {
                endpoint_volume.SetMute(false, &super::notifications::AUDIO_HUB_EVENT_CONTEXT)?;
            }
            Ok(DeviceVolumeState {
                volume: endpoint_volume.GetMasterVolumeLevelScalar()?,
                muted: endpoint_volume.GetMute()?.as_bool(),
            })
        }
    })
}

/// 设置指定输出或输入端点的静音状态。
pub fn set_device_mute(device_id: &str, muted: bool) -> Result<DeviceVolumeState> {
    with_com(|| {
        let enumerator = create_device_enumerator()?;
        let device = get_device_by_id(&enumerator, device_id)?;
        let endpoint_volume = get_endpoint_volume(&device)?;
        unsafe {
            endpoint_volume.SetMute(muted, &super::notifications::AUDIO_HUB_EVENT_CONTEXT)?;
            Ok(DeviceVolumeState {
                volume: endpoint_volume.GetMasterVolumeLevelScalar()?,
                muted: endpoint_volume.GetMute()?.as_bool(),
            })
        }
    })
}

/// 实际枚举逻辑。
fn enumerate_devices_impl(dataflow: EDataFlow) -> Result<Vec<AudioDevice>> {
    with_com(|| unsafe {
        let enumerator = create_device_enumerator()?;

        // 根据数据流方向获取对应的默认设备 ID
        let default_id = get_default_device_id_for_flow(dataflow);

        // 枚举指定方向的所有活跃设备
        let collection = enumerator.EnumAudioEndpoints(dataflow, DEVICE_STATE_ACTIVE)?;

        let count = collection.GetCount()?;
        let mut devices = Vec::with_capacity(count as usize);

        for i in 0..count {
            let device = collection.Item(i)?;

            // 获取设备 ID
            let device_id = get_device_id(&device)?;

            // 获取友好名称；失败时回退到设备 ID
            let name = get_device_friendly_name(&device).unwrap_or_else(|_| device_id.clone());

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

// ── 会话管理（Phase 4）───────────────────────────────────────

/// 从 PID 获取进程的可执行文件名（例如 "League of Legends.exe"）。
///
/// 使用 `CreateToolhelp32Snapshot` 遍历进程快照，无需打开进程句柄——
/// 因此对受保护进程（反作弊、管理员权限）同样有效。
fn snapshot_process_names() -> std::collections::HashMap<u32, String> {
    let mut names = std::collections::HashMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return names;
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                if entry.th32ProcessID != 0 {
                    names.insert(
                        entry.th32ProcessID,
                        String::from_utf16_lossy(&entry.szExeFile[..end]),
                    );
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }
    names
}

/// 通过 ToolHelp 快照读取进程名，避免对受保护进程调用 `OpenProcess`。
pub(crate) fn process_name(pid: u32) -> Option<String> {
    snapshot_process_names().remove(&pid)
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
    with_com(|| unsafe {
        let enumerator = create_device_enumerator()?;
        let process_names = snapshot_process_names();
        let default_device_id = get_default_render_device_id().ok();

        // 枚举所有活跃的输出设备
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;

        let device_count = devices.GetCount()?;
        let mut all_sessions = Vec::new();

        for di in 0..device_count {
            let device = devices.Item(di)?;
            let device_id = get_device_id(&device)?;

            // 激活 IAudioSessionManager2；某些设备可能没有活跃会话
            let session_manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(mgr) => mgr,
                Err(_) => continue,
            };

            let session_enumerator = match session_manager.GetSessionEnumerator() {
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
                let process_name = process_names.get(&pid).cloned();
                let raw_name = session_ctrl.GetDisplayName().ok();
                let display_name = raw_name
                    .and_then(|pwstr| pwstr.to_string().ok())
                    .filter(|s| !s.is_empty() && !s.starts_with('@'))
                    .or_else(|| process_name.clone())
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
                    process_name,
                    device_id: device_id.clone(),
                    pid,
                    volume,
                    muted,
                });
            }
        }

        // 默认输出端点优先，使主界面展示和修改的都是当前扬声器会话；
        // 其他端点上独有的会话仍会保留，供路由和插件功能使用。
        deduplicate_sessions(&mut all_sessions, default_device_id.as_deref());

        Ok(all_sessions)
    })
}

fn deduplicate_sessions(sessions: &mut Vec<AudioSession>, default_device_id: Option<&str>) {
    sessions.sort_by_key(|session| default_device_id != Some(session.device_id.as_str()));
    // 同一 (PID, 显示名称) 只保留一个；系统音效在每个端点各有一份。
    let mut seen = std::collections::HashSet::new();
    sessions.retain(|session| seen.insert((session.pid, session.display_name.clone())));
}

/// 仅枚举指定输出端点上的音频会话，用于按扬声器保存独立音量。
pub fn enumerate_sessions_for_device(device_id: &str) -> Result<Vec<AudioSession>> {
    with_com(|| unsafe {
        let enumerator = create_device_enumerator()?;
        let device = get_device_by_id(&enumerator, device_id)?;
        let process_names = snapshot_process_names();
        let session_manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let session_enumerator = session_manager.GetSessionEnumerator()?;
        let count = session_enumerator.GetCount()?;
        let mut sessions = Vec::new();

        for index in 0..count {
            let session_ctrl = match session_enumerator.GetSession(index) {
                Ok(control) => control,
                Err(_) => continue,
            };
            let pid = session_ctrl
                .cast::<IAudioSessionControl2>()
                .and_then(|control| control.GetProcessId())
                .unwrap_or(0);
            let process_name = process_names.get(&pid).cloned();
            let display_name = session_ctrl
                .GetDisplayName()
                .ok()
                .and_then(|value| value.to_string().ok())
                .filter(|value| !value.is_empty() && !value.starts_with('@'))
                .or_else(|| process_name.clone())
                .or_else(|| (pid == 0).then(|| "系统音效".to_string()));
            let Some(display_name) = display_name else {
                continue;
            };
            let (volume, muted) = session_ctrl
                .cast::<ISimpleAudioVolume>()
                .map(|control| {
                    (
                        control.GetMasterVolume().unwrap_or(1.0),
                        control.GetMute().unwrap_or(BOOL(0)).as_bool(),
                    )
                })
                .unwrap_or((1.0, false));
            sessions.push(AudioSession {
                display_name,
                process_name,
                device_id: device_id.to_string(),
                pid,
                volume,
                muted,
            });
        }
        Ok(sessions)
    })
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
    let affected = with_com(|| unsafe {
        for_each_session_by_pid(pid, |vol| {
            vol.SetMasterVolume(volume, &super::notifications::AUDIO_HUB_EVENT_CONTEXT)
                .map(|_| ())
        })
    })?;
    if affected == 0 {
        Err(session_not_found(pid))
    } else {
        Ok(())
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
    let affected = with_com(|| unsafe {
        for_each_session_by_pid(pid, |vol| {
            vol.SetMute(muted, &super::notifications::AUDIO_HUB_EVENT_CONTEXT)
                .map(|_| ())
        })
    })?;
    if affected == 0 {
        Err(session_not_found(pid))
    } else {
        Ok(())
    }
}

/// 仅设置指定输出端点上匹配 PID 的会话音量。
pub fn set_session_volume_for_device(device_id: &str, pid: u32, volume: f32) -> Result<()> {
    let volume = volume.clamp(0.0, 1.0);
    let affected = with_com(|| unsafe {
        for_each_session_by_pid_on_device(device_id, pid, |control| {
            control
                .SetMasterVolume(volume, &super::notifications::AUDIO_HUB_EVENT_CONTEXT)
                .map(|_| ())
        })
    })?;
    if affected == 0 {
        Err(session_not_found(pid))
    } else {
        Ok(())
    }
}

/// 仅设置指定输出端点上匹配 PID 的会话静音状态。
pub fn set_session_mute_for_device(device_id: &str, pid: u32, muted: bool) -> Result<()> {
    let affected = with_com(|| unsafe {
        for_each_session_by_pid_on_device(device_id, pid, |control| {
            control
                .SetMute(muted, &super::notifications::AUDIO_HUB_EVENT_CONTEXT)
                .map(|_| ())
        })
    })?;
    if affected == 0 {
        Err(session_not_found(pid))
    } else {
        Ok(())
    }
}

fn session_not_found(pid: u32) -> Error {
    Error::new(
        HRESULT::from_win32(1168),
        format!("未找到 PID {pid} 对应的活跃音频会话"),
    )
}

/// 遍历所有输出设备上的会话，对匹配 PID 的会话执行回调。
unsafe fn for_each_session_by_pid(
    target_pid: u32,
    mut f: impl FnMut(&ISimpleAudioVolume) -> Result<()>,
) -> Result<usize> {
    let enumerator = create_device_enumerator()?;
    let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }?;
    let device_count = unsafe { devices.GetCount() }?;
    let mut affected = 0;

    for di in 0..device_count {
        let Ok(device) = (unsafe { devices.Item(di) }) else {
            continue;
        };

        let Ok(session_manager) =
            (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) })
        else {
            continue;
        };

        let Ok(session_enumerator) = (unsafe { session_manager.GetSessionEnumerator() }) else {
            continue;
        };

        let Ok(count) = (unsafe { session_enumerator.GetCount() }) else {
            continue;
        };

        for si in 0..count {
            let Ok(session_ctrl) = (unsafe { session_enumerator.GetSession(si) }) else {
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
                f(&vol)?;
                affected += 1;
            }
        }
    }

    Ok(affected)
}

unsafe fn for_each_session_by_pid_on_device(
    device_id: &str,
    target_pid: u32,
    mut f: impl FnMut(&ISimpleAudioVolume) -> Result<()>,
) -> Result<usize> {
    let enumerator = create_device_enumerator()?;
    let device = get_device_by_id(&enumerator, device_id)?;
    let session_manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None) }?;
    let session_enumerator = unsafe { session_manager.GetSessionEnumerator() }?;
    let count = unsafe { session_enumerator.GetCount() }?;
    let mut affected = 0;
    for index in 0..count {
        let Ok(session_ctrl) = (unsafe { session_enumerator.GetSession(index) }) else {
            continue;
        };
        let pid = session_ctrl
            .cast::<IAudioSessionControl2>()
            .and_then(|control| unsafe { control.GetProcessId() })
            .unwrap_or(0);
        if pid != target_pid {
            continue;
        }
        if let Ok(control) = session_ctrl.cast::<ISimpleAudioVolume>() {
            f(&control)?;
            affected += 1;
        }
    }
    Ok(affected)
}

// ── 默认设备切换（Phase 5+）───────────────────────────────

/// 将指定端点设置为默认设备（自动判断输出/输入方向）。
pub fn set_default_device(device_id: &str) -> Result<()> {
    // 从设备 ID 判断方向：{0.0.0.xxx} 开头的是输出，{0.0.1.xxx} 是输入
    let is_input = device_id.contains("{0.0.1.");
    let direction = if is_input {
        DeviceDirection::Input
    } else {
        DeviceDirection::Output
    };
    ensure_known_device(device_id, direction)?;
    let _apartment = ComApartment::initialize(COINIT_APARTMENTTHREADED)?;
    super::policy_config::set_default_endpoint(device_id, is_input)
}

/// 设置特定应用的输出设备（per-app 路由，Win10 1803+）。
pub fn set_app_output_device(pid: u32, device_id: &str) -> Result<()> {
    if pid == 0 {
        return Err(Error::new(
            HRESULT::from_win32(87),
            "系统音效不支持 per-app 路由",
        ));
    }
    if !device_id.is_empty() {
        ensure_known_device(device_id, DeviceDirection::Output)?;
    }
    let _apartment = ComApartment::initialize(COINIT_APARTMENTTHREADED)?;
    super::policy_config::set_app_output_device(pid, device_id)
}

/// 读取特定应用当前保存的输出设备；`None` 表示跟随系统默认输出。
pub fn get_app_output_device(pid: u32) -> Result<Option<String>> {
    if pid == 0 {
        return Ok(None);
    }
    let _apartment = ComApartment::initialize(COINIT_APARTMENTTHREADED)?;
    super::policy_config::get_app_output_device(pid)
}

fn ensure_known_device(device_id: &str, direction: DeviceDirection) -> Result<()> {
    let devices = enumerate_devices(direction)?;
    if devices.iter().any(|device| device.device_id == device_id) {
        Ok(())
    } else {
        Err(Error::new(
            HRESULT::from_win32(1168),
            "指定的音频设备不存在或当前不可用",
        ))
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
pub fn apply_profile(profile: &super::profile::Profile) -> Result<()> {
    let sessions = enumerate_sessions()?;
    let active_pids: std::collections::HashSet<u32> =
        sessions.iter().map(|session| session.pid).collect();

    for entry in &profile.entries {
        let target_pid = if active_pids.contains(&entry.pid) {
            Some(entry.pid)
        } else {
            sessions
                .iter()
                .find(|session| {
                    session
                        .display_name
                        .eq_ignore_ascii_case(&entry.display_name)
                })
                .map(|session| session.pid)
        };

        if let Some(pid) = target_pid {
            apply_to_pid(pid, entry.volume, entry.muted)?;
        }
    }

    Ok(())
}

/// 设置指定 PID 的音量和静音。
fn apply_to_pid(pid: u32, volume: f32, muted: bool) -> Result<()> {
    // 先设静音，再设音量（静音状态独立于音量值）
    set_session_mute(pid, muted)?;
    set_session_volume(pid, volume)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_sessions_prefer_the_current_default_output() {
        let mut sessions = vec![
            AudioSession {
                display_name: "Player".to_string(),
                process_name: Some("player.exe".to_string()),
                device_id: "other".to_string(),
                pid: 9,
                volume: 0.2,
                muted: false,
            },
            AudioSession {
                display_name: "Player".to_string(),
                process_name: Some("player.exe".to_string()),
                device_id: "default".to_string(),
                pid: 9,
                volume: 0.8,
                muted: false,
            },
        ];
        deduplicate_sessions(&mut sessions, Some("default"));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].device_id, "default");
        assert_eq!(sessions[0].volume, 0.8);
    }
}
