//! 默认音频设备切换——尝试多种 COM 接口实现。
//!
//! Win10 及更早版本可通过 `IPolicyConfig::SetDefaultEndpoint` 实现。
//! Win11 已系统性地禁用此类调用（返回 S_OK 但不生效）。
//! 前端降级方案：打开 Windows 声音设置面板。

use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::{
    Media::Audio::{eCommunications, eConsole, eMultimedia, ERole},
    System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
};

// ── CLSID ──────────────────────────────────────────────
const CLSID_WIN7: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const CLSID_WIN10: GUID = GUID::from_u128(0x294935ce_f637_4e7c_a41b_ab255460b862);

// ── IID ────────────────────────────────────────────────
const IID_WIN7: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);

// ── Win7 IPolicyConfig vtable（11 方法，对齐 AudioSwitcher 源码）─

#[repr(C)]
struct PolicyConfigVtbl {
    query_interface: unsafe extern "system" fn(
        *mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    get_mix_format: usize,
    get_device_format: usize,
    set_device_format: usize,
    get_processing_period: usize,
    set_processing_period: usize,
    get_share_mode: usize,
    set_share_mode: usize,
    get_property_value: usize,
    set_property_value: usize,
    set_default_endpoint: unsafe extern "system" fn(
        *mut core::ffi::c_void, PCWSTR, ERole,
    ) -> HRESULT,
    set_endpoint_visibility: usize,
}

#[repr(transparent)]
struct PolicyConfig(*mut *mut PolicyConfigVtbl);

impl PolicyConfig {
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

impl Clone for PolicyConfig {
    fn clone(&self) -> Self {
        unsafe { ((**self.0).add_ref)(self.0 as *mut _ as *mut core::ffi::c_void); }
        Self(self.0)
    }
}
impl Drop for PolicyConfig {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                ((**self.0).release)(self.0 as *mut _ as *mut core::ffi::c_void);
            }
        }
    }
}
unsafe impl windows::core::Interface for PolicyConfig {
    type Vtable = PolicyConfigVtbl;
    const IID: GUID = IID_WIN7;
}

// ── 公开 API ────────────────────────────────────────────

/// 尝试将指定端点设为默认设备。
///
/// 在当前 Windows 版本上可能不生效（Win11 已锁定此 API）。
/// 失败时前端会降级打开 Windows 声音设置面板。
pub fn set_default_endpoint(device_id: &str) -> windows::core::Result<()> {
    // 尝试 Win7 IID + Win10 CLSID（在我们的 Win11 测试机上可创建对象）
    if let Ok(cfg) =
        unsafe { CoCreateInstance::<_, PolicyConfig>(&CLSID_WIN10, None, CLSCTX_INPROC_SERVER) }
    {
        unsafe {
            let _ = cfg.set_default(device_id, eConsole);
            let _ = cfg.set_default(device_id, eMultimedia);
            let _ = cfg.set_default(device_id, eCommunications);
        }
        return Ok(());
    }

    // 尝试 Win7 IID + Win7 CLSID
    if let Ok(cfg) =
        unsafe { CoCreateInstance::<_, PolicyConfig>(&CLSID_WIN7, None, CLSCTX_INPROC_SERVER) }
    {
        unsafe {
            let _ = cfg.set_default(device_id, eConsole);
            let _ = cfg.set_default(device_id, eMultimedia);
            let _ = cfg.set_default(device_id, eCommunications);
        }
        return Ok(());
    }

    Err(windows::core::Error::from_hresult(HRESULT::from_win32(0x8000_FFFF)))
}
