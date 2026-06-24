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

    /** 将指定端点设为默认设备 */
    async setDefaultDevice(deviceId) {
        return await window.__TAURI__.core.invoke('set_default_device', {
            deviceId: deviceId,
        });
    },

    /** 打开 Windows 声音设置面板（降级方案） */
    async openSoundSettings() {
        return await window.__TAURI__.core.invoke('open_sound_settings');
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
