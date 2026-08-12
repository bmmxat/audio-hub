use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crate::plugins::voicemeeter::{VoicemeeterConfiguration, VoicemeeterManager};

use super::{
    device::{AudioDevice, DeviceDirection},
    wasapi,
};

const STATE_FILE_NAME: &str = "simple-route-session.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleRouteApplication {
    pub key: String,
    pub display_name: String,
    pub pid: u32,
    pub original_output_device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SimpleRouteSession {
    original_default_input_id: String,
    virtual_microphone_id: String,
    voicemeeter_input_id: String,
    original_voicemeeter_configuration: VoicemeeterConfiguration,
    physical_microphone_name: String,
    #[serde(default)]
    monitor_device_name: Option<String>,
    applications: Vec<SimpleRouteApplication>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimpleRouteStatus {
    pub active: bool,
    pub applications: Vec<SimpleRouteApplication>,
    pub physical_microphone_name: Option<String>,
    pub monitor_device_name: Option<String>,
    pub virtual_microphone_id: Option<String>,
    pub voicemeeter_input_id: Option<String>,
    pub recovery_pending: bool,
}

pub struct SimpleRouteManager {
    session: Mutex<Option<SimpleRouteSession>>,
    state_path: PathBuf,
    recovery_pending: AtomicBool,
}

impl SimpleRouteManager {
    pub fn load(app_data_dir: &Path) -> Self {
        let state_path = app_data_dir.join(STATE_FILE_NAME);
        let session = fs::read_to_string(&state_path)
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok());
        Self {
            recovery_pending: AtomicBool::new(session.is_some()),
            session: Mutex::new(session),
            state_path,
        }
    }

    pub fn status(&self) -> SimpleRouteStatus {
        let guard = self
            .session
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match guard.as_ref() {
            Some(session) => SimpleRouteStatus {
                active: true,
                applications: session.applications.clone(),
                physical_microphone_name: Some(session.physical_microphone_name.clone()),
                monitor_device_name: session.monitor_device_name.clone(),
                virtual_microphone_id: Some(session.virtual_microphone_id.clone()),
                voicemeeter_input_id: Some(session.voicemeeter_input_id.clone()),
                recovery_pending: self.recovery_pending.load(Ordering::Relaxed),
            },
            None => SimpleRouteStatus {
                active: false,
                applications: Vec::new(),
                physical_microphone_name: None,
                monitor_device_name: None,
                virtual_microphone_id: None,
                voicemeeter_input_id: None,
                recovery_pending: false,
            },
        }
    }

    pub fn prepare(&self, voicemeeter: &VoicemeeterManager) -> Result<SimpleRouteStatus, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "简易流转状态已损坏。".to_string())?;
        if guard.is_some() {
            return self.status_with_guard(&guard);
        }

        let session = create_prepared_session(voicemeeter)?;
        if let Err(error) = self.save_session(&session) {
            let _ = restore_default_input(&session);
            let _ = voicemeeter.restore(session.original_voicemeeter_configuration.clone());
            return Err(error);
        }
        *guard = Some(session);
        self.recovery_pending.store(false, Ordering::Relaxed);
        self.status_with_guard(&guard)
    }

    pub fn enable_application(
        &self,
        pid: u32,
        key: String,
        display_name: String,
        voicemeeter: &VoicemeeterManager,
    ) -> Result<SimpleRouteStatus, String> {
        validate_application(pid, &key, &display_name)?;
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "简易流转状态已损坏。".to_string())?;
        if guard.is_none() {
            drop(guard);
            self.prepare(voicemeeter)?;
            guard = self
                .session
                .lock()
                .map_err(|_| "简易流转状态已损坏。".to_string())?;
        }
        let session = guard
            .as_mut()
            .ok_or_else(|| "简易流转尚未准备完成。".to_string())?;
        if session
            .applications
            .iter()
            .any(|application| application.key == key)
        {
            return Ok(status_from_session(
                session,
                self.recovery_pending.load(Ordering::Relaxed),
            ));
        }
        let original_output_device_id = wasapi::get_app_output_device(pid)
            .map_err(|error| format!("无法读取应用原输出设备：{error:?}"))?;
        wasapi::set_app_output_device(pid, &session.voicemeeter_input_id)
            .map_err(|error| format!("无法将应用加入简易流转：{error:?}"))?;
        session.applications.push(SimpleRouteApplication {
            key,
            display_name,
            pid,
            original_output_device_id: original_output_device_id.clone(),
        });
        if let Err(error) = self.save_session(session) {
            let restore_id = original_output_device_id.as_deref().unwrap_or_default();
            let _ = wasapi::set_app_output_device(pid, restore_id);
            session.applications.pop();
            return Err(error);
        }
        self.recovery_pending.store(false, Ordering::Relaxed);
        self.status_with_guard(&guard)
    }

    pub fn disable_application(
        &self,
        key: &str,
        current_pid: Option<u32>,
    ) -> Result<SimpleRouteStatus, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "简易流转状态已损坏。".to_string())?;
        let Some(session) = guard.as_mut() else {
            return Ok(inactive_status());
        };
        let index = session
            .applications
            .iter()
            .position(|application| application.key == key)
            .ok_or_else(|| "该应用未启用简易流转。".to_string())?;
        let application = session.applications[index].clone();
        let pid = current_pid
            .filter(|pid| *pid != 0)
            .unwrap_or(application.pid);
        let restore_id = application
            .original_output_device_id
            .as_deref()
            .unwrap_or_default();
        wasapi::set_app_output_device(pid, restore_id)
            .map_err(|error| format!("无法恢复应用原输出设备：{error:?}"))?;

        session.applications.remove(index);
        self.save_session(session)?;
        self.recovery_pending.store(false, Ordering::Relaxed);
        Ok(status_from_session(session, false))
    }

    pub fn sync_monitor_to_default(
        &self,
        voicemeeter: &VoicemeeterManager,
    ) -> Result<SimpleRouteStatus, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "简易流转状态已损坏。".to_string())?;
        let Some(session) = guard.as_mut() else {
            return Ok(inactive_status());
        };
        let output_devices = wasapi::enumerate_devices(DeviceDirection::Output)
            .map_err(|error| format!("无法读取输出设备：{error:?}"))?;
        let monitor = find_default_physical_output(&output_devices).ok_or_else(|| {
            "新的 Windows 默认输出不是物理扬声器或耳机，A1 已保持不变以避免音频回路。".to_string()
        })?;
        if session.monitor_device_name.as_deref() == Some(monitor.name.as_str()) {
            return Ok(status_from_session(session, false));
        }

        let previous_monitor = session.monitor_device_name.clone().or_else(|| {
            session
                .original_voicemeeter_configuration
                .monitor_device_name
                .clone()
        });
        voicemeeter
            .set_monitor_device(&monitor.name)
            .map_err(|error| format!("无法同步 VoiceMeeter A1：{error}"))?;
        session.monitor_device_name = Some(monitor.name.clone());
        if let Err(error) = self.save_session(session) {
            if let Some(previous_monitor_name) = previous_monitor.as_deref() {
                let _ = voicemeeter.set_monitor_device(previous_monitor_name);
            }
            session.monitor_device_name = previous_monitor;
            return Err(error);
        }
        self.recovery_pending.store(false, Ordering::Relaxed);
        Ok(status_from_session(session, false))
    }

    pub fn stop_all(&self, voicemeeter: &VoicemeeterManager) -> Result<SimpleRouteStatus, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "简易流转状态已损坏。".to_string())?;
        let Some(session) = guard.as_ref() else {
            return Ok(inactive_status());
        };
        for application in &session.applications {
            let restore_id = application
                .original_output_device_id
                .as_deref()
                .unwrap_or_default();
            wasapi::set_app_output_device(application.pid, restore_id).map_err(|error| {
                format!("无法恢复 {} 的输出：{error:?}", application.display_name)
            })?;
        }
        restore_default_input(session)?;
        voicemeeter
            .restore(session.original_voicemeeter_configuration.clone())
            .map_err(|error| format!("无法恢复 VoiceMeeter 原配置：{error}"))?;
        *guard = None;
        self.recovery_pending.store(false, Ordering::Relaxed);
        self.remove_state_file()?;
        Ok(inactive_status())
    }

    fn save_session(&self, session: &SimpleRouteSession) -> Result<(), String> {
        if let Some(parent) = self.state_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建简易流转配置目录：{error}"))?;
        }
        let contents = serde_json::to_vec_pretty(session)
            .map_err(|error| format!("无法序列化简易流转状态：{error}"))?;
        fs::write(&self.state_path, contents)
            .map_err(|error| format!("无法保存简易流转恢复状态：{error}"))
    }

    fn remove_state_file(&self) -> Result<(), String> {
        match fs::remove_file(&self.state_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("无法清理简易流转恢复状态：{error}")),
        }
    }

    fn status_with_guard(
        &self,
        guard: &Option<SimpleRouteSession>,
    ) -> Result<SimpleRouteStatus, String> {
        Ok(guard
            .as_ref()
            .map(|session| status_from_session(session, false))
            .unwrap_or_else(inactive_status))
    }
}

fn create_prepared_session(voicemeeter: &VoicemeeterManager) -> Result<SimpleRouteSession, String> {
    let vm_status = voicemeeter.status();
    if !vm_status.connected {
        return Err("VoiceMeeter 尚未运行或 Remote API 未连接。".to_string());
    }
    let original_configuration = vm_status
        .configuration
        .ok_or_else(|| "无法读取 VoiceMeeter 当前配置。".to_string())?;
    let output_devices = wasapi::enumerate_devices(DeviceDirection::Output)
        .map_err(|error| format!("无法读取输出设备：{error:?}"))?;
    let input_devices = wasapi::enumerate_devices(DeviceDirection::Input)
        .map_err(|error| format!("无法读取输入设备：{error:?}"))?;
    let voicemeeter_input = find_main_voicemeeter_input(&output_devices)
        .ok_or_else(|| "未检测到 VoiceMeeter Input 播放设备。".to_string())?;
    let virtual_microphone = find_b1_virtual_microphone(&input_devices)
        .ok_or_else(|| "未检测到 VoiceMeeter B1 虚拟麦克风。".to_string())?;
    let original_default_input = input_devices
        .iter()
        .find(|device| device.is_default)
        .ok_or_else(|| "未检测到 Windows 默认麦克风。".to_string())?;
    let physical_microphone = find_physical_microphone(
        &input_devices,
        original_configuration.physical_input.device_name.as_deref(),
    )
    .ok_or_else(|| "未检测到可用的物理麦克风。".to_string())?;
    let monitor = find_default_physical_output(&output_devices).ok_or_else(|| {
        "未检测到 Windows 默认物理扬声器。请先在 Audio Hub 或 Windows 中将实际扬声器/耳机设为默认输出。"
            .to_string()
    })?;

    let mut requested = original_configuration.clone();
    requested.monitor_device_name = Some(monitor.name.clone());
    requested.main_mix.monitor_enabled = true;
    requested.main_mix.virtual_microphone_enabled = true;
    requested.main_mix.input_muted = false;
    requested.main_mix.virtual_microphone_muted = false;
    requested.physical_input.device_name = Some(physical_microphone.name.clone());
    requested.physical_input.muted = false;
    requested.physical_input.monitor_enabled = false;
    requested.physical_input.main_mix_enabled = true;

    voicemeeter
        .apply(requested)
        .map_err(|error| format!("无法准备 VoiceMeeter 简易混音：{error}"))?;
    if let Err(error) = wasapi::set_default_device(&virtual_microphone.device_id) {
        let _ = voicemeeter.restore(original_configuration.clone());
        return Err(format!("无法将虚拟麦克风设为默认设备：{error:?}"));
    }
    if !wait_for_default_input(&virtual_microphone.device_id) {
        let _ = wasapi::set_default_device(&original_default_input.device_id);
        let _ = voicemeeter.restore(original_configuration.clone());
        return Err("Windows 未确认默认麦克风切换，已恢复原设置。".to_string());
    }

    Ok(SimpleRouteSession {
        original_default_input_id: original_default_input.device_id.clone(),
        virtual_microphone_id: virtual_microphone.device_id.clone(),
        voicemeeter_input_id: voicemeeter_input.device_id.clone(),
        original_voicemeeter_configuration: original_configuration,
        physical_microphone_name: physical_microphone.name.clone(),
        monitor_device_name: Some(monitor.name.clone()),
        applications: Vec::new(),
    })
}

fn restore_default_input(session: &SimpleRouteSession) -> Result<(), String> {
    let current_default = wasapi::enumerate_devices(DeviceDirection::Input)
        .map_err(|error| format!("无法读取当前默认麦克风：{error:?}"))?
        .into_iter()
        .find(|device| device.is_default);
    if current_default
        .as_ref()
        .is_some_and(|device| device.device_id == session.virtual_microphone_id)
    {
        wasapi::set_default_device(&session.original_default_input_id)
            .map_err(|error| format!("无法恢复原默认麦克风：{error:?}"))?;
    }
    Ok(())
}

fn wait_for_default_input(device_id: &str) -> bool {
    for _ in 0..6 {
        thread::sleep(Duration::from_millis(100));
        if wasapi::enumerate_devices(DeviceDirection::Input)
            .ok()
            .is_some_and(|devices| {
                devices
                    .iter()
                    .any(|device| device.is_default && device.device_id == device_id)
            })
        {
            return true;
        }
    }
    false
}

fn find_main_voicemeeter_input(devices: &[AudioDevice]) -> Option<&AudioDevice> {
    devices
        .iter()
        .find(|device| {
            let name = device.name.to_ascii_lowercase();
            name.contains("voicemeeter input") && !name.contains("aux")
        })
        .or_else(|| {
            devices.iter().find(|device| {
                let name = device.name.to_ascii_lowercase();
                is_voicemeeter_name(&name)
                    && (name.contains("input") || name.contains("vaio"))
                    && !name.contains("aux")
                    && !name.contains("vaio3")
            })
        })
}

fn find_b1_virtual_microphone(devices: &[AudioDevice]) -> Option<&AudioDevice> {
    devices
        .iter()
        .find(|device| {
            let name = device.name.to_ascii_lowercase();
            is_voicemeeter_name(&name) && name.contains("out b1")
        })
        .or_else(|| {
            devices.iter().find(|device| {
                let name = device.name.to_ascii_lowercase();
                is_voicemeeter_name(&name)
                    && name.contains("output")
                    && !name.contains("aux")
                    && !name.contains("b2")
                    && !name.contains("b3")
            })
        })
}

fn find_physical_microphone<'a>(
    devices: &'a [AudioDevice],
    configured_name: Option<&str>,
) -> Option<&'a AudioDevice> {
    devices
        .iter()
        .find(|device| device.is_default && !is_virtual_audio_device(&device.name))
        .or_else(|| {
            configured_name.and_then(|name| {
                devices.iter().find(|device| {
                    device.name.eq_ignore_ascii_case(name) && !is_virtual_audio_device(&device.name)
                })
            })
        })
        .or_else(|| {
            devices
                .iter()
                .find(|device| !is_virtual_audio_device(&device.name))
        })
}

fn find_default_physical_output(devices: &[AudioDevice]) -> Option<&AudioDevice> {
    devices
        .iter()
        .find(|device| device.is_default && !is_virtual_audio_device(&device.name))
}

fn is_voicemeeter_name(name: &str) -> bool {
    name.contains("voicemeeter") || name.contains("vb-audio")
}

fn is_virtual_audio_device(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    is_voicemeeter_name(&name) || name.contains("virtual") || name.contains("steam streaming")
}

fn validate_application(pid: u32, key: &str, display_name: &str) -> Result<(), String> {
    if pid == 0 {
        return Err("系统声音不能使用简易流转。".to_string());
    }
    if key.trim().is_empty() || key.len() > 512 {
        return Err("应用标识无效。".to_string());
    }
    if display_name.trim().is_empty() || display_name.len() > 256 {
        return Err("应用名称无效。".to_string());
    }
    Ok(())
}

fn status_from_session(session: &SimpleRouteSession, recovery_pending: bool) -> SimpleRouteStatus {
    SimpleRouteStatus {
        active: true,
        applications: session.applications.clone(),
        physical_microphone_name: Some(session.physical_microphone_name.clone()),
        monitor_device_name: session.monitor_device_name.clone(),
        virtual_microphone_id: Some(session.virtual_microphone_id.clone()),
        voicemeeter_input_id: Some(session.voicemeeter_input_id.clone()),
        recovery_pending,
    }
}

fn inactive_status() -> SimpleRouteStatus {
    SimpleRouteStatus {
        active: false,
        applications: Vec::new(),
        physical_microphone_name: None,
        monitor_device_name: None,
        virtual_microphone_id: None,
        voicemeeter_input_id: None,
        recovery_pending: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, is_default: bool) -> AudioDevice {
        AudioDevice {
            name: name.to_string(),
            device_id: name.to_string(),
            is_default,
        }
    }

    #[test]
    fn discovers_main_input_and_b1_across_common_names() {
        let outputs = vec![
            device(
                "VoiceMeeter AUX Input (VB-Audio VoiceMeeter AUX VAIO)",
                false,
            ),
            device("VoiceMeeter Input (VB-Audio VoiceMeeter VAIO)", false),
        ];
        let inputs = vec![
            device("VoiceMeeter Out B2 (VB-Audio VoiceMeeter AUX VAIO)", false),
            device("VoiceMeeter Out B1 (VB-Audio VoiceMeeter VAIO)", false),
        ];
        assert!(
            find_main_voicemeeter_input(&outputs)
                .unwrap()
                .name
                .contains("Input")
        );
        assert!(
            find_b1_virtual_microphone(&inputs)
                .unwrap()
                .name
                .contains("B1")
        );
    }

    #[test]
    fn prefers_default_physical_microphone() {
        let inputs = vec![
            device("VoiceMeeter Out B1", true),
            device("USB Microphone", false),
            device("Headset Microphone", false),
        ];
        assert_eq!(
            find_physical_microphone(&inputs, Some("Headset Microphone"))
                .unwrap()
                .name,
            "Headset Microphone"
        );
    }

    #[test]
    fn requires_default_output_to_be_physical_for_simple_route() {
        let virtual_default = vec![
            device("VoiceMeeter Input", true),
            device("USB Speakers", false),
        ];
        assert!(find_default_physical_output(&virtual_default).is_none());

        let physical_default = vec![
            device("VoiceMeeter Input", false),
            device("USB Speakers", true),
        ];
        assert_eq!(
            find_default_physical_output(&physical_default)
                .unwrap()
                .name,
            "USB Speakers"
        );
    }
}
