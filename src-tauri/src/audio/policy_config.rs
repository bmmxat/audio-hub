//! WinRT 音频策略接口 — 默认设备切换 + Per-app 路由。
//!
//! EarTrumpet 同款实现：
//!   RoGetActivationFactory("Windows.Media.Internal.AudioPolicyConfig")
//!   → IAudioPolicyConfigFactoryVariantFor21H2 (IID: ab3d4648-...)
//!   → SetPersistedDefaultAudioEndpoint(processId, flow, role, HSTRING)

use windows::core::{GUID, HSTRING, HRESULT, PCWSTR};
use windows::Win32::{
    Media::Audio::{eCommunications, eConsole, eMultimedia, eRender, EDataFlow, ERole},
    System::WinRT::RoGetActivationFactory,
};

// ── IAudioPolicyConfigFactory (21H2+, IInspectable-based) ─

const IID_POLICY_FACTORY_21H2: GUID =
    GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324);

const DEVINTERFACE_AUDIO_RENDER: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const DEVINTERFACE_AUDIO_CAPTURE: &str = "#{2eef81be-33fa-4800-9670-1cd474972c3f}";
const MMDEVAPI_TOKEN: &str = r"\\?\SWD#MMDEVAPI#";

// vtable: IUnknown(3) + IInspectable(3) + 19 incomplete + 3 real
#[repr(C)]
struct PolicyFactoryVtbl {
    // IUnknown
    query_interface: unsafe extern "system" fn(
        *mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    // IInspectable（WinRT 接口必须）
    get_iids: usize,
    get_runtime_class_name: usize,
    get_trust_level: usize,
    // 19 incomplete methods
    _m6: usize, _m7: usize, _m8: usize, _m9: usize, _m10: usize,
    _m11: usize, _m12: usize, _m13: usize, _m14: usize, _m15: usize,
    _m16: usize, _m17: usize, _m18: usize, _m19: usize, _m20: usize,
    _m21: usize, _m22: usize, _m23: usize, _m24: usize,
    // SetPersistedDefaultAudioEndpoint (slot 25)
    set_persisted: unsafe extern "system" fn(
        *mut core::ffi::c_void, u32, EDataFlow, ERole, *mut core::ffi::c_void, // HSTRING
    ) -> HRESULT,
    // GetPersistedDefaultAudioEndpoint (slot 26)
    get_persisted: unsafe extern "system" fn(
        *mut core::ffi::c_void, u32, EDataFlow, ERole, *mut *mut core::ffi::c_void,
    ) -> HRESULT,
    // ClearAllPersistedApplicationDefaultEndpoints (slot 27)
    clear_all: unsafe extern "system" fn(
        *mut core::ffi::c_void,
    ) -> HRESULT,
}

#[repr(transparent)]
struct PolicyFactory(*mut *mut PolicyFactoryVtbl);

unsafe impl Send for PolicyFactory {}
unsafe impl Sync for PolicyFactory {}

impl PolicyFactory {
    /// 设置 per-app 或系统默认端点（processId=0 即系统默认）。
    unsafe fn set_persisted_default(
        &self, pid: u32, flow: EDataFlow, role: ERole, device_id: &str,
    ) -> windows::core::Result<()> {
        use windows::Win32::System::WinRT::WindowsCreateString;
        // 空字符串 = 清除路由（传 null HSTRING）
        let hstring = if device_id.is_empty() {
            None
        } else {
            let formatted = format_device_id(device_id, flow);
            let wide: Vec<u16> = formatted.encode_utf16().collect();
            Some(unsafe { WindowsCreateString(Some(&wide))? })
        };
        let raw = hstring
            .as_ref()
            .map(|h| unsafe { core::mem::transmute_copy(h) })
            .unwrap_or(std::ptr::null_mut());
        let hr = unsafe {
            ((**self.0).set_persisted)(
                self.0 as *mut _ as *mut core::ffi::c_void,
                pid, flow, role, raw,
            )
        };
        // 阻止 HSTRING Drop 时释放（已被 COM 接管）
        if let Some(h) = hstring {
            core::mem::forget(h);
        }
        hr.ok()
    }

    /// 清除所有 per-app 默认端点设置。
    #[allow(dead_code)]
    unsafe fn clear_all_persisted(&self) -> windows::core::Result<()> {
        unsafe {
            ((**self.0).clear_all)(self.0 as *mut _ as *mut core::ffi::c_void)
        }
        .ok()
    }
}

impl Clone for PolicyFactory {
    fn clone(&self) -> Self {
        unsafe { ((**self.0).add_ref)(self.0 as *mut _ as *mut core::ffi::c_void); }
        Self(self.0)
    }
}
impl Drop for PolicyFactory {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                ((**self.0).release)(self.0 as *mut _ as *mut core::ffi::c_void);
            }
        }
    }
}
unsafe impl windows::core::Interface for PolicyFactory {
    type Vtable = PolicyFactoryVtbl;
    const IID: GUID = IID_POLICY_FACTORY_21H2;
}

// ── 设备 ID 格式化 ─────────────────────────────────────
fn format_device_id(device_id: &str, flow: EDataFlow) -> String {
    let suffix = if flow == eRender {
        DEVINTERFACE_AUDIO_RENDER
    } else {
        DEVINTERFACE_AUDIO_CAPTURE
    };
    format!("{MMDEVAPI_TOKEN}{device_id}{suffix}")
}

// ── IPolicyConfigWin7（EarTrumpet 同款，12 方法）────────

const CLSID_POLICY_WIN7: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const IID_POLICY_WIN7: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);

#[repr(C)]
struct PolicyConfigWin7Vtbl {
    // IUnknown
    query_interface: unsafe extern "system" fn(
        *mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    // EarTrumpet: 8 Unused + Get/SetPropertyValue + SetDefaultEndpoint + SetEndpointVisibility = 12
    _u1: usize, _u2: usize, _u3: usize, _u4: usize,
    _u5: usize, _u6: usize, _u7: usize, _u8: usize,
    get_property_value: usize,
    set_property_value: usize,
    set_default_endpoint: unsafe extern "system" fn(
        *mut core::ffi::c_void, PCWSTR, ERole,
    ) -> HRESULT,
    set_endpoint_visibility: usize,
}

#[repr(transparent)]
struct PolicyConfigWin7(*mut *mut PolicyConfigWin7Vtbl);

impl PolicyConfigWin7 {
    unsafe fn set_default(&self, id: &str, role: ERole) -> windows::core::Result<()> {
        let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            ((**self.0).set_default_endpoint)(
                self.0 as *mut _ as *mut core::ffi::c_void,
                PCWSTR::from_raw(wide.as_ptr()),
                role,
            )
        }
        .ok()
    }
}

impl Clone for PolicyConfigWin7 {
    fn clone(&self) -> Self {
        unsafe { ((**self.0).add_ref)(self.0 as *mut _ as *mut core::ffi::c_void); }
        Self(self.0)
    }
}
impl Drop for PolicyConfigWin7 {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                ((**self.0).release)(self.0 as *mut _ as *mut core::ffi::c_void);
            }
        }
    }
}
unsafe impl windows::core::Interface for PolicyConfigWin7 {
    type Vtable = PolicyConfigWin7Vtbl;
    const IID: GUID = IID_POLICY_WIN7;
}

// ── 公开 API ────────────────────────────────────────────

fn get_factory() -> windows::core::Result<PolicyFactory> {
    let class_id = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
    unsafe { RoGetActivationFactory(&class_id) }
}

pub fn set_default_endpoint(device_id: &str, is_input: bool) -> windows::core::Result<()> {
    use windows::Win32::Media::Audio::eCapture;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    let flow = if is_input { eCapture } else { eRender };

    // 路径 A：IPolicyConfigWin7（系统默认设备）
    if let Ok(cfg) =
        unsafe { CoCreateInstance::<_, PolicyConfigWin7>(&CLSID_POLICY_WIN7, None, CLSCTX_INPROC_SERVER) }
    {
        unsafe {
            let _ = cfg.set_default(device_id, eConsole);
            let _ = cfg.set_default(device_id, eMultimedia);
            let _ = cfg.set_default(device_id, eCommunications);
        }
        return Ok(());
    }

    // 路径 B：WinRT factory 回退
    let factory = get_factory()?;
    unsafe {
        factory.set_persisted_default(0, flow, eConsole, device_id)?;
        factory.set_persisted_default(0, flow, eMultimedia, device_id)?;
    }
    Ok(())
}

pub fn set_app_output_device(pid: u32, device_id: &str) -> windows::core::Result<()> {
    let factory = get_factory()?;
    // 空字符串 = 清除该 PID 的 per-app 路由（传 null HSTRING 给 SetPersistedDefaultAudioEndpoint）
    unsafe {
        factory.set_persisted_default(pid, eRender, eConsole, device_id)?;
        factory.set_persisted_default(pid, eRender, eMultimedia, device_id)?;
    }
    Ok(())
}
