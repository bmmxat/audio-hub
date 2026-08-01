/* global window */
// Audio Hub — Tauri invoke API 封装
// 使用 window.__TAURI__ 全局 API（withGlobalTauri: true 模式）

window.AudioAPI = {
    /** 获取默认输出设备 ID */
    async getDefaultDeviceId() {
        return await window.__TAURI__.core.invoke('get_default_device_id');
    },

    /** 获取默认输出设备名称 */
    async getDefaultDeviceName() {
        return await window.__TAURI__.core.invoke('get_default_device_name');
    },

    /** 枚举指定方向的音频设备 */
    async enumerateDevices(direction) {
        return await window.__TAURI__.core.invoke('enumerate_devices', {
            direction: direction,
        });
    },

    /** 枚举所有输出设备上的音频会话 */
    async enumerateSessions() {
        return await window.__TAURI__.core.invoke('enumerate_sessions');
    },

    /** 设置指定 PID 应用的音量（0.0 ~ 1.0） */
    async setSessionVolume(pid, volume) {
        return await window.__TAURI__.core.invoke('set_session_volume', {
            pid: pid,
            volume: volume,
        });
    },

    /** 设置指定 PID 应用的静音状态 */
    async setSessionMute(pid, muted) {
        return await window.__TAURI__.core.invoke('set_session_mute', {
            pid: pid,
            muted: muted,
        });
    },

    /** 获取输出设备或麦克风的 Windows 主音量 */
    async getDeviceVolume(deviceId) {
        return await window.__TAURI__.core.invoke('get_device_volume', {
            deviceId: deviceId,
        });
    },

    /** 设置输出设备或麦克风的 Windows 主音量 */
    async setDeviceVolume(deviceId, volume) {
        return await window.__TAURI__.core.invoke('set_device_volume', {
            deviceId: deviceId,
            volume: volume,
        });
    },

    /** 设置输出设备或麦克风的静音状态 */
    async setDeviceMute(deviceId, muted) {
        return await window.__TAURI__.core.invoke('set_device_mute', {
            deviceId: deviceId,
            muted: muted,
        });
    },

    /** 获取登录 Windows 后自动启动状态 */
    async getAutostartEnabled() {
        return await window.__TAURI__.core.invoke('get_autostart_enabled');
    },

    /** 设置登录 Windows 后自动启动 */
    async setAutostartEnabled(enabled) {
        return await window.__TAURI__.core.invoke('set_autostart_enabled', {
            enabled: enabled,
        });
    },

    /** 将指定端点设为默认设备 */
    async setDefaultDevice(deviceId) {
        return await window.__TAURI__.core.invoke('set_default_device', {
            deviceId: deviceId,
        });
    },

    /** Per-app 路由：设置应用输出设备 */
    async setAppOutputDevice(pid, deviceId) {
        return await window.__TAURI__.core.invoke('set_app_output_device', {
            pid: pid,
            deviceId: deviceId,
        });
    },

    /** 打开 Windows 声音设置面板（降级方案） */
    async openSoundSettings() {
        return await window.__TAURI__.core.invoke('open_sound_settings');
    },

    /** Windows 原生音频通知监听器是否可用 */
    async audioNotificationsAvailable() {
        return await window.__TAURI__.core.invoke(
            'audio_notifications_available',
        );
    },

    async processLoopbackSupport() {
        return await window.__TAURI__.core.invoke('process_loopback_support');
    },

    async processCaptureStatus() {
        return await window.__TAURI__.core.invoke('process_capture_status');
    },

    async startProcessCapture(pid) {
        return await window.__TAURI__.core.invoke('start_process_capture', {
            pid: pid,
        });
    },

    async stopProcessCapture() {
        return await window.__TAURI__.core.invoke('stop_process_capture');
    },

    async revealCaptureFile(path) {
        return await window.__TAURI__.core.invoke('reveal_capture_file', {
            path: path,
        });
    },

    async equalizerApoStatus() {
        return await window.__TAURI__.core.invoke('equalizer_apo_status');
    },

    async equalizerApoEnabledDevices(deviceIds) {
        return await window.__TAURI__.core.invoke('equalizer_apo_enabled_devices', {
            deviceIds: deviceIds,
        });
    },

    async equalizerApoProcessingState(outputDeviceId, inputDeviceId) {
        return await window.__TAURI__.core.invoke('equalizer_apo_processing_state', {
            outputDeviceId: outputDeviceId,
            inputDeviceId: inputDeviceId,
        });
    },

    async chooseRnnoisePluginDirectory() {
        return await window.__TAURI__.core.invoke('choose_rnnoise_plugin_directory');
    },

    async getGlobalEq(deviceId) {
        return await window.__TAURI__.core.invoke('get_global_eq', {
            deviceId: deviceId,
        });
    },

    async setGlobalEq(deviceId, deviceName, config) {
        return await window.__TAURI__.core.invoke('set_global_eq', {
            deviceId: deviceId,
            deviceName: deviceName,
            config: config,
        });
    },

    async getMicrophoneProcessing(deviceId) {
        return await window.__TAURI__.core.invoke('get_microphone_processing', {
            deviceId: deviceId,
        });
    },

    async setMicrophoneProcessing(deviceId, deviceName, config) {
        return await window.__TAURI__.core.invoke('set_microphone_processing', {
            deviceId: deviceId,
            deviceName: deviceName,
            config: config,
        });
    },

    async listGlobalEqPresets(deviceId) {
        return await window.__TAURI__.core.invoke('list_global_eq_presets', {
            deviceId: deviceId,
        });
    },

    async getGlobalEqPreset(deviceId, presetName) {
        return await window.__TAURI__.core.invoke('get_global_eq_preset', {
            deviceId: deviceId,
            presetName: presetName,
        });
    },

    async saveGlobalEqPreset(deviceId, deviceName, presetName, config) {
        return await window.__TAURI__.core.invoke('save_global_eq_preset', {
            deviceId: deviceId,
            deviceName: deviceName,
            presetName: presetName,
            config: config,
        });
    },

    async activateGlobalEqPreset(deviceId, presetName) {
        return await window.__TAURI__.core.invoke('activate_global_eq_preset', {
            deviceId: deviceId,
            presetName: presetName,
        });
    },

    async deleteGlobalEqPreset(deviceId, presetName) {
        return await window.__TAURI__.core.invoke('delete_global_eq_preset', {
            deviceId: deviceId,
            presetName: presetName,
        });
    },

    async connectEqualizerApo() {
        return await window.__TAURI__.core.invoke('connect_equalizer_apo');
    },

    async disconnectEqualizerApo() {
        return await window.__TAURI__.core.invoke('disconnect_equalizer_apo');
    },

    async openEqualizerApoDownload() {
        return await window.__TAURI__.core.invoke('open_equalizer_apo_download');
    },

    async openEqualizerApoConfigurator() {
        return await window.__TAURI__.core.invoke('open_equalizer_apo_configurator');
    },

    async voicemeeterStatus() {
        return await window.__TAURI__.core.invoke('voicemeeter_status');
    },

    async startVoicemeeter() {
        return await window.__TAURI__.core.invoke('start_voicemeeter');
    },

    async showVoicemeeter() {
        return await window.__TAURI__.core.invoke('show_voicemeeter');
    },

    async restartVoicemeeterAudioEngine() {
        return await window.__TAURI__.core.invoke(
            'restart_voicemeeter_audio_engine',
        );
    },

    async shutdownVoicemeeter() {
        return await window.__TAURI__.core.invoke('shutdown_voicemeeter');
    },

    async applyVoicemeeterConfiguration(configuration) {
        return await window.__TAURI__.core.invoke(
            'apply_voicemeeter_configuration',
            { configuration: configuration },
        );
    },

    async openVoicemeeterDownload() {
        return await window.__TAURI__.core.invoke('open_voicemeeter_download');
    },

    async simpleRouteStatus() {
        return await window.__TAURI__.core.invoke('simple_route_status');
    },

    async enableSimpleRouteApplication(pid, key, displayName) {
        return await window.__TAURI__.core.invoke(
            'enable_simple_route_application',
            { pid: pid, key: key, displayName: displayName },
        );
    },

    async disableSimpleRouteApplication(key, currentPid) {
        return await window.__TAURI__.core.invoke(
            'disable_simple_route_application',
            { key: key, currentPid: currentPid },
        );
    },

    async stopAllSimpleRoutes() {
        return await window.__TAURI__.core.invoke('stop_all_simple_routes');
    },

    async syncSimpleRouteMonitor() {
        return await window.__TAURI__.core.invoke('sync_simple_route_monitor');
    },

    // ── Profile ────────────────────────────────────────
    async saveProfile(name, sessions) {
        return await window.__TAURI__.core.invoke('save_profile', {
            name: name,
            sessions: sessions,
        });
    },
    async listProfiles() {
        return await window.__TAURI__.core.invoke('list_profiles');
    },
    async applyProfile(name) {
        return await window.__TAURI__.core.invoke('apply_profile', {
            name: name,
        });
    },
    async deleteProfile(name) {
        return await window.__TAURI__.core.invoke('delete_profile', {
            name: name,
        });
    },
};
