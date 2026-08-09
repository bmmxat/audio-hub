/* global window, AudioAPI */
// 音量调节小面板：默认输出设备音量滑块 + 静音开关。

const $ = (sel) => document.querySelector(sel);

let deviceId = null;
let muted = false;
let busy = false;
let unlisten = null;

function render(state) {
    const slider = $('#vp-slider');
    const pct = $('#vp-pct');
    const muteBtn = $('#vp-mute');
    const status = $('#vp-status');
    if (slider) {
        slider.value = String(Math.round(state.volume * 100));
    }
    if (pct) {
        pct.textContent = `${Math.round(state.volume * 100)}%`;
    }
    if (muteBtn) {
        muteBtn.textContent = state.muted ? '取消静音' : '静音';
    }
    if (status) {
        status.textContent = '';
    }
    muted = state.muted;
}

async function load() {
    try {
        const nextDeviceId = await AudioAPI.getDefaultDeviceId();
        deviceId = nextDeviceId;
        const state = await AudioAPI.getDeviceVolume(deviceId);
        render(state);
        const deviceName = await defaultDeviceName();
        const deviceLabel = $('#vp-device');
        if (deviceLabel) {
            deviceLabel.textContent = deviceName || '默认输出设备';
            deviceLabel.title = deviceLabel.textContent;
        }
    } catch (err) {
        const status = $('#vp-status');
        if (status) {
            status.textContent = `读取失败：${err}`;
        }
    }
}

async function defaultDeviceName() {
    try {
        const devices = await AudioAPI.enumerateDevices('Output');
        return devices.find((device) => device.is_default)?.name || '';
    } catch {
        return '';
    }
}

async function setVolume(percent) {
    if (!deviceId || busy) return;
    busy = true;
    try {
        const state = await AudioAPI.setDeviceVolume(deviceId, percent / 100);
        render(state);
    } catch (err) {
        const status = $('#vp-status');
        if (status) {
            status.textContent = `设置失败：${err}`;
        }
        load();
    } finally {
        busy = false;
    }
}

async function toggleMute() {
    if (!deviceId || busy) return;
    busy = true;
    try {
        const state = await AudioAPI.setDeviceMute(deviceId, !muted);
        render(state);
    } catch (err) {
        const status = $('#vp-status');
        if (status) {
            status.textContent = `设置失败：${err}`;
        }
        load();
    } finally {
        busy = false;
    }
}

window.addEventListener('DOMContentLoaded', async () => {
    // 与主窗口保持同一主题
    const savedTheme = localStorage.getItem('audio-hub-theme');
    const systemDark = window.matchMedia?.('(prefers-color-scheme: dark)').matches;
    document.documentElement.dataset.theme =
        savedTheme || (systemDark ? 'dark' : 'light');

    $('#vp-slider')?.addEventListener('input', (e) => {
        const percent = Number(e.target.value);
        $('#vp-pct').textContent = `${percent}%`;
        setVolume(percent);
    });
    $('#vp-mute')?.addEventListener('click', toggleMute);

    await load();

    try {
        unlisten = await window.__TAURI__.event.listen('audio-devices-changed', () => {
            load();
        });
    } catch {
        // 事件监听不可用时保持静态读取。
    }
});

window.addEventListener('beforeunload', () => {
    if (typeof unlisten === 'function') {
        unlisten();
    }
});
