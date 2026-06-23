use windows::{
    core::*,
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::*,
        System::{
            Com::{
                StructuredStorage::PROPVARIANT, *,
            },
            ProcessStatus::K32GetModuleBaseNameW,
            Threading::{
                OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
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

            // 先获取默认设备 ID，用于对比
            let default_id = get_default_render_device_id().ok();

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

/// 从 PID 获取进程的可执行文件名（例如 "chrome.exe"）。
///
/// 返回 `None` 如果进程无法访问（例如系统进程或权限不足）。
fn get_process_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }

    unsafe {
        // 打开进程句柄
        let handle = OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
        .ok()?;

        // 查询进程的可执行文件名
        let mut buffer = [0u16; 260]; // MAX_PATH
        let len = K32GetModuleBaseNameW(handle, None, &mut buffer);

        // 关闭句柄（忽略返回值，句柄关闭失败不影响后续逻辑）
        let _ = windows::Win32::Foundation::CloseHandle(handle);

        if len == 0 {
            return None;
        }

        let wide = &buffer[..len as usize];
        String::from_utf16(wide).ok()
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
                    // 2. 从 PID 反查进程名
                    // 3. 系统音效特殊标记
                    // 以上都无法获取 → 跳过（进程可能已退出，无意义）
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

                    // PID 非零但无法解析名称的会话直接跳过
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
