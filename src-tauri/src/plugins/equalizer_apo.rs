use std::{
    collections::BTreeMap,
    ffi::{OsStr, c_void},
    fs,
    io::Write,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use windows::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
        System::Registry::{
            HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW,
        },
        UI::{
            Shell::{
                FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog,
                IFileOpenDialog, SIGDN_FILESYSPATH, ShellExecuteW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::PCWSTR,
};

const REGISTRY_KEY: &str = r"SOFTWARE\EqualizerAPO";
const REGISTRY_VALUE: &str = "ConfigPath";
const CHILD_APOS_REGISTRY_KEY: &str = r"SOFTWARE\EqualizerAPO\Child APOs";
const MANAGED_FILE_NAME: &str = "audio-hub.txt";
const MAIN_CONFIG_NAME: &str = "config.txt";
const BACKUP_FILE_NAME: &str = "config.audio-hub.backup.txt";
const BEGIN_MARKER: &str = "# >>> Audio Hub Equalizer APO plugin >>>";
const END_MARKER: &str = "# <<< Audio Hub Equalizer APO plugin <<<";
const GRAPHIC_EQ_FREQUENCIES: [f32; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];
const MAX_EQ_BANDS: usize = 10;
const MIN_FREQUENCY_HZ: f32 = 20.0;
const MAX_FREQUENCY_HZ: f32 = 20_000.0;
const MIN_GAIN_DB: f32 = -18.0;
const MAX_GAIN_DB: f32 = 18.0;
const MIN_Q: f32 = 0.1;
const MAX_Q: f32 = 12.0;
const MIN_PREAMP_DB: f32 = -24.0;
const MAX_PREAMP_DB: f32 = 12.0;
const MIN_MIC_GAIN_DB: f32 = -12.0;
const MAX_MIC_GAIN_DB: f32 = 18.0;
const RNNOISE_MONO_FILE_NAME: &str = "rnnoise_mono.dll";
const RNNOISE_STEREO_FILE_NAME: &str = "rnnoise_stereo.dll";

#[derive(Debug, Clone, Serialize)]
pub struct EqualizerApoStatus {
    pub installed: bool,
    pub connected: bool,
    pub config_path: Option<String>,
    pub managed_config_path: Option<String>,
    pub configurator_path: Option<String>,
    pub backup_exists: bool,
    pub rnnoise_mono_path: Option<String>,
    pub rnnoise_stereo_path: Option<String>,
    pub rnnoise_plugin_directory: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqFilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBandConfig {
    pub kind: EqFilterKind,
    pub frequency_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalEqConfig {
    pub enabled: bool,
    pub preamp_db: f32,
    pub auto_headroom: bool,
    pub bands: Vec<EqBandConfig>,
}

impl Default for GlobalEqConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.0,
            auto_headroom: true,
            bands: graphic_eq_bands([0.0; 10]),
        }
    }
}

impl GlobalEqConfig {
    fn validate(&self) -> Result<(), String> {
        if !self.preamp_db.is_finite() || !(MIN_PREAMP_DB..=MAX_PREAMP_DB).contains(&self.preamp_db)
        {
            return Err(format!(
                "EQ 前级增益必须在 {MIN_PREAMP_DB} dB 到 {MAX_PREAMP_DB} dB 之间"
            ));
        }
        if self.bands.len() > MAX_EQ_BANDS {
            return Err(format!("EQ 最多支持 {MAX_EQ_BANDS} 个频段"));
        }
        for (index, band) in self.bands.iter().enumerate() {
            let number = index + 1;
            if !band.frequency_hz.is_finite()
                || !(MIN_FREQUENCY_HZ..=MAX_FREQUENCY_HZ).contains(&band.frequency_hz)
            {
                return Err(format!(
                    "EQ 第 {number} 段频率必须在 {MIN_FREQUENCY_HZ} Hz 到 {MAX_FREQUENCY_HZ} Hz 之间"
                ));
            }
            if !band.gain_db.is_finite() || !(MIN_GAIN_DB..=MAX_GAIN_DB).contains(&band.gain_db) {
                return Err(format!(
                    "EQ 第 {number} 段增益必须在 {MIN_GAIN_DB} dB 到 {MAX_GAIN_DB} dB 之间"
                ));
            }
            if !band.q.is_finite() || !(MIN_Q..=MAX_Q).contains(&band.q) {
                return Err(format!(
                    "EQ 第 {number} 段 Q 值必须在 {MIN_Q} 到 {MAX_Q} 之间"
                ));
            }
        }
        Ok(())
    }

    pub fn effective_preamp_db(&self) -> f32 {
        if !self.auto_headroom {
            return self.preamp_db;
        }
        let largest_boost = self
            .bands
            .iter()
            .filter(|band| band.enabled)
            .map(|band| band.gain_db)
            .fold(0.0_f32, f32::max);
        self.preamp_db.min(-largest_boost)
    }

    fn normalize_graphic_bands(&mut self) {
        if self.bands.len() == GRAPHIC_EQ_FREQUENCIES.len()
            && self
                .bands
                .iter()
                .zip(GRAPHIC_EQ_FREQUENCIES)
                .all(|(band, frequency)| (band.frequency_hz - frequency).abs() < 0.1)
        {
            return;
        }

        let mut source: Vec<_> = self
            .bands
            .iter()
            .filter(|band| band.enabled)
            .map(|band| (band.frequency_hz, band.gain_db))
            .collect();
        source.sort_by(|left, right| left.0.total_cmp(&right.0));
        let gains = GRAPHIC_EQ_FREQUENCIES.map(|frequency| interpolate_gain(&source, frequency));
        self.bands = graphic_eq_bands(gains);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RnnoiseChannelMode {
    #[default]
    Mono,
    Stereo,
}

impl RnnoiseChannelMode {
    fn display_name(self) -> &'static str {
        match self {
            Self::Mono => "单声道",
            Self::Stereo => "立体声",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Mono => RNNOISE_MONO_FILE_NAME,
            Self::Stereo => RNNOISE_STEREO_FILE_NAME,
        }
    }
}

#[derive(Debug, Default)]
struct RnnoisePlugins {
    mono: Option<PathBuf>,
    stereo: Option<PathBuf>,
}

impl RnnoisePlugins {
    fn path_for(&self, mode: RnnoiseChannelMode) -> Option<&Path> {
        match mode {
            RnnoiseChannelMode::Mono => self.mono.as_deref(),
            RnnoiseChannelMode::Stereo => self.stereo.as_deref(),
        }
    }

    fn directory(&self) -> Option<PathBuf> {
        self.mono
            .as_deref()
            .or(self.stereo.as_deref())
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MicrophoneConfig {
    pub enabled: bool,
    pub gain_db: f32,
    pub rnnoise_enabled: bool,
    pub rnnoise_mode: RnnoiseChannelMode,
    #[serde(skip_serializing)]
    high_pass_enabled: Option<bool>,
    #[serde(skip_serializing)]
    high_pass_hz: Option<f32>,
    #[serde(skip_serializing)]
    hiss_reduction_db: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MicrophoneConfigState {
    pub config: MicrophoneConfig,
    pub configured: bool,
}

impl Default for MicrophoneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gain_db: 8.0,
            rnnoise_enabled: true,
            rnnoise_mode: RnnoiseChannelMode::Mono,
            high_pass_enabled: None,
            high_pass_hz: None,
            hiss_reduction_db: None,
        }
    }
}

impl MicrophoneConfig {
    fn validate(&self) -> Result<(), String> {
        if !self.gain_db.is_finite() || !(MIN_MIC_GAIN_DB..=MAX_MIC_GAIN_DB).contains(&self.gain_db)
        {
            return Err(format!(
                "麦克风增益必须在 {MIN_MIC_GAIN_DB} dB 到 {MAX_MIC_GAIN_DB} dB 之间"
            ));
        }
        Ok(())
    }

    fn discard_legacy_noise_filters(&mut self) -> bool {
        let migrated = self.high_pass_enabled.is_some()
            || self.high_pass_hz.is_some()
            || self.hiss_reduction_db.is_some();
        self.high_pass_enabled = None;
        self.high_pass_hz = None;
        self.hiss_reduction_db = None;
        migrated
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointProfile {
    device_name: String,
    config: GlobalEqConfig,
    #[serde(default)]
    presets: BTreeMap<String, GlobalEqConfig>,
    #[serde(default)]
    active_preset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MicrophoneProfile {
    device_name: String,
    config: MicrophoneConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct EqPresetCatalog {
    pub active_preset: String,
    pub presets: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProfileStore {
    #[serde(default)]
    endpoints: BTreeMap<String, EndpointProfile>,
    #[serde(default)]
    microphones: BTreeMap<String, MicrophoneProfile>,
    #[serde(default)]
    rnnoise_plugin_directory: Option<String>,
}

pub fn status(app_data_dir: Option<&Path>) -> EqualizerApoStatus {
    let Some(config_path) = detect_config_path() else {
        return EqualizerApoStatus {
            installed: false,
            connected: false,
            config_path: None,
            managed_config_path: None,
            configurator_path: None,
            backup_exists: false,
            rnnoise_mono_path: None,
            rnnoise_stereo_path: None,
            rnnoise_plugin_directory: None,
        };
    };

    let main_config = config_path.join(MAIN_CONFIG_NAME);
    let connected = fs::read_to_string(&main_config)
        .map(|contents| managed_block_range(&contents).is_some())
        .unwrap_or(false);
    let configurator = find_device_configurator(&config_path);
    let configured_plugin_directory = app_data_dir
        .and_then(|directory| load_store(directory).ok())
        .and_then(|store| store.rnnoise_plugin_directory)
        .and_then(valid_plugin_directory);
    let plugins = detect_rnnoise_plugins(&config_path, configured_plugin_directory.as_deref());
    let visible_plugin_directory = configured_plugin_directory.or_else(|| plugins.directory());

    EqualizerApoStatus {
        installed: true,
        connected,
        managed_config_path: Some(path_text(&config_path.join(MANAGED_FILE_NAME))),
        configurator_path: configurator.as_deref().map(path_text),
        backup_exists: config_path.join(BACKUP_FILE_NAME).is_file(),
        config_path: Some(path_text(&config_path)),
        rnnoise_mono_path: plugins.mono.as_deref().map(path_text),
        rnnoise_stereo_path: plugins.stereo.as_deref().map(path_text),
        rnnoise_plugin_directory: visible_plugin_directory.as_deref().map(path_text),
    }
}

pub fn enabled_device_ids(device_ids: Vec<String>) -> Vec<String> {
    device_ids
        .into_iter()
        .filter(|device_id| equalizer_apo_enabled_for_device(device_id))
        .collect()
}

pub fn choose_rnnoise_plugin_directory(
    app_data_dir: &Path,
) -> Result<Option<EqualizerApoStatus>, String> {
    let Some(directory) = pick_rnnoise_plugin_directory()? else {
        return Ok(None);
    };
    let direct_plugins = RnnoisePlugins {
        mono: directory
            .join(RNNOISE_MONO_FILE_NAME)
            .is_file()
            .then(|| directory.join(RNNOISE_MONO_FILE_NAME)),
        stereo: directory
            .join(RNNOISE_STEREO_FILE_NAME)
            .is_file()
            .then(|| directory.join(RNNOISE_STEREO_FILE_NAME)),
    };
    if direct_plugins.mono.is_none() && direct_plugins.stereo.is_none() {
        return Err(format!(
            "所选文件夹中没有 {} 或 {}。",
            RNNOISE_MONO_FILE_NAME, RNNOISE_STEREO_FILE_NAME
        ));
    }

    let mut store = load_store(app_data_dir)?;
    store.rnnoise_plugin_directory = Some(path_text(&directory));
    if status(None).connected {
        let config_path =
            detect_config_path().ok_or_else(|| "未检测到 Equalizer APO。".to_string())?;
        let plugins = detect_rnnoise_plugins(&config_path, Some(&directory));
        render_managed_config(&store, &plugins)?;
    }
    save_store(app_data_dir, &store)?;
    if status(None).connected {
        write_managed_config(&store)?;
    }
    Ok(Some(status(Some(app_data_dir))))
}

pub fn get_endpoint_eq(app_data_dir: &Path, device_id: &str) -> Result<GlobalEqConfig, String> {
    validate_device_id(device_id)?;
    let mut config = load_store(app_data_dir)?
        .endpoints
        .get(device_id)
        .map(|profile| profile.config.clone())
        .unwrap_or_default();
    config.normalize_graphic_bands();
    Ok(config)
}

pub fn get_microphone_config(
    app_data_dir: &Path,
    device_id: &str,
) -> Result<MicrophoneConfigState, String> {
    validate_device_id(device_id)?;
    let mut store = load_store(app_data_dir)?;
    let migrated = store
        .microphones
        .get_mut(device_id)
        .is_some_and(|profile| profile.config.discard_legacy_noise_filters());
    if migrated {
        save_store(app_data_dir, &store)?;
        if status(None).connected {
            write_managed_config(&store)?;
        }
    }
    let profile = store.microphones.get(device_id);
    Ok(MicrophoneConfigState {
        config: profile
            .map(|profile| profile.config.clone())
            .unwrap_or_default(),
        configured: profile.is_some(),
    })
}

pub fn set_microphone_config(
    app_data_dir: &Path,
    device_id: &str,
    device_name: &str,
    config: MicrophoneConfig,
) -> Result<MicrophoneConfig, String> {
    validate_device_id(device_id)?;
    validate_device_name(device_name)?;
    require_equalizer_apo_enabled(device_id)?;
    config.validate()?;
    let mut store = load_store(app_data_dir)?;
    if config.enabled && config.rnnoise_enabled {
        let config_path =
            detect_config_path().ok_or_else(|| "未检测到 Equalizer APO。".to_string())?;
        let configured_directory = store
            .rnnoise_plugin_directory
            .as_deref()
            .and_then(|value| valid_plugin_directory(value.to_string()));
        let plugins = detect_rnnoise_plugins(&config_path, configured_directory.as_deref());
        plugins.path_for(config.rnnoise_mode).ok_or_else(|| {
            format!(
                "未检测到 RNNoise {}插件。请将 {} 放入 Equalizer APO 的 VSTPlugins\\AudioHub 文件夹。",
                config.rnnoise_mode.display_name(),
                config.rnnoise_mode.file_name()
            )
        })?;
    }

    store.microphones.insert(
        device_id.to_string(),
        MicrophoneProfile {
            device_name: device_name.to_string(),
            config: config.clone(),
        },
    );
    save_store(app_data_dir, &store)?;
    if status(None).connected {
        write_managed_config(&store)?;
    }
    Ok(config)
}

pub fn set_endpoint_eq(
    app_data_dir: &Path,
    device_id: &str,
    device_name: &str,
    mut config: GlobalEqConfig,
) -> Result<GlobalEqConfig, String> {
    validate_device_id(device_id)?;
    validate_device_name(device_name)?;
    require_equalizer_apo_enabled(device_id)?;
    // 设备级开关已从界面移除：保存过的设备配置在插件连接时始终启用。
    config.enabled = true;
    config.normalize_graphic_bands();
    config.validate()?;

    let mut store = load_store(app_data_dir)?;
    let profile = store
        .endpoints
        .entry(device_id.to_string())
        .or_insert_with(|| EndpointProfile {
            device_name: device_name.to_string(),
            config: config.clone(),
            presets: BTreeMap::new(),
            active_preset: None,
        });
    migrate_endpoint_profile(profile);
    let preset_name = profile
        .active_preset
        .clone()
        .unwrap_or_else(|| "当前音色".to_string());
    profile.device_name = device_name.to_string();
    profile.config = config.clone();
    profile.presets.insert(preset_name.clone(), config.clone());
    profile.active_preset = Some(preset_name);
    save_store(app_data_dir, &store)?;

    if status(None).connected {
        write_managed_config(&store)?;
    }
    Ok(config)
}

pub fn list_presets(app_data_dir: &Path, device_id: &str) -> Result<EqPresetCatalog, String> {
    validate_device_id(device_id)?;
    let store = load_store(app_data_dir)?;
    if let Some(profile) = store.endpoints.get(device_id) {
        let active_preset = profile
            .active_preset
            .clone()
            .unwrap_or_else(|| "当前音色".to_string());
        return Ok(EqPresetCatalog {
            active_preset,
            presets: profile.presets.keys().cloned().collect(),
        });
    }
    Ok(EqPresetCatalog {
        active_preset: "默认".to_string(),
        presets: vec!["默认".to_string()],
    })
}

pub fn get_preset(
    app_data_dir: &Path,
    device_id: &str,
    preset_name: &str,
) -> Result<GlobalEqConfig, String> {
    validate_device_id(device_id)?;
    validate_preset_name(preset_name)?;
    let store = load_store(app_data_dir)?;
    if let Some(profile) = store.endpoints.get(device_id) {
        return profile
            .presets
            .get(preset_name)
            .cloned()
            .ok_or_else(|| format!("音色预设「{preset_name}」不存在"));
    }
    if preset_name == "默认" {
        Ok(GlobalEqConfig::default())
    } else {
        Err(format!("音色预设「{preset_name}」不存在"))
    }
}

pub fn save_preset(
    app_data_dir: &Path,
    device_id: &str,
    device_name: &str,
    preset_name: &str,
    mut config: GlobalEqConfig,
) -> Result<GlobalEqConfig, String> {
    validate_device_id(device_id)?;
    validate_device_name(device_name)?;
    validate_preset_name(preset_name)?;
    require_equalizer_apo_enabled(device_id)?;
    config.enabled = true;
    config.normalize_graphic_bands();
    config.validate()?;

    let mut store = load_store(app_data_dir)?;
    let profile = store
        .endpoints
        .entry(device_id.to_string())
        .or_insert_with(|| EndpointProfile {
            device_name: device_name.to_string(),
            config: config.clone(),
            presets: BTreeMap::from([(preset_name.to_string(), config.clone())]),
            active_preset: Some(preset_name.to_string()),
        });
    migrate_endpoint_profile(profile);
    profile.device_name = device_name.to_string();
    profile.config = config.clone();
    profile
        .presets
        .insert(preset_name.to_string(), config.clone());
    profile.active_preset = Some(preset_name.to_string());
    save_store(app_data_dir, &store)?;
    if status(None).connected {
        write_managed_config(&store)?;
    }
    Ok(config)
}

pub fn activate_preset(
    app_data_dir: &Path,
    device_id: &str,
    preset_name: &str,
) -> Result<GlobalEqConfig, String> {
    validate_device_id(device_id)?;
    validate_preset_name(preset_name)?;
    let mut store = load_store(app_data_dir)?;
    let profile = store
        .endpoints
        .get_mut(device_id)
        .ok_or_else(|| "该输出设备还没有保存音色预设".to_string())?;
    let config = profile
        .presets
        .get(preset_name)
        .cloned()
        .ok_or_else(|| format!("音色预设「{preset_name}」不存在"))?;
    profile.config = config.clone();
    profile.active_preset = Some(preset_name.to_string());
    save_store(app_data_dir, &store)?;
    if status(None).connected {
        write_managed_config(&store)?;
    }
    Ok(config)
}

pub fn delete_preset(
    app_data_dir: &Path,
    device_id: &str,
    preset_name: &str,
) -> Result<EqPresetCatalog, String> {
    validate_device_id(device_id)?;
    validate_preset_name(preset_name)?;
    let mut store = load_store(app_data_dir)?;
    let profile = store
        .endpoints
        .get_mut(device_id)
        .ok_or_else(|| "该输出设备还没有保存音色预设".to_string())?;
    if profile.presets.len() <= 1 {
        return Err("至少需要保留一个音色预设".to_string());
    }
    if profile.presets.remove(preset_name).is_none() {
        return Err(format!("音色预设「{preset_name}」不存在"));
    }
    if profile.active_preset.as_deref() == Some(preset_name) {
        let next = profile
            .presets
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| "没有可切换的音色预设".to_string())?;
        profile.config = profile.presets[&next].clone();
        profile.active_preset = Some(next);
    }
    let catalog = EqPresetCatalog {
        active_preset: profile
            .active_preset
            .clone()
            .ok_or_else(|| "音色预设状态无效".to_string())?,
        presets: profile.presets.keys().cloned().collect(),
    };
    save_store(app_data_dir, &store)?;
    if status(None).connected {
        write_managed_config(&store)?;
    }
    Ok(catalog)
}

pub fn connect(app_data_dir: &Path) -> Result<EqualizerApoStatus, String> {
    let config_path = detect_config_path().ok_or_else(|| {
        "未检测到 Equalizer APO。请先从官方页面安装，并用 Configurator 将它启用到目标输出设备。"
            .to_string()
    })?;
    let main_path = config_path.join(MAIN_CONFIG_NAME);
    let original = fs::read_to_string(&main_path)
        .map_err(|error| format!("无法读取 Equalizer APO 主配置：{error}"))?;

    if managed_block_range(&original).is_none()
        && (original.contains(BEGIN_MARKER) || original.contains(END_MARKER))
    {
        return Err(
            "Equalizer APO 主配置中存在不完整的 Audio Hub 标记；为避免破坏配置，已停止接入。"
                .to_string(),
        );
    }

    let backup_path = config_path.join(BACKUP_FILE_NAME);
    if !backup_path.exists() {
        fs::copy(&main_path, &backup_path)
            .map_err(|error| format!("无法备份 Equalizer APO 主配置：{error}"))?;
    }

    write_managed_config(&load_store(app_data_dir)?)?;

    if managed_block_range(&original).is_none() {
        let mut updated = original;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(BEGIN_MARKER);
        updated.push('\n');
        updated.push_str("Include: audio-hub.txt\n");
        updated.push_str(END_MARKER);
        updated.push('\n');
        atomic_write(&main_path, updated.as_bytes())
            .map_err(|error| permission_hint("接入 Equalizer APO", error))?;
    }
    Ok(status(Some(app_data_dir)))
}

pub fn disconnect(app_data_dir: &Path) -> Result<EqualizerApoStatus, String> {
    let config_path = detect_config_path().ok_or_else(|| "未检测到 Equalizer APO。".to_string())?;
    let main_path = config_path.join(MAIN_CONFIG_NAME);
    let contents = fs::read_to_string(&main_path)
        .map_err(|error| format!("无法读取 Equalizer APO 主配置：{error}"))?;
    if let Some((start, end)) = managed_block_range(&contents) {
        let mut updated = contents;
        updated.replace_range(start..end, "");
        atomic_write(&main_path, updated.as_bytes())
            .map_err(|error| permission_hint("断开 Equalizer APO 接入", error))?;
    } else if contents.contains(BEGIN_MARKER) || contents.contains(END_MARKER) {
        return Err(
            "Equalizer APO 主配置中存在不完整的 Audio Hub 标记；为避免破坏配置，未做修改。"
                .to_string(),
        );
    }
    Ok(status(Some(app_data_dir)))
}

pub fn open_download_page() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg("https://sourceforge.net/projects/equalizerapo/files/")
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开 Equalizer APO 官方下载页：{error}"))
}

pub fn open_configurator() -> Result<(), String> {
    let current = status(None);
    let path = current
        .configurator_path
        .ok_or_else(|| "未找到 Equalizer APO Configurator.exe。".to_string())?;
    launch_elevated(Path::new(&path))
}

fn find_device_configurator(config_path: &Path) -> Option<PathBuf> {
    let install_dir = config_path.parent().unwrap_or(config_path);
    ["DeviceSelector.exe", "Configurator.exe"]
        .into_iter()
        .map(|name| install_dir.join(name))
        .find(|path| path.is_file())
}

fn launch_elevated(path: &Path) -> Result<(), String> {
    let verb = wide_null("runas");
    let executable = wide_null(path.as_os_str());
    let empty = wide_null("");
    let working_dir = wide_null(path.parent().unwrap_or(Path::new("")));
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(executable.as_ptr()),
            PCWSTR(empty.as_ptr()),
            PCWSTR(working_dir.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    let code = result.0 as usize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!(
            "无法以管理员身份启动 Equalizer APO 设备配置器（ShellExecute 错误 {code}）。\
             如果你取消了 UAC 确认，请重新点击并选择“是”。"
        ))
    }
}

fn write_managed_config(store: &ProfileStore) -> Result<(), String> {
    let config_path = detect_config_path().ok_or_else(|| "未检测到 Equalizer APO。".to_string())?;
    let configured_directory = store
        .rnnoise_plugin_directory
        .as_deref()
        .and_then(|value| valid_plugin_directory(value.to_string()));
    let plugins = detect_rnnoise_plugins(&config_path, configured_directory.as_deref());
    let rendered = render_managed_config(store, &plugins)?;
    atomic_write(&config_path.join(MANAGED_FILE_NAME), rendered.as_bytes())
        .map_err(|error| permission_hint("写入 Audio Hub EQ 配置", error))
}

fn render_managed_config(
    store: &ProfileStore,
    rnnoise_plugins: &RnnoisePlugins,
) -> Result<String, String> {
    let mut output = String::from(
        "# Audio Hub managed Equalizer APO configuration.\n\
         # Changes in this file may be overwritten by Audio Hub.\n\n",
    );
    let mut enabled_output_count = 0_u32;

    for (device_id, profile) in &store.endpoints {
        if !profile.config.enabled {
            continue;
        }
        let mut config = profile.config.clone();
        config.normalize_graphic_bands();
        let selector = endpoint_guid(device_id)?;
        enabled_output_count += 1;
        output.push_str(&format!("# Output: {}\n", profile.device_name));
        output.push_str(&format!("Device: {selector}\n"));
        output.push_str(&format!("Preamp: {:.1} dB\n", config.effective_preamp_db()));
        output.push_str("GraphicEQ:");
        for band in &config.bands {
            output.push_str(&format!(" {:.1} {:.1};", band.frequency_hz, band.gain_db));
        }
        output.push_str("\n\n");
    }

    if enabled_output_count == 0 {
        output.push_str("# No output endpoint EQ is enabled.\n");
    }

    for (device_id, profile) in &store.microphones {
        let config = &profile.config;
        if !config.enabled {
            continue;
        }
        config.validate()?;
        let selector = endpoint_guid(device_id)?;
        output.push_str(&format!("\n# Microphone: {}\n", profile.device_name));
        output.push_str(&format!("Device: {selector}\n"));
        output.push_str("Channel: all\n");
        output.push_str(&format!("Preamp: {:.1} dB\n", config.gain_db));
        if config.rnnoise_enabled {
            let plugin_path = rnnoise_plugins
                .path_for(config.rnnoise_mode)
                .ok_or_else(|| {
                    format!(
                        "麦克风「{}」启用了 RNNoise，但未找到 {}。",
                        profile.device_name,
                        config.rnnoise_mode.file_name()
                    )
                })?;
            output.push_str(&format!(
                "VSTPlugin: Library \"{}\"\n",
                path_text(plugin_path)
            ));
        }
    }

    output.push_str("Device: all\n");
    Ok(output)
}

fn profiles_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("equalizer-apo").join("endpoints.json")
}

fn graphic_eq_bands(gains: [f32; 10]) -> Vec<EqBandConfig> {
    GRAPHIC_EQ_FREQUENCIES
        .into_iter()
        .zip(gains)
        .enumerate()
        .map(|(index, (frequency_hz, gain_db))| EqBandConfig {
            kind: if index == 0 {
                EqFilterKind::LowShelf
            } else if index == GRAPHIC_EQ_FREQUENCIES.len() - 1 {
                EqFilterKind::HighShelf
            } else {
                EqFilterKind::Peaking
            },
            frequency_hz,
            gain_db,
            q: if index == 0 || index == GRAPHIC_EQ_FREQUENCIES.len() - 1 {
                0.707
            } else {
                1.414
            },
            enabled: true,
        })
        .collect()
}

fn interpolate_gain(source: &[(f32, f32)], frequency: f32) -> f32 {
    let Some(&(first_frequency, first_gain)) = source.first() else {
        return 0.0;
    };
    if frequency <= first_frequency {
        return first_gain;
    }
    let &(last_frequency, last_gain) = source.last().expect("source is not empty");
    if frequency >= last_frequency {
        return last_gain;
    }

    for pair in source.windows(2) {
        let (left_frequency, left_gain) = pair[0];
        let (right_frequency, right_gain) = pair[1];
        if frequency <= right_frequency {
            let position = (frequency.ln() - left_frequency.ln())
                / (right_frequency.ln() - left_frequency.ln());
            return left_gain + (right_gain - left_gain) * position;
        }
    }
    last_gain
}

fn load_store(app_data_dir: &Path) -> Result<ProfileStore, String> {
    let path = profiles_path(app_data_dir);
    if !path.exists() {
        return Ok(ProfileStore::default());
    }
    let contents =
        fs::read_to_string(&path).map_err(|error| format!("无法读取全局 EQ 配置：{error}"))?;
    let mut store: ProfileStore = serde_json::from_str(&contents)
        .map_err(|error| format!("全局 EQ 配置格式无效：{error}"))?;
    for profile in store.endpoints.values_mut() {
        migrate_endpoint_profile(profile);
    }
    Ok(store)
}

fn migrate_endpoint_profile(profile: &mut EndpointProfile) {
    profile.config.normalize_graphic_bands();
    if profile.presets.is_empty() {
        profile
            .presets
            .insert("当前音色".to_string(), profile.config.clone());
    }
    for config in profile.presets.values_mut() {
        config.normalize_graphic_bands();
    }
    let active_is_valid = profile
        .active_preset
        .as_ref()
        .is_some_and(|name| profile.presets.contains_key(name));
    if !active_is_valid {
        profile.active_preset = profile.presets.keys().next().cloned();
    }
    if let Some(config) = profile
        .active_preset
        .as_ref()
        .and_then(|name| profile.presets.get(name))
    {
        profile.config = config.clone();
    }
}

fn save_store(app_data_dir: &Path, store: &ProfileStore) -> Result<(), String> {
    let path = profiles_path(app_data_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建插件配置目录：{error}"))?;
    }
    let contents =
        serde_json::to_vec_pretty(store).map_err(|error| format!("无法序列化全局 EQ：{error}"))?;
    atomic_write(&path, &contents).map_err(|error| format!("无法保存全局 EQ：{error}"))
}

fn detect_config_path() -> Option<PathBuf> {
    registry_config_path()
        .into_iter()
        .chain(default_config_paths())
        .map(normalize_config_path)
        .find(|path| path.join(MAIN_CONFIG_NAME).is_file())
}

fn detect_rnnoise_plugins(
    config_path: &Path,
    configured_directory: Option<&Path>,
) -> RnnoisePlugins {
    let install_dir = config_path.parent().unwrap_or(config_path);
    let mut plugin_dirs = Vec::new();
    if let Some(directory) = configured_directory {
        plugin_dirs.push(directory.to_path_buf());
    }
    plugin_dirs.push(install_dir.join("VSTPlugins").join("AudioHub"));
    plugin_dirs.push(install_dir.join("VSTPlugins"));

    let find = |file_name: &str| {
        plugin_dirs
            .iter()
            .map(|directory| directory.join(file_name))
            .find(|path| path.is_file())
    };

    RnnoisePlugins {
        mono: find(RNNOISE_MONO_FILE_NAME),
        stereo: find(RNNOISE_STEREO_FILE_NAME),
    }
}

fn valid_plugin_directory(value: String) -> Option<PathBuf> {
    if value.trim().is_empty()
        || value.contains(['\r', '\n', '"'])
        || !Path::new(&value).is_absolute()
    {
        return None;
    }
    Some(PathBuf::from(value))
}

fn normalize_config_path(path: PathBuf) -> PathBuf {
    if path.join(MAIN_CONFIG_NAME).is_file() {
        path
    } else {
        path.join("config")
    }
}

fn default_config_paths() -> impl Iterator<Item = PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .map(|base| base.join("EqualizerAPO").join("config"))
}

fn registry_config_path() -> Option<PathBuf> {
    let subkey = wide_null(REGISTRY_KEY);
    let value = wide_null(REGISTRY_VALUE);
    let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY;
    let mut byte_count = 0_u32;
    let first = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            flags,
            None,
            None,
            Some(&mut byte_count),
        )
    };
    if first != ERROR_SUCCESS || byte_count < 2 {
        return None;
    }

    let mut buffer = vec![0_u16; (byte_count as usize).div_ceil(2)];
    let second = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            flags,
            None,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            Some(&mut byte_count),
        )
    };
    if second != ERROR_SUCCESS {
        return None;
    }
    let length = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

fn equalizer_apo_enabled_for_device(device_id: &str) -> bool {
    let Ok(guid) = endpoint_guid(device_id) else {
        return false;
    };
    let subkey = wide_null(format!(r"{CHILD_APOS_REGISTRY_KEY}\{guid}"));
    let value = wide_null("Version");
    let mut byte_count = 0_u32;
    (unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            None,
            Some(&mut byte_count),
        )
    }) == ERROR_SUCCESS
}

fn require_equalizer_apo_enabled(device_id: &str) -> Result<(), String> {
    if equalizer_apo_enabled_for_device(device_id) {
        Ok(())
    } else {
        Err("该设备尚未在 Equalizer APO 设备配置器中启用。请先打开设备配置器并勾选它。".to_string())
    }
}

fn pick_rnnoise_plugin_directory() -> Result<Option<PathBuf>, String> {
    thread::spawn(pick_rnnoise_plugin_directory_on_sta)
        .join()
        .map_err(|_| "选择 RNNoise 插件文件夹时发生异常。".to_string())?
}

fn pick_rnnoise_plugin_directory_on_sta() -> Result<Option<PathBuf>, String> {
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    initialized
        .ok()
        .map_err(|error| format!("无法初始化插件文件夹选择器：{error}"))?;
    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let _guard = ComGuard;

    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| format!("无法创建插件文件夹选择器：{error}"))?;
    let title = wide_null("选择包含 RNNoise VST2 DLL 的文件夹");
    unsafe {
        dialog
            .SetTitle(PCWSTR(title.as_ptr()))
            .and_then(|_| dialog.GetOptions())
            .and_then(|options| {
                dialog
                    .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
            })
            .map_err(|error| format!("无法配置插件文件夹选择器：{error}"))?;
    }

    if let Err(error) = unsafe { dialog.Show(None) } {
        if error.code().0 as u32 == 0x8007_04c7 {
            return Ok(None);
        }
        return Err(format!("无法打开插件文件夹选择器：{error}"));
    }

    let item = unsafe { dialog.GetResult() }
        .map_err(|error| format!("无法读取所选插件文件夹：{error}"))?;
    let raw_path = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }
        .map_err(|error| format!("无法读取所选插件文件夹路径：{error}"))?;
    let path = unsafe { raw_path.to_string() }
        .map(PathBuf::from)
        .map_err(|error| format!("插件文件夹路径无效：{error}"));
    unsafe { CoTaskMemFree(Some(raw_path.0.cast::<c_void>())) };
    path.map(Some)
}

fn endpoint_guid(device_id: &str) -> Result<&str, String> {
    let start = device_id
        .rfind('{')
        .ok_or_else(|| "输出设备 ID 中没有可供 Equalizer APO 匹配的 GUID。".to_string())?;
    let guid = &device_id[start..];
    if !guid.ends_with('}')
        || guid.len() != 38
        || !guid[1..guid.len() - 1]
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() || ch == '-')
    {
        return Err("输出设备 ID 中的 GUID 格式无效。".to_string());
    }
    Ok(guid)
}

fn validate_device_id(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains(['\r', '\n']) {
        return Err("输出设备 ID 无效。".to_string());
    }
    Ok(())
}

fn validate_device_name(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains(['\r', '\n']) {
        return Err("输出设备名称无效。".to_string());
    }
    Ok(())
}

fn validate_preset_name(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.trim() != value {
        return Err("音色预设名称不能为空，且首尾不能包含空格".to_string());
    }
    if value.chars().count() > 40 {
        return Err("音色预设名称不能超过 40 个字符".to_string());
    }
    if value.chars().any(char::is_control) {
        return Err("音色预设名称包含无效字符".to_string());
    }
    Ok(())
}

fn managed_block_range(contents: &str) -> Option<(usize, usize)> {
    let start = contents.find(BEGIN_MARKER)?;
    let marker_end = contents[start..].find(END_MARKER)? + start + END_MARKER.len();
    let end = if contents.as_bytes().get(marker_end) == Some(&b'\r')
        && contents.as_bytes().get(marker_end + 1) == Some(&b'\n')
    {
        marker_end + 2
    } else if contents.as_bytes().get(marker_end) == Some(&b'\n') {
        marker_end + 1
    } else {
        marker_end
    };
    Some((start, end))
}

fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let temp_path = path.with_extension("audio-hub.tmp");
    let mut file = fs::File::create(&temp_path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);

    let from = wide_null(temp_path.as_os_str());
    let to = wide_null(path.as_os_str());
    let mut replace_error = None;
    for retry_delay_ms in [0, 10, 30] {
        if retry_delay_ms > 0 {
            thread::sleep(Duration::from_millis(retry_delay_ms));
        }
        match unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } {
            Ok(()) => return Ok(()),
            Err(error) => {
                let error = win32_io_error(error);
                if error.kind() != std::io::ErrorKind::PermissionDenied {
                    let _ = fs::remove_file(&temp_path);
                    return Err(error);
                }
                replace_error = Some(error);
            }
        }
    }

    // Equalizer APO 读取配置时可能没有授予 FILE_SHARE_DELETE，导致目标文件
    // 无法被原子替换，但仍允许写入。此时原地更新可避免用户切换预设失败。
    let direct_result = (|| {
        let mut target = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        target.write_all(contents)?;
        target.sync_all()
    })();
    let _ = fs::remove_file(&temp_path);

    match direct_result {
        Ok(()) => Ok(()),
        Err(direct_error) => {
            let replace_error =
                replace_error.unwrap_or_else(|| std::io::Error::other("无法替换目标配置文件"));
            Err(std::io::Error::new(
                direct_error.kind(),
                format!("原子替换失败（{replace_error}），且原地写入失败（{direct_error}）"),
            ))
        }
    }
}

fn win32_io_error(error: windows::core::Error) -> std::io::Error {
    let hresult = error.code().0 as u32;
    std::io::Error::from_raw_os_error((hresult & 0xffff) as i32)
}

fn wide_null(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn permission_hint(action: &str, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        format!("{action}失败：没有写入权限。请以管理员身份运行 Audio Hub 后重试一次。")
    } else {
        format!("{action}失败：{error}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rnnoise_plugins() -> RnnoisePlugins {
        RnnoisePlugins {
            mono: Some(PathBuf::from(
                r"D:\EqualizerAPO\VSTPlugins\AudioHub\rnnoise_mono.dll",
            )),
            stereo: Some(PathBuf::from(
                r"D:\EqualizerAPO\VSTPlugins\AudioHub\rnnoise_stereo.dll",
            )),
        }
    }

    fn sample_store() -> ProfileStore {
        let mut store = ProfileStore::default();
        let mut config = GlobalEqConfig {
            enabled: true,
            ..GlobalEqConfig::default()
        };
        config.bands[0].gain_db = 6.0;
        store.endpoints.insert(
            "{0.0.0.00000000}.{01234567-89AB-CDEF-0123-456789ABCDEF}".to_string(),
            EndpointProfile {
                device_name: "Test Headphones".to_string(),
                config,
                presets: BTreeMap::new(),
                active_preset: None,
            },
        );
        store
    }

    #[test]
    fn auto_headroom_offsets_largest_boost() {
        let store = sample_store();
        let profile = &store.endpoints.values().next().unwrap().config;
        assert_eq!(profile.effective_preamp_db(), -6.0);
    }

    #[test]
    fn renders_device_scoped_equalizer_apo_config() {
        let rendered = render_managed_config(&sample_store(), &sample_rnnoise_plugins()).unwrap();
        assert!(rendered.contains("Device: {01234567-89AB-CDEF-0123-456789ABCDEF}"));
        assert!(rendered.contains("Preamp: -6.0 dB"));
        assert!(rendered.contains("GraphicEQ: 31.5 6.0;"));
        assert!(rendered.contains("16000.0 0.0;"));
        assert!(rendered.ends_with("Device: all\n"));
    }

    #[test]
    fn renders_microphone_gain_and_rnnoise_plugin() {
        let mut store = sample_store();
        store.microphones.insert(
            "{0.0.1.00000000}.{89ABCDEF-0123-4567-89AB-CDEF01234567}".to_string(),
            MicrophoneProfile {
                device_name: "Test Microphone".to_string(),
                config: MicrophoneConfig::default(),
            },
        );

        let rendered = render_managed_config(&store, &sample_rnnoise_plugins()).unwrap();
        assert!(rendered.contains("# Microphone: Test Microphone"));
        assert!(rendered.contains("Device: {89ABCDEF-0123-4567-89AB-CDEF01234567}"));
        assert!(rendered.contains("Channel: all"));
        assert!(rendered.contains("Preamp: 8.0 dB"));
        assert!(rendered.contains(
            r#"VSTPlugin: Library "D:\EqualizerAPO\VSTPlugins\AudioHub\rnnoise_mono.dll""#
        ));
        assert!(!rendered.contains("Filter:"));
    }

    #[test]
    fn validates_microphone_processing_ranges() {
        assert!(MicrophoneConfig::default().validate().is_ok());

        let too_loud = MicrophoneConfig {
            gain_db: 24.0,
            ..MicrophoneConfig::default()
        };
        assert!(too_loud.validate().is_err());
    }

    #[test]
    fn rejects_enabled_rnnoise_when_plugin_is_missing() {
        let mut store = sample_store();
        store.microphones.insert(
            "{0.0.1.00000000}.{89ABCDEF-0123-4567-89AB-CDEF01234567}".to_string(),
            MicrophoneProfile {
                device_name: "Test Microphone".to_string(),
                config: MicrophoneConfig::default(),
            },
        );

        let error = render_managed_config(&store, &RnnoisePlugins::default()).unwrap_err();
        assert!(error.contains(RNNOISE_MONO_FILE_NAME));
    }

    #[test]
    fn migrates_legacy_microphone_filters_to_rnnoise_defaults() {
        let legacy = r#"{
            "enabled": true,
            "gain_db": 6.0,
            "high_pass_enabled": true,
            "high_pass_hz": 80.0,
            "hiss_reduction_db": 2.0
        }"#;
        let mut config: MicrophoneConfig = serde_json::from_str(legacy).unwrap();

        assert_eq!(config.gain_db, 6.0);
        assert!(config.rnnoise_enabled);
        assert_eq!(config.rnnoise_mode, RnnoiseChannelMode::Mono);
        assert!(config.discard_legacy_noise_filters());
        assert!(!config.discard_legacy_noise_filters());
        let migrated = serde_json::to_string(&config).unwrap();
        assert!(!migrated.contains("high_pass"));
        assert!(!migrated.contains("hiss_reduction"));
    }

    #[test]
    fn falls_back_when_reader_blocks_atomic_replace() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_dir = std::env::temp_dir().join(format!(
            "audio-hub-atomic-write-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&test_dir).unwrap();
        let target = test_dir.join("audio-hub.txt");

        atomic_write(&target, b"before").unwrap();
        let reader = fs::OpenOptions::new()
            .read(true)
            // 模拟允许读写、但未授予 FILE_SHARE_DELETE 的 Equalizer APO 读取句柄。
            .share_mode(0x0000_0001 | 0x0000_0002)
            .open(&target)
            .unwrap();

        atomic_write(&target, b"after").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "after");
        assert!(!target.with_extension("audio-hub.tmp").exists());

        drop(reader);
        fs::remove_dir_all(test_dir).unwrap();
    }

    #[test]
    fn migrates_five_bands_to_ten_graphic_points() {
        let mut config = GlobalEqConfig {
            bands: vec![
                EqBandConfig {
                    kind: EqFilterKind::LowShelf,
                    frequency_hz: 80.0,
                    gain_db: 0.0,
                    q: 0.707,
                    enabled: true,
                },
                EqBandConfig {
                    kind: EqFilterKind::Peaking,
                    frequency_hz: 250.0,
                    gain_db: 0.0,
                    q: 1.0,
                    enabled: true,
                },
                EqBandConfig {
                    kind: EqFilterKind::Peaking,
                    frequency_hz: 1_000.0,
                    gain_db: 0.0,
                    q: 1.0,
                    enabled: true,
                },
                EqBandConfig {
                    kind: EqFilterKind::Peaking,
                    frequency_hz: 4_000.0,
                    gain_db: 0.0,
                    q: 1.0,
                    enabled: true,
                },
                EqBandConfig {
                    kind: EqFilterKind::HighShelf,
                    frequency_hz: 12_000.0,
                    gain_db: 0.0,
                    q: 0.707,
                    enabled: true,
                },
            ],
            ..GlobalEqConfig::default()
        };
        config.bands[0].gain_db = 6.0;
        config.bands[1].gain_db = -4.0;
        config.normalize_graphic_bands();
        assert_eq!(config.bands.len(), 10);
        assert_eq!(config.bands[0].gain_db, 6.0);
        assert_eq!(config.bands[3].gain_db, -4.0);
    }

    #[test]
    fn migrates_legacy_endpoint_config_to_named_preset() {
        let mut profile = EndpointProfile {
            device_name: "Legacy device".to_string(),
            config: GlobalEqConfig::default(),
            presets: BTreeMap::new(),
            active_preset: None,
        };
        migrate_endpoint_profile(&mut profile);
        assert_eq!(profile.active_preset.as_deref(), Some("当前音色"));
        assert!(profile.presets.contains_key("当前音色"));
    }

    #[test]
    fn validates_preset_names() {
        assert!(validate_preset_name("FPS 脚步增强").is_ok());
        assert!(validate_preset_name(" 电影").is_err());
        assert!(validate_preset_name("坏\n名称").is_err());
    }

    #[test]
    fn finds_complete_managed_block_only() {
        let text = format!("before\n{BEGIN_MARKER}\nInclude: audio-hub.txt\n{END_MARKER}\nafter");
        let (start, end) = managed_block_range(&text).unwrap();
        assert_eq!(&text[..start], "before\n");
        assert_eq!(&text[end..], "after");
        assert!(managed_block_range(BEGIN_MARKER).is_none());
    }
}
