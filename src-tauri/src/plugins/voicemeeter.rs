use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::{CString, c_char, c_void},
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::Duration,
};

type VmResult = i32;
type LoginFn = unsafe extern "system" fn() -> VmResult;
type LogoutFn = unsafe extern "system" fn() -> VmResult;
type RunVoicemeeterFn = unsafe extern "system" fn(i32) -> VmResult;
type GetVoicemeeterTypeFn = unsafe extern "system" fn(*mut i32) -> VmResult;
type IsParametersDirtyFn = unsafe extern "system" fn() -> VmResult;
type GetParameterFloatFn = unsafe extern "system" fn(*const c_char, *mut f32) -> VmResult;
type SetParameterFloatFn = unsafe extern "system" fn(*const c_char, f32) -> VmResult;
type GetParameterStringWFn = unsafe extern "system" fn(*const c_char, *mut u16) -> VmResult;
type SetParameterStringWFn = unsafe extern "system" fn(*const c_char, *const u16) -> VmResult;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(file_name: *const u16) -> isize;
    fn GetProcAddress(module: isize, proc_name: *const u8) -> *const c_void;
    fn FreeLibrary(module: isize) -> i32;
}

#[derive(Debug, Clone, Serialize)]
pub struct VoicemeeterStatus {
    pub installed: bool,
    pub running: bool,
    pub connected: bool,
    pub edition: Option<String>,
    pub edition_code: Option<i32>,
    pub install_directory: Option<String>,
    pub configuration: Option<VoicemeeterConfiguration>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoicemeeterConfiguration {
    pub monitor_device_name: Option<String>,
    pub a1_equalizer: Option<VoicemeeterEqConfiguration>,
    pub main_mix: VoicemeeterMixConfiguration,
    pub aux_mix: Option<VoicemeeterMixConfiguration>,
    pub physical_input: VoicemeeterPhysicalInputConfiguration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoicemeeterMixConfiguration {
    pub monitor_enabled: bool,
    pub virtual_microphone_enabled: bool,
    pub input_muted: bool,
    pub virtual_microphone_muted: bool,
    pub input_gain_db: f32,
    pub virtual_microphone_gain_db: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoicemeeterPhysicalInputConfiguration {
    pub device_name: Option<String>,
    pub muted: bool,
    pub gain_db: f32,
    pub monitor_enabled: bool,
    pub main_mix_enabled: bool,
    pub aux_mix_enabled: bool,
    pub audibility: Option<f32>,
    pub compressor: Option<f32>,
    pub noise_gate: Option<f32>,
    pub denoiser: Option<f32>,
    pub equalizer: Option<VoicemeeterEqConfiguration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoicemeeterEqConfiguration {
    pub enabled: bool,
    pub bands: Vec<VoicemeeterEqBand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoicemeeterEqBand {
    pub enabled: bool,
    pub filter_type: i32,
    pub frequency_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

pub struct VoicemeeterManager {
    session: Mutex<Option<RemoteSession>>,
}

impl Default for VoicemeeterManager {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
}

impl VoicemeeterManager {
    pub fn status(&self) -> VoicemeeterStatus {
        let Some(install_dir) = find_install_directory() else {
            return missing_status();
        };
        match self.with_session(&install_dir, |session| Ok(session.status())) {
            Ok(status) => status,
            Err(error) => VoicemeeterStatus {
                installed: true,
                running: false,
                connected: false,
                edition: None,
                edition_code: None,
                install_directory: Some(path_text(&install_dir)),
                configuration: None,
                note: error,
            },
        }
    }

    pub fn start(&self) -> Result<VoicemeeterStatus, String> {
        let install_dir = require_install_directory()?;
        self.with_session(&install_dir, |session| {
            if !session.is_running() {
                let edition = preferred_edition(&install_dir);
                session.api.check(
                    unsafe { (session.api.run_voicemeeter)(edition) },
                    "启动 VoiceMeeter",
                )?;
                thread::sleep(Duration::from_millis(900));
            }
            session.status_result()
        })
    }

    pub fn show(&self) -> Result<VoicemeeterStatus, String> {
        let install_dir = require_install_directory()?;
        self.with_session(&install_dir, |session| {
            session.ensure_running(&install_dir)?;
            session.api.set_float("Command.Show", 1.0)?;
            session.status_result()
        })
    }

    pub fn restart_audio_engine(&self) -> Result<VoicemeeterStatus, String> {
        let install_dir = require_install_directory()?;
        self.with_session(&install_dir, |session| {
            session.ensure_running(&install_dir)?;
            session.api.set_float("Command.Restart", 1.0)?;
            thread::sleep(Duration::from_millis(350));
            session.status_result()
        })
    }

    pub fn shutdown(&self) -> Result<VoicemeeterStatus, String> {
        let install_dir = require_install_directory()?;
        self.with_session(&install_dir, |session| {
            if session.is_running() {
                session.api.set_float("Command.Shutdown", 1.0)?;
                for _ in 0..20 {
                    thread::sleep(Duration::from_millis(50));
                    if !session.is_running() {
                        break;
                    }
                }
                if session.is_running() {
                    return Err("VoiceMeeter 未能在预期时间内退出。".to_string());
                }
            }
            session.status_result()
        })
    }

    pub fn set_monitor_device(&self, device_name: &str) -> Result<(), String> {
        let device_name = device_name.trim();
        if device_name.is_empty() || device_name.len() > 256 {
            return Err("本地监听设备名称无效。".to_string());
        }
        let install_dir = require_install_directory()?;
        self.with_session(&install_dir, |session| {
            if !session.is_running() {
                return Err("VoiceMeeter 未启动，无法同步本地监听设备。".to_string());
            }
            session.api.set_string("Bus[0].Device.wdm", device_name)
        })
    }

    pub fn apply(
        &self,
        configuration: VoicemeeterConfiguration,
    ) -> Result<VoicemeeterStatus, String> {
        validate_configuration(&configuration)?;
        let install_dir = require_install_directory()?;
        self.with_session(&install_dir, |session| {
            session.ensure_running(&install_dir)?;
            let layout = session.layout()?;

            apply_mix(&session.api, layout.main_mix, &configuration.main_mix)?;
            match (layout.aux_mix, configuration.aux_mix.as_ref()) {
                (Some(aux_layout), Some(aux_configuration)) => {
                    apply_mix(&session.api, aux_layout, aux_configuration)?;
                }
                (None, Some(_)) => {
                    return Err(
                        "当前 VoiceMeeter Standard 不支持 AUX 混音，请安装 Banana 或 Potato。"
                            .to_string(),
                    );
                }
                _ => {}
            }

            if let Some(device_name) = configuration
                .monitor_device_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                session.api.set_string("Bus[0].Device.wdm", device_name)?;
            }

            if layout.supports_bus_equalizer() {
                if let Some(requested) = configuration.a1_equalizer.as_ref() {
                    let current = read_equalizer(&session.api, "Bus", 0)?;
                    if &current != requested {
                        apply_equalizer(&session.api, "Bus", 0, requested, 0..8)?;
                    }
                }
            } else if configuration.a1_equalizer.is_some() {
                return Err("VoiceMeeter Standard 不支持 A1 参数均衡器。".to_string());
            }

            let physical_device_parameter =
                format!("Strip[{}].Device.name", layout.physical_input_strip);
            let current_physical_device = session
                .api
                .get_string(&physical_device_parameter)
                .unwrap_or_default();
            let requested_physical_device = configuration
                .physical_input
                .device_name
                .as_deref()
                .map(str::trim)
                .unwrap_or_default();
            if current_physical_device.trim() != requested_physical_device {
                session.api.set_string(
                    &format!("Strip[{}].Device.wdm", layout.physical_input_strip),
                    requested_physical_device,
                )?;
            }
            session.api.set_float(
                &format!("Strip[{}].Mute", layout.physical_input_strip),
                bool_value(configuration.physical_input.muted),
            )?;
            session.api.set_float(
                &format!("Strip[{}].Gain", layout.physical_input_strip),
                configuration.physical_input.gain_db,
            )?;
            session.api.set_float(
                &format!("Strip[{}].A1", layout.physical_input_strip),
                bool_value(configuration.physical_input.monitor_enabled),
            )?;
            session.api.set_float(
                &format!("Strip[{}].B1", layout.physical_input_strip),
                bool_value(configuration.physical_input.main_mix_enabled),
            )?;
            if layout.aux_mix.is_some() {
                session.api.set_float(
                    &format!("Strip[{}].B2", layout.physical_input_strip),
                    bool_value(configuration.physical_input.aux_mix_enabled),
                )?;
            }

            let strip = layout.physical_input_strip;
            match layout.code {
                1 => {
                    if let Some(value) = configuration.physical_input.audibility {
                        session
                            .api
                            .set_float(&format!("Strip[{strip}].Audibility"), value)?;
                    }
                }
                2 | 3 => {
                    if let Some(value) = configuration.physical_input.compressor {
                        session
                            .api
                            .set_float(&format!("Strip[{strip}].Comp"), value)?;
                    }
                    if let Some(value) = configuration.physical_input.noise_gate {
                        session
                            .api
                            .set_float(&format!("Strip[{strip}].Gate"), value)?;
                    }
                }
                _ => unreachable!(),
            }
            if layout.supports_denoiser()
                && let Some(value) = configuration.physical_input.denoiser
            {
                session
                    .api
                    .set_float(&format!("Strip[{strip}].Denoiser"), value)?;
            }
            if layout.supports_strip_equalizer()
                && let Some(requested) = configuration.physical_input.equalizer.as_ref()
            {
                let current = read_equalizer(&session.api, "Strip", strip)?;
                if &current != requested {
                    apply_equalizer(&session.api, "Strip", strip, requested, 0..2)?;
                }
            }

            // VoiceMeeter accepts Remote API writes asynchronously. Its cached
            // parameters can still contain the previous values immediately after
            // successful setters, so keep the requested configuration in the
            // response and let an explicit refresh reconcile the live state.
            thread::sleep(Duration::from_millis(60));
            let mut status = session.status_result()?;
            status.configuration = Some(configuration.clone());
            Ok(status)
        })
    }

    fn with_session<T>(
        &self,
        install_dir: &Path,
        action: impl FnOnce(&mut RemoteSession) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "VoiceMeeter 控制器状态已损坏。".to_string())?;
        if guard.is_none() {
            *guard = Some(RemoteSession::open(install_dir)?);
        }
        action(guard.as_mut().expect("session was initialized"))
    }
}

struct RemoteSession {
    api: RemoteApi,
}

impl RemoteSession {
    fn open(install_dir: &Path) -> Result<Self, String> {
        let api = RemoteApi::load(install_dir)?;
        api.check(unsafe { (api.login)() }, "连接 VoiceMeeter Remote API")?;
        Ok(Self { api })
    }

    fn is_running(&self) -> bool {
        (unsafe { (self.api.is_parameters_dirty)() }) >= 0
    }

    fn ensure_running(&self, install_dir: &Path) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }
        self.api.check(
            unsafe { (self.api.run_voicemeeter)(preferred_edition(install_dir)) },
            "启动 VoiceMeeter",
        )?;
        thread::sleep(Duration::from_millis(900));
        if self.is_running() {
            Ok(())
        } else {
            Err("VoiceMeeter 已启动，但 Remote API 尚未就绪。请稍后重试。".to_string())
        }
    }

    fn status(&mut self) -> VoicemeeterStatus {
        self.status_result()
            .unwrap_or_else(|error| VoicemeeterStatus {
                installed: true,
                running: self.is_running(),
                connected: false,
                edition: None,
                edition_code: None,
                install_directory: find_install_directory().as_deref().map(path_text),
                configuration: None,
                note: error,
            })
    }

    fn status_result(&self) -> Result<VoicemeeterStatus, String> {
        let install_dir = find_install_directory();
        if !self.is_running() {
            return Ok(VoicemeeterStatus {
                installed: true,
                running: false,
                connected: false,
                edition: None,
                edition_code: None,
                install_directory: install_dir.as_deref().map(path_text),
                configuration: None,
                note: "已安装，等待启动。VoiceMeeter 需要运行才能提供虚拟麦克风。".to_string(),
            });
        }

        let edition_code = self.api.voicemeeter_type()?;
        let layout = EditionLayout::from_code(edition_code)?;
        let configuration = VoicemeeterConfiguration {
            monitor_device_name: self
                .api
                .get_string("Bus[0].Device.name")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            a1_equalizer: layout
                .supports_bus_equalizer()
                .then(|| read_equalizer(&self.api, "Bus", 0))
                .transpose()?,
            main_mix: read_mix(&self.api, layout.main_mix)?,
            aux_mix: layout
                .aux_mix
                .map(|mix_layout| read_mix(&self.api, mix_layout))
                .transpose()?,
            physical_input: VoicemeeterPhysicalInputConfiguration {
                device_name: self
                    .api
                    .get_string(&format!(
                        "Strip[{}].Device.name",
                        layout.physical_input_strip
                    ))
                    .ok()
                    .filter(|value| !value.trim().is_empty()),
                muted: self
                    .api
                    .get_bool(&format!("Strip[{}].Mute", layout.physical_input_strip))?,
                gain_db: self
                    .api
                    .get_float(&format!("Strip[{}].Gain", layout.physical_input_strip))?,
                monitor_enabled: self
                    .api
                    .get_bool(&format!("Strip[{}].A1", layout.physical_input_strip))?,
                main_mix_enabled: self
                    .api
                    .get_bool(&format!("Strip[{}].B1", layout.physical_input_strip))?,
                aux_mix_enabled: if layout.aux_mix.is_some() {
                    self.api
                        .get_bool(&format!("Strip[{}].B2", layout.physical_input_strip))?
                } else {
                    false
                },
                audibility: (layout.code == 1)
                    .then(|| {
                        self.api.get_float(&format!(
                            "Strip[{}].Audibility",
                            layout.physical_input_strip
                        ))
                    })
                    .transpose()?,
                compressor: (layout.code >= 2)
                    .then(|| {
                        self.api
                            .get_float(&format!("Strip[{}].Comp", layout.physical_input_strip))
                    })
                    .transpose()?,
                noise_gate: (layout.code >= 2)
                    .then(|| {
                        self.api
                            .get_float(&format!("Strip[{}].Gate", layout.physical_input_strip))
                    })
                    .transpose()?,
                denoiser: layout
                    .supports_denoiser()
                    .then(|| {
                        self.api
                            .get_float(&format!("Strip[{}].Denoiser", layout.physical_input_strip))
                    })
                    .transpose()?,
                equalizer: layout
                    .supports_strip_equalizer()
                    .then(|| read_equalizer(&self.api, "Strip", layout.physical_input_strip))
                    .transpose()?,
            },
        };

        Ok(VoicemeeterStatus {
            installed: true,
            running: true,
            connected: true,
            edition: Some(layout.name.to_string()),
            edition_code: Some(edition_code),
            install_directory: install_dir.as_deref().map(path_text),
            configuration: Some(configuration),
            note: "VoiceMeeter 音频引擎正在后台运行，参数调整会立即生效。".to_string(),
        })
    }

    fn layout(&self) -> Result<EditionLayout, String> {
        EditionLayout::from_code(self.api.voicemeeter_type()?)
    }
}

impl Drop for RemoteSession {
    fn drop(&mut self) {
        unsafe {
            (self.api.logout)();
        }
    }
}

struct RemoteApi {
    module: isize,
    login: LoginFn,
    logout: LogoutFn,
    run_voicemeeter: RunVoicemeeterFn,
    get_voicemeeter_type: GetVoicemeeterTypeFn,
    is_parameters_dirty: IsParametersDirtyFn,
    get_parameter_float: GetParameterFloatFn,
    set_parameter_float: SetParameterFloatFn,
    get_parameter_string_w: GetParameterStringWFn,
    set_parameter_string_w: SetParameterStringWFn,
}

impl RemoteApi {
    fn load(install_dir: &Path) -> Result<Self, String> {
        let dll_name = if cfg!(target_pointer_width = "64") {
            "VoicemeeterRemote64.dll"
        } else {
            "VoicemeeterRemote.dll"
        };
        let dll_path = install_dir.join(dll_name);
        if !dll_path.is_file() {
            return Err(format!(
                "未找到 {}。请修复或重新安装 VoiceMeeter。",
                dll_name
            ));
        }
        let wide_path = wide_null(&dll_path);
        let module = unsafe { LoadLibraryW(wide_path.as_ptr()) };
        if module == 0 {
            return Err(format!("无法加载 {}。", dll_path.display()));
        }

        let loaded = (|| unsafe {
            Ok(Self {
                module,
                login: load_symbol(module, b"VBVMR_Login\0")?,
                logout: load_symbol(module, b"VBVMR_Logout\0")?,
                run_voicemeeter: load_symbol(module, b"VBVMR_RunVoicemeeter\0")?,
                get_voicemeeter_type: load_symbol(module, b"VBVMR_GetVoicemeeterType\0")?,
                is_parameters_dirty: load_symbol(module, b"VBVMR_IsParametersDirty\0")?,
                get_parameter_float: load_symbol(module, b"VBVMR_GetParameterFloat\0")?,
                set_parameter_float: load_symbol(module, b"VBVMR_SetParameterFloat\0")?,
                get_parameter_string_w: load_symbol(module, b"VBVMR_GetParameterStringW\0")?,
                set_parameter_string_w: load_symbol(module, b"VBVMR_SetParameterStringW\0")?,
            })
        })();
        if loaded.is_err() {
            unsafe {
                FreeLibrary(module);
            }
        }
        loaded
    }

    fn check(&self, result: VmResult, operation: &str) -> Result<VmResult, String> {
        if result >= 0 {
            Ok(result)
        } else {
            Err(format!("{operation}失败（Remote API 错误 {result}）。"))
        }
    }

    fn voicemeeter_type(&self) -> Result<i32, String> {
        let mut value = 0;
        self.check(
            unsafe { (self.get_voicemeeter_type)(&mut value) },
            "读取 VoiceMeeter 版本",
        )?;
        Ok(value)
    }

    fn get_float(&self, parameter: &str) -> Result<f32, String> {
        let parameter = c_string(parameter)?;
        let mut value = 0.0;
        self.check(
            unsafe { (self.get_parameter_float)(parameter.as_ptr(), &mut value) },
            "读取 VoiceMeeter 参数",
        )?;
        Ok(value)
    }

    fn get_bool(&self, parameter: &str) -> Result<bool, String> {
        Ok(self.get_float(parameter)? >= 0.5)
    }

    fn set_float(&self, parameter: &str, value: f32) -> Result<(), String> {
        let parameter = c_string(parameter)?;
        self.check(
            unsafe { (self.set_parameter_float)(parameter.as_ptr(), value) },
            "更新 VoiceMeeter 参数",
        )?;
        Ok(())
    }

    fn get_string(&self, parameter: &str) -> Result<String, String> {
        let parameter = c_string(parameter)?;
        let mut buffer = vec![0_u16; 512];
        self.check(
            unsafe { (self.get_parameter_string_w)(parameter.as_ptr(), buffer.as_mut_ptr()) },
            "读取 VoiceMeeter 文本参数",
        )?;
        let length = buffer
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(buffer.len());
        Ok(String::from_utf16_lossy(&buffer[..length]))
    }

    fn set_string(&self, parameter: &str, value: &str) -> Result<(), String> {
        let parameter = c_string(parameter)?;
        let value = wide_null(value);
        self.check(
            unsafe { (self.set_parameter_string_w)(parameter.as_ptr(), value.as_ptr()) },
            "更新 VoiceMeeter 文本参数",
        )?;
        Ok(())
    }
}

impl Drop for RemoteApi {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.module);
        }
    }
}

unsafe fn load_symbol<T: Copy>(module: isize, name: &'static [u8]) -> Result<T, String> {
    let address = unsafe { GetProcAddress(module, name.as_ptr()) };
    if address.is_null() {
        let symbol = String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]);
        return Err(format!("VoiceMeeter Remote API 缺少函数 {symbol}。"));
    }
    Ok(unsafe { std::mem::transmute_copy(&address) })
}

#[derive(Clone, Copy)]
struct MixLayout {
    virtual_input_strip: usize,
    virtual_microphone_route: usize,
    virtual_microphone_bus: usize,
}

#[derive(Clone, Copy)]
struct EditionLayout {
    code: i32,
    name: &'static str,
    physical_input_strip: usize,
    main_mix: MixLayout,
    aux_mix: Option<MixLayout>,
}

impl EditionLayout {
    fn from_code(code: i32) -> Result<Self, String> {
        match code {
            1 => Ok(Self {
                code,
                name: "VoiceMeeter Standard",
                physical_input_strip: 0,
                main_mix: MixLayout {
                    virtual_input_strip: 2,
                    virtual_microphone_route: 1,
                    virtual_microphone_bus: 1,
                },
                aux_mix: None,
            }),
            2 => Ok(Self {
                code,
                name: "VoiceMeeter Banana",
                physical_input_strip: 0,
                main_mix: MixLayout {
                    virtual_input_strip: 3,
                    virtual_microphone_route: 1,
                    virtual_microphone_bus: 3,
                },
                aux_mix: Some(MixLayout {
                    virtual_input_strip: 4,
                    virtual_microphone_route: 2,
                    virtual_microphone_bus: 4,
                }),
            }),
            3 => Ok(Self {
                code,
                name: "VoiceMeeter Potato",
                physical_input_strip: 0,
                main_mix: MixLayout {
                    virtual_input_strip: 5,
                    virtual_microphone_route: 1,
                    virtual_microphone_bus: 5,
                },
                aux_mix: Some(MixLayout {
                    virtual_input_strip: 6,
                    virtual_microphone_route: 2,
                    virtual_microphone_bus: 6,
                }),
            }),
            _ => Err(format!("暂不支持该 VoiceMeeter 版本类型（{code}）。")),
        }
    }

    fn supports_bus_equalizer(self) -> bool {
        self.code >= 2
    }

    fn supports_denoiser(self) -> bool {
        self.code == 3
    }

    fn supports_strip_equalizer(self) -> bool {
        self.code == 3
    }
}

fn read_equalizer(
    api: &RemoteApi,
    owner: &str,
    owner_index: usize,
) -> Result<VoicemeeterEqConfiguration, String> {
    let prefix = format!("{owner}[{owner_index}].EQ");
    let bands = (0..6)
        .map(|cell| {
            let cell_prefix = format!("{prefix}.channel[0].cell[{cell}]");
            Ok(VoicemeeterEqBand {
                enabled: api.get_bool(&format!("{cell_prefix}.on"))?,
                filter_type: api.get_float(&format!("{cell_prefix}.type"))?.round() as i32,
                frequency_hz: api.get_float(&format!("{cell_prefix}.f"))?,
                gain_db: api.get_float(&format!("{cell_prefix}.gain"))?,
                q: api.get_float(&format!("{cell_prefix}.q"))?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(VoicemeeterEqConfiguration {
        enabled: api.get_bool(&format!("{prefix}.on"))?,
        bands,
    })
}

fn apply_equalizer(
    api: &RemoteApi,
    owner: &str,
    owner_index: usize,
    configuration: &VoicemeeterEqConfiguration,
    channels: std::ops::Range<usize>,
) -> Result<(), String> {
    let prefix = format!("{owner}[{owner_index}].EQ");
    api.set_float(&format!("{prefix}.on"), bool_value(configuration.enabled))?;
    for channel in channels {
        for (cell, band) in configuration.bands.iter().enumerate() {
            let cell_prefix = format!("{prefix}.channel[{channel}].cell[{cell}]");
            api.set_float(&format!("{cell_prefix}.on"), bool_value(band.enabled))?;
            api.set_float(&format!("{cell_prefix}.type"), band.filter_type as f32)?;
            api.set_float(&format!("{cell_prefix}.f"), band.frequency_hz)?;
            api.set_float(&format!("{cell_prefix}.gain"), band.gain_db)?;
            api.set_float(&format!("{cell_prefix}.q"), band.q)?;
        }
    }
    Ok(())
}

fn read_mix(api: &RemoteApi, layout: MixLayout) -> Result<VoicemeeterMixConfiguration, String> {
    Ok(VoicemeeterMixConfiguration {
        monitor_enabled: api.get_bool(&format!("Strip[{}].A1", layout.virtual_input_strip))?,
        virtual_microphone_enabled: api.get_bool(&format!(
            "Strip[{}].B{}",
            layout.virtual_input_strip, layout.virtual_microphone_route
        ))?,
        input_muted: api.get_bool(&format!("Strip[{}].Mute", layout.virtual_input_strip))?,
        virtual_microphone_muted: api
            .get_bool(&format!("Bus[{}].Mute", layout.virtual_microphone_bus))?,
        input_gain_db: api.get_float(&format!("Strip[{}].Gain", layout.virtual_input_strip))?,
        virtual_microphone_gain_db: api
            .get_float(&format!("Bus[{}].Gain", layout.virtual_microphone_bus))?,
    })
}

fn apply_mix(
    api: &RemoteApi,
    layout: MixLayout,
    configuration: &VoicemeeterMixConfiguration,
) -> Result<(), String> {
    api.set_float(
        &format!("Strip[{}].A1", layout.virtual_input_strip),
        bool_value(configuration.monitor_enabled),
    )?;
    api.set_float(
        &format!(
            "Strip[{}].B{}",
            layout.virtual_input_strip, layout.virtual_microphone_route
        ),
        bool_value(configuration.virtual_microphone_enabled),
    )?;
    api.set_float(
        &format!("Strip[{}].Mute", layout.virtual_input_strip),
        bool_value(configuration.input_muted),
    )?;
    api.set_float(
        &format!("Strip[{}].Gain", layout.virtual_input_strip),
        configuration.input_gain_db,
    )?;
    api.set_float(
        &format!("Bus[{}].Mute", layout.virtual_microphone_bus),
        bool_value(configuration.virtual_microphone_muted),
    )?;
    api.set_float(
        &format!("Bus[{}].Gain", layout.virtual_microphone_bus),
        configuration.virtual_microphone_gain_db,
    )
}

fn validate_configuration(configuration: &VoicemeeterConfiguration) -> Result<(), String> {
    let mut gains = vec![
        ("主混音输入增益", configuration.main_mix.input_gain_db),
        (
            "主混音虚拟输出增益",
            configuration.main_mix.virtual_microphone_gain_db,
        ),
        ("物理输入增益", configuration.physical_input.gain_db),
    ];
    if let Some(aux_mix) = configuration.aux_mix.as_ref() {
        gains.extend([
            ("AUX 混音输入增益", aux_mix.input_gain_db),
            ("AUX 混音虚拟输出增益", aux_mix.virtual_microphone_gain_db),
        ]);
    }
    for (name, value) in gains {
        if !value.is_finite() || !(-60.0..=12.0).contains(&value) {
            return Err(format!("{name}必须在 -60 dB 到 +12 dB 之间。"));
        }
    }
    for (name, value) in [
        ("可听度", configuration.physical_input.audibility),
        ("压缩器", configuration.physical_input.compressor),
        ("噪声门", configuration.physical_input.noise_gate),
        ("降噪", configuration.physical_input.denoiser),
    ] {
        if let Some(value) = value
            && (!value.is_finite() || !(0.0..=10.0).contains(&value))
        {
            return Err(format!("{name}强度必须在 0 到 10 之间。"));
        }
    }
    if let Some(equalizer) = configuration.a1_equalizer.as_ref() {
        validate_equalizer("A1 输出", equalizer)?;
    }
    if let Some(equalizer) = configuration.physical_input.equalizer.as_ref() {
        validate_equalizer("物理麦克风", equalizer)?;
    }
    Ok(())
}

fn validate_equalizer(
    name: &str,
    configuration: &VoicemeeterEqConfiguration,
) -> Result<(), String> {
    if configuration.bands.len() != 6 {
        return Err(format!("{name}均衡器必须包含 6 个频段。"));
    }
    for (index, band) in configuration.bands.iter().enumerate() {
        if !(0..=6).contains(&band.filter_type) {
            return Err(format!("{name}均衡器第 {} 段类型无效。", index + 1));
        }
        if !band.frequency_hz.is_finite() || !(20.0..=20_000.0).contains(&band.frequency_hz) {
            return Err(format!("{name}均衡器第 {} 段频率无效。", index + 1));
        }
        if !band.gain_db.is_finite() || !(-12.0..=12.0).contains(&band.gain_db) {
            return Err(format!("{name}均衡器第 {} 段增益无效。", index + 1));
        }
        if !band.q.is_finite() || !(0.3..=100.0).contains(&band.q) {
            return Err(format!("{name}均衡器第 {} 段 Q 值无效。", index + 1));
        }
    }
    Ok(())
}

fn preferred_edition(install_dir: &Path) -> i32 {
    if ["voicemeeter8x64.exe", "voicemeeter8.exe"]
        .iter()
        .any(|name| install_dir.join(name).is_file())
    {
        3
    } else if ["voicemeeterpro_x64.exe", "voicemeeterpro.exe"]
        .iter()
        .any(|name| install_dir.join(name).is_file())
    {
        2
    } else {
        1
    }
}

fn find_install_directory() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
        if let Some(root) = env::var_os(variable) {
            candidates.push(PathBuf::from(root).join("VB").join("Voicemeeter"));
        }
    }
    candidates.push(PathBuf::from(r"C:\Program Files (x86)\VB\Voicemeeter"));
    candidates.push(PathBuf::from(r"C:\Program Files\VB\Voicemeeter"));
    candidates.into_iter().find(|directory| {
        directory
            .join(if cfg!(target_pointer_width = "64") {
                "VoicemeeterRemote64.dll"
            } else {
                "VoicemeeterRemote.dll"
            })
            .is_file()
    })
}

fn require_install_directory() -> Result<PathBuf, String> {
    find_install_directory().ok_or_else(|| {
        "未检测到 VoiceMeeter。请先从 VB-Audio 官方网站安装并重启 Windows。".to_string()
    })
}

fn missing_status() -> VoicemeeterStatus {
    VoicemeeterStatus {
        installed: false,
        running: false,
        connected: false,
        edition: None,
        edition_code: None,
        install_directory: None,
        configuration: None,
        note: "未检测到 VoiceMeeter。Audio Hub 不会自动安装或捆绑第三方软件。".to_string(),
    }
}

fn c_string(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| "VoiceMeeter 参数中包含无效的空字符。".to_string())
}

fn wide_null(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value
        .as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn bool_value(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

pub fn open_download_page() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg("https://vb-audio.com/Voicemeeter/")
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开 VoiceMeeter 官方下载页：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_configuration() -> VoicemeeterConfiguration {
        VoicemeeterConfiguration {
            monitor_device_name: None,
            a1_equalizer: None,
            main_mix: VoicemeeterMixConfiguration {
                monitor_enabled: true,
                virtual_microphone_enabled: true,
                input_muted: false,
                virtual_microphone_muted: false,
                input_gain_db: 0.0,
                virtual_microphone_gain_db: 0.0,
            },
            aux_mix: None,
            physical_input: VoicemeeterPhysicalInputConfiguration {
                device_name: None,
                muted: false,
                gain_db: 0.0,
                monitor_enabled: false,
                main_mix_enabled: false,
                aux_mix_enabled: false,
                audibility: None,
                compressor: None,
                noise_gate: None,
                denoiser: None,
                equalizer: None,
            },
        }
    }

    #[test]
    fn maps_supported_editions_to_main_vaio_and_b1() {
        let standard = EditionLayout::from_code(1).unwrap();
        assert_eq!(standard.main_mix.virtual_input_strip, 2);
        assert_eq!(standard.main_mix.virtual_microphone_bus, 1);
        assert!(standard.aux_mix.is_none());
        assert!(!standard.supports_bus_equalizer());
        assert!(!standard.supports_denoiser());

        let banana = EditionLayout::from_code(2).unwrap();
        assert_eq!(banana.main_mix.virtual_input_strip, 3);
        assert_eq!(banana.main_mix.virtual_microphone_bus, 3);
        assert_eq!(banana.aux_mix.unwrap().virtual_input_strip, 4);
        assert_eq!(banana.aux_mix.unwrap().virtual_microphone_bus, 4);
        assert!(banana.supports_bus_equalizer());
        assert!(!banana.supports_denoiser());
        assert!(!banana.supports_strip_equalizer());

        let potato = EditionLayout::from_code(3).unwrap();
        assert_eq!(potato.main_mix.virtual_input_strip, 5);
        assert_eq!(potato.main_mix.virtual_microphone_bus, 5);
        assert_eq!(potato.aux_mix.unwrap().virtual_input_strip, 6);
        assert_eq!(potato.aux_mix.unwrap().virtual_microphone_bus, 6);
        assert!(potato.supports_bus_equalizer());
        assert!(potato.supports_denoiser());
        assert!(potato.supports_strip_equalizer());
    }

    #[test]
    fn rejects_out_of_range_gain() {
        let mut configuration = valid_configuration();
        configuration.main_mix.input_gain_db = 13.0;
        assert!(validate_configuration(&configuration).is_err());
    }

    #[test]
    fn rejects_out_of_range_dsp_strength() {
        let mut configuration = valid_configuration();
        configuration.physical_input.noise_gate = Some(10.1);
        assert!(validate_configuration(&configuration).is_err());
    }

    #[test]
    fn rejects_incomplete_equalizer() {
        let mut configuration = valid_configuration();
        configuration.a1_equalizer = Some(VoicemeeterEqConfiguration {
            enabled: true,
            bands: Vec::new(),
        });
        assert!(validate_configuration(&configuration).is_err());
    }
}
