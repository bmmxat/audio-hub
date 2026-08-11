/* global window, AudioAPI */
// Audio Hub — UI 状态管理和渲染

// ── 状态 ──────────────────────────────────────────────
const state = {
    defaultOutput: null,
    defaultInput: null,
    outputDevices: [],
    inputDevices: [],
    sessions: [],
    loading: false,
    error: null,
    drawerOpen: false,
    hiddenSessions: new Set(),
    showHidden: false,
    autoRefreshId: null,
    refreshing: false,
    notificationRefreshTimer: null,
    notificationUnlisteners: [],
    pendingSessionRefresh: false,
    pendingDeviceRefresh: false,
    deviceVolumeFollowEnabled: false,
    deviceVolumeSnapshots: {},
    deviceVolumeFollowBusy: false,
    deviceVolumeSnapshotTimer: null,
    captureStatus: {
        supported: false,
        windows_build: 0,
        active: false,
        pid: null,
        output_path: null,
        elapsed_ms: 0,
        last_error: null,
    },
    captureBusy: false,
    captureStatusTimer: null,
    globalEqStatus: null,
    globalEqConfig: null,
    globalEqSavedConfig: null,
    globalEqDeviceId: null,
    globalEqPresets: [],
    globalEqPresetName: null,
    globalEqBusy: false,
    globalEqMessage: null,
    globalEqDirty: false,
    equalizerApoTab: 'output',
    equalizerApoEnabledDeviceIds: new Set(),
    microphoneConfig: null,
    microphoneSavedConfig: null,
    microphoneDeviceId: null,
    microphoneBusy: false,
    microphoneMessage: null,
    microphoneDirty: false,
    microphoneConfigured: false,
    sessionDevices: {},         // 稳定会话标识 → deviceId 映射
    drawerVolumes: { output: null, input: null },
    drawerVolumeRequestIds: { output: 0, input: 0 },
    autostartBusy: false,
    autostartEnabled: false,
    autostartMessage: null,
    closeBehavior: 'minimize',
    closeBehaviorChosen: false,
    closeBehaviorBusy: false,
    closeBehaviorMessage: null,
    closeBehaviorFirstChoice: false,
    unfocusedMuteStatus: {
        applications: [],
        auto_muted_keys: [],
        foreground_key: null,
        paused: false,
    },
    unfocusedMuteBusy: false,
    unfocusedMuteMessage: null,
    voicemeeterStatus: null,
    voicemeeterConfiguration: null,
    voicemeeterBusy: false,
    voicemeeterMessage: null,
    voicemeeterSourceManagerTarget: null,
    voicemeeterDspOpen: false,
    voicemeeterMode: localStorage.getItem('audio-hub-voicemeeter-mode') || 'simple',
    voicemeeterModeMenuOpen: false,
    simpleRouteBusy: false,
    simpleRouteMonitorSyncBusy: false,
    simpleRouteStatus: {
        active: false,
        applications: [],
        physical_microphone_name: null,
        monitor_device_name: null,
        virtual_microphone_id: null,
        voicemeeter_input_id: null,
        recovery_pending: false,
    },
    voicemeeterApoProcessing: {
        output_eq_active: false,
        microphone_processing_active: false,
    },
};

// ── DOM 引用 ─────────────────────────────────────────
const $ = (sel) => document.querySelector(sel);

const dom = {
    themeToggleBtn: $('#theme-toggle-btn'),
    themeIcon: $('#theme-icon'),
    deviceDrawer: $('#device-drawer'),
    drawerOverlay: $('#drawer-overlay'),
    drawerCloseBtn: $('#drawer-close-btn'),
    deviceList: $('#device-list'),
    errorBanner: $('#error-banner'),
    errorMsg: $('#error-msg'),
    sessionList: $('#session-list'),
    sessionCount: $('#session-count'),
    hiddenBadge: $('#hidden-badge'),
    deviceVolumeFollowBtn: $('#device-volume-follow-btn'),
    deviceVolumeFollowLabel: $('#device-volume-follow-label'),
    statusText: $('#status-text'),
    statusbarOutput: $('#statusbar-output'),
    statusbarInput: $('#statusbar-input'),
    statusbarOutputName: $('#statusbar-output-name'),
    statusbarInputName: $('#statusbar-input-name'),
    globalEqBtn: $('#global-eq-btn'),
    globalEqModal: $('#global-eq-modal'),
    globalEqCloseBtn: $('#global-eq-close-btn'),
    globalEqContent: $('#global-eq-content'),
    unfocusedMuteBtn: $('#unfocused-mute-btn'),
    unfocusedMuteBtnLabel: $('#unfocused-mute-btn-label'),
    unfocusedMuteModal: $('#unfocused-mute-modal'),
    unfocusedMuteCloseBtn: $('#unfocused-mute-close-btn'),
    unfocusedMuteContent: $('#unfocused-mute-content'),
    voicemeeterBtn: $('#voicemeeter-btn'),
    voicemeeterBtnLabel: $('#voicemeeter-btn-label'),
    voicemeeterModeMenu: $('#voicemeeter-mode-menu'),
    voicemeeterModal: $('#voicemeeter-modal'),
    voicemeeterCloseBtn: $('#voicemeeter-close-btn'),
    voicemeeterContent: $('#voicemeeter-content'),
};

// ── 生命周期 ─────────────────────────────────────────
document.addEventListener('DOMContentLoaded', async () => {
    initTheme();
    initHiddenState();
    initSessionDevices();
    initDeviceVolumeFollowState();
    await loadAllData();
    await refreshGlobalEqEntry();
    await refreshUnfocusedMuteStatus();
    await refreshCaptureStatus();
    setupEventListeners();
    await maybeStartVoicemeeter();
    await refreshSimpleRouteIntegration();
    const notificationsAvailable = await setupAudioNotifications();
    startAutoRefresh(notificationsAvailable ? 30000 : 3000);
    startCaptureStatusPolling();
});

window.addEventListener('beforeunload', () => {
    for (const unlisten of state.notificationUnlisteners) {
        unlisten();
    }
    if (state.captureStatusTimer) clearInterval(state.captureStatusTimer);
    if (state.deviceVolumeSnapshotTimer) {
        clearTimeout(state.deviceVolumeSnapshotTimer);
        saveCurrentDeviceVolumeSnapshot();
    }
});

// ── 数据加载 ─────────────────────────────────────────
async function loadAllData() {
    state.loading = true;
    state.error = null;
    setStatus('加载中…');

    try {
        const previousDefaultOutput = state.defaultOutput;
        const previousSessions = state.sessions;
        const [outputDevices, inputDevices, sessions] =
            await Promise.all([
                AudioAPI.enumerateDevices('Output'),
                AudioAPI.enumerateDevices('Input'),
                AudioAPI.enumerateSessions(),
            ]);

        const nextDefaultOutput = outputDevices.find((d) => d.is_default) || null;
        const defaultOutputChanged =
            (previousDefaultOutput?.device_id || null) !==
            (nextDefaultOutput?.device_id || null);
        if (
            state.deviceVolumeFollowEnabled &&
            defaultOutputChanged &&
            previousDefaultOutput
        ) {
            saveDeviceVolumeSnapshot(
                previousDefaultOutput.device_id,
                previousDefaultOutput.name,
                previousSessions,
            );
        }

        state.outputDevices = outputDevices;
        state.inputDevices = inputDevices;
        state.sessions = sessions;
        migrateLegacyLocalState();
        state.defaultOutput = nextDefaultOutput;
        state.defaultInput = inputDevices.find((d) => d.is_default) || null;
        state.error = null;

        let followResult = null;
        if (
            state.deviceVolumeFollowEnabled &&
            nextDefaultOutput &&
            (defaultOutputChanged || !previousDefaultOutput)
        ) {
            followResult = await activateDeviceVolumeSnapshot(nextDefaultOutput);
        } else if (state.deviceVolumeFollowEnabled) {
            saveCurrentDeviceVolumeSnapshot();
        }

        setStatus(followResult?.applied > 0
            ? `已恢复 ${followResult.applied} 个应用在 ${nextDefaultOutput.name} 的音量`
            : '就绪');
    } catch (err) {
        state.error = typeof err === 'string' ? err : JSON.stringify(err);
        setStatus('加载失败');
    }

    state.loading = false;
    renderAll();
}

// ── 全量渲染 ─────────────────────────────────────────
function renderAll() {
    renderSessionList();
    renderUnfocusedMuteEntry();
    renderGlobalEqEntry();
    renderDeviceVolumeFollowEntry();
    renderDeviceList();
    renderStatusbar();
    renderError();
}

async function refreshGlobalEqEntry() {
    try {
        state.globalEqStatus = await AudioAPI.equalizerApoStatus();
    } catch (err) {
        console.warn('无法读取 Equalizer APO 状态', err);
    }
    renderGlobalEqEntry();
}

function renderGlobalEqEntry() {
    const connected = Boolean(state.globalEqStatus?.connected);
    dom.globalEqBtn.classList.toggle('active', connected);
    dom.globalEqBtn.classList.toggle('offline', !connected);
    dom.globalEqBtn.title = connected
        ? 'Equalizer APO 已启用，点击调节输出 EQ 与麦克风处理'
        : '通过 Equalizer APO 调节输出 EQ 与麦克风处理';
}

const DEVICE_VOLUME_FOLLOW_ENABLED_KEY = 'audio-hub-device-volume-follow';
const DEVICE_VOLUME_SNAPSHOTS_KEY = 'audio-hub-device-volume-snapshots-v1';

function initDeviceVolumeFollowState() {
    state.deviceVolumeFollowEnabled =
        localStorage.getItem(DEVICE_VOLUME_FOLLOW_ENABLED_KEY) === 'true';
    try {
        const saved = JSON.parse(
            localStorage.getItem(DEVICE_VOLUME_SNAPSHOTS_KEY) || '{}',
        );
        state.deviceVolumeSnapshots = saved && typeof saved === 'object'
            ? saved
            : {};
    } catch {
        state.deviceVolumeSnapshots = {};
    }
    renderDeviceVolumeFollowEntry();
}

function renderDeviceVolumeFollowEntry() {
    const enabled = state.deviceVolumeFollowEnabled;
    dom.deviceVolumeFollowBtn.classList.toggle('active', enabled);
    dom.deviceVolumeFollowBtn.classList.toggle('offline', !enabled);
    dom.deviceVolumeFollowBtn.disabled = state.deviceVolumeFollowBusy;
    dom.deviceVolumeFollowLabel.textContent = state.deviceVolumeFollowBusy
        ? '同步音量…'
        : '音量随扬声器';
    dom.deviceVolumeFollowBtn.title = enabled
        ? '已启用：切换默认输出设备时自动保存并恢复应用音量和静音状态'
        : '按默认输出设备分别记住并恢复应用音量和静音状态';
}

function deviceVolumeSessionKey(session) {
    return unfocusedMuteSessionKey(session);
}

function saveDeviceVolumeSnapshots() {
    localStorage.setItem(
        DEVICE_VOLUME_SNAPSHOTS_KEY,
        JSON.stringify(state.deviceVolumeSnapshots),
    );
}

function saveDeviceVolumeSnapshot(deviceId, deviceName, sessions) {
    if (!deviceId || !Array.isArray(sessions)) return;
    const savedSessions = {};
    const autoMutedKeys = new Set(state.unfocusedMuteStatus.auto_muted_keys || []);
    for (const session of sessions) {
        const key = deviceVolumeSessionKey(session);
        savedSessions[key] = {
            display_name: session.display_name,
            volume: Number(session.volume),
            // 未聚焦静音是临时状态，不应写入扬声器的长期快照。
            muted: Boolean(session.muted) && !autoMutedKeys.has(key),
        };
    }
    state.deviceVolumeSnapshots[deviceId] = {
        device_name: deviceName || '',
        updated_at: Date.now(),
        sessions: savedSessions,
    };
    saveDeviceVolumeSnapshots();
}

function saveCurrentDeviceVolumeSnapshot() {
    if (!state.deviceVolumeFollowEnabled || state.deviceVolumeFollowBusy) return;
    if (!state.defaultOutput) return;
    saveDeviceVolumeSnapshot(
        state.defaultOutput.device_id,
        state.defaultOutput.name,
        state.sessions,
    );
}

function scheduleCurrentDeviceVolumeSnapshot() {
    if (!state.deviceVolumeFollowEnabled || state.deviceVolumeFollowBusy) return;
    if (state.deviceVolumeSnapshotTimer) {
        clearTimeout(state.deviceVolumeSnapshotTimer);
    }
    state.deviceVolumeSnapshotTimer = setTimeout(() => {
        state.deviceVolumeSnapshotTimer = null;
        saveCurrentDeviceVolumeSnapshot();
    }, 150);
}

async function activateDeviceVolumeSnapshot(device) {
    const snapshot = state.deviceVolumeSnapshots[device.device_id];
    if (!snapshot?.sessions) {
        saveCurrentDeviceVolumeSnapshot();
        return { found: false, applied: 0 };
    }

    state.deviceVolumeFollowBusy = true;
    renderDeviceVolumeFollowEntry();
    try {
        const results = await Promise.allSettled(
            state.sessions.map(async (session) => {
                const saved = snapshot.sessions[deviceVolumeSessionKey(session)];
                if (!saved || !Number.isFinite(saved.volume)) return false;
                await AudioAPI.setSessionVolume(
                    session.pid,
                    Math.max(0, Math.min(1, saved.volume)),
                );
                await AudioAPI.setSessionMute(session.pid, Boolean(saved.muted));
                return true;
            }),
        );
        const applied = results.filter(
            (result) => result.status === 'fulfilled' && result.value,
        ).length;
        state.sessions = await AudioAPI.enumerateSessions();
        return { found: true, applied };
    } finally {
        state.deviceVolumeFollowBusy = false;
        renderDeviceVolumeFollowEntry();
    }
}

function toggleDeviceVolumeFollow() {
    if (state.deviceVolumeFollowBusy) return;
    state.deviceVolumeFollowEnabled = !state.deviceVolumeFollowEnabled;
    localStorage.setItem(
        DEVICE_VOLUME_FOLLOW_ENABLED_KEY,
        state.deviceVolumeFollowEnabled ? 'true' : 'false',
    );
    if (state.deviceVolumeFollowEnabled) {
        saveCurrentDeviceVolumeSnapshot();
        setStatus(state.defaultOutput
            ? `已记住 ${state.defaultOutput.name} 的当前应用音量`
            : '已启用音量随扬声器');
    } else {
        setStatus('已关闭音量随扬声器，已保存的设备音量仍会保留');
    }
    renderDeviceVolumeFollowEntry();
}

// ── 会话列表（主体）──────────────────────────────────
function renderSessionList() {
    dom.sessionList.classList.toggle(
        'simple-route-enabled',
        simpleRouteButtonsAvailable(),
    );
    if (state.loading) {
        dom.sessionList.innerHTML = Array(5).fill(
            '<div class="skeleton skeleton-row"></div>',
        ).join('');
        dom.sessionCount.textContent = '—';
        dom.hiddenBadge.classList.add('hidden');
        return;
    }

    // 分离可见和隐藏
    const orderedSessions = [...state.sessions].sort(compareAudioSessions);
    const visible = orderedSessions.filter(
        (session) => !state.hiddenSessions.has(sessionKey(session)),
    );
    const hidden = orderedSessions.filter(
        (session) => state.hiddenSessions.has(sessionKey(session)),
    );

    dom.sessionCount.textContent = `共 ${visible.length} 个`;

    // 隐藏徽章
    if (hidden.length > 0) {
        dom.hiddenBadge.classList.remove('hidden');
        dom.hiddenBadge.textContent = state.showHidden
            ? '👁️ 隐藏中'
            : `👁️ 隐藏 ${hidden.length}`;
        if (state.showHidden) {
            dom.hiddenBadge.classList.add('showing');
        } else {
            dom.hiddenBadge.classList.remove('showing');
        }
    } else {
        dom.hiddenBadge.classList.add('hidden');
    }

    if (visible.length === 0 && hidden.length === 0) {
        dom.sessionList.innerHTML =
            '<div class="empty-state">没有活跃的音频会话</div>';
        return;
    }

    let html = '';

    // 可见应用
    for (const s of visible) {
        html += renderSessionItem(s, false);
    }

    // 隐藏应用（展开时显示）
    if (state.showHidden && hidden.length > 0) {
        html +=
            '<div class="section-divider" style="padding:16px 4px 8px;">👁️ 已隐藏</div>';
        for (const s of hidden) {
            html += renderSessionItem(s, true);
        }
    }

    dom.sessionList.innerHTML = html;
}

function compareAudioSessions(left, right) {
    if (left.pid === 0 && right.pid !== 0) return -1;
    if (right.pid === 0 && left.pid !== 0) return 1;
    const byName = String(left.display_name || '').localeCompare(
        String(right.display_name || ''),
        'zh-CN',
        { numeric: true, sensitivity: 'base' },
    );
    if (byName !== 0) return byName;
    return sessionKey(left).localeCompare(sessionKey(right), 'zh-CN', {
        numeric: true,
        sensitivity: 'base',
    });
}

function renderSessionItem(session, isHidden) {
    const volPct = Math.round(session.volume * 100);
    const muteCls = session.muted ? 'mute-btn muted' : 'mute-btn';
    const muteSvg = session.muted
        ? '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="23" y1="9" x2="17" y2="15"/><line x1="17" y1="9" x2="23" y2="15"/></svg>'
        : '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>';
    const hideSvg = isHidden
        ? '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10"/></svg>'
        : '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>';
    const icon = sessionIcon(session.display_name);
    const hiddenCls = isHidden ? ' hidden-item' : '';
    const stableKey = sessionKey(session);
    const focusMuteManaged = unfocusedMuteApplicationKeys().has(
        unfocusedMuteSessionKey(session),
    );
    const simpleRouteActive = state.simpleRouteStatus.applications.some(
        (application) => application.key === stableKey,
    );
    const simpleRouteControl = simpleRouteButtonsAvailable()
        ? session.pid === 0
            ? '<span class="simple-route-placeholder"></span>'
            : `
                <button class="simple-route-btn ${simpleRouteActive ? 'active' : ''}"
                    data-simple-route data-pid="${session.pid}"
                    data-session-key="${escAttr(stableKey)}"
                    title="${simpleRouteActive
        ? '停止将此应用和物理麦克风传到虚拟麦克风'
        : '将此应用和物理麦克风传到默认虚拟麦克风'}"
                    ${state.simpleRouteBusy ? 'disabled' : ''}>
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none"
                        stroke="currentColor" stroke-width="2" stroke-linecap="round"
                        stroke-linejoin="round">
                        <path d="M4 7h9"/><path d="M11 4l3 3-3 3"/>
                        <path d="M20 17h-9"/><path d="M13 14l-3 3 3 3"/>
                    </svg>
                    <span>${simpleRouteActive ? '流转中' : '传到麦克风'}</span>
                </button>`
        : '';
    const currentDev = state.sessionDevices[stableKey] || '';
    const currentDevName = currentDev
        ? state.outputDevices.find((d) => d.device_id === currentDev)?.name || '设备已断开'
        : '默认';
    const devOpts = state.outputDevices
        .map(
            (d) =>
                `<div class="route-option" data-device-id="${escAttr(d.device_id)}" data-session-key="${escAttr(stableKey)}" data-pid="${session.pid}">${esc(d.name)}</div>`,
        )
        .join('');
    const isRecording =
        state.captureStatus.active && state.captureStatus.pid === session.pid;
    const captureDisabled =
        session.pid === 0 ||
        state.captureBusy ||
        !state.captureStatus.supported ||
        (state.captureStatus.active && !isRecording);
    const captureTitle = session.pid === 0
        ? '系统声音不支持按进程捕获'
        : !state.captureStatus.supported
            ? `需要 Windows Build 20348 或更高版本（当前 ${state.captureStatus.windows_build || '未知'}）`
            : isRecording
                ? '停止录制并保存 WAV'
                : state.captureStatus.active
                    ? '请先停止当前录制'
                    : '录制此应用及其子进程的原始声音';
    const captureSvg = isRecording
        ? '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="5" width="14" height="14" rx="2"/></svg>'
        : '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="12" r="6"/></svg>';
    return `
        <div class="session-item${hiddenCls}${focusMuteManaged ? ' focus-mute-managed' : ''}" data-pid="${session.pid}">
            <span class="session-icon">${icon}</span>
            <span class="session-name" title="${escAttr(focusMuteManaged
        ? `${session.display_name} · 已启用未聚焦自动静音`
        : session.display_name)}">${esc(session.display_name)}</span>
            <div class="volume-slider-wrapper">
                <input type="range"
                       class="volume-slider"
                       min="0" max="100"
                       value="${volPct}"
                       style="--fill: ${volPct}%"
                       data-pid="${session.pid}">
            </div>
            <span class="volume-pct" data-pid="${session.pid}">${volPct}%</span>
            <button class="${muteCls}" data-pid="${session.pid}">${muteSvg}</button>
            ${session.pid === 0
        ? '<span class="route-label-fixed">系统默认</span>'
        : simpleRouteActive
            ? '<span class="route-label-fixed simple-managed">简易流转管理</span>'
            : `
            <div class="route-wrapper" data-pid="${session.pid}">
                <button class="route-trigger${currentDev ? ' locked' : ''}" data-pid="${session.pid}" title="输出设备${currentDev ? '（已锁定）' : ''}">
                    ${currentDev ? '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/><circle cx="12" cy="16" r="1"/></svg>' : '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>'}
                    <span class="route-label">${esc(currentDevName)}</span>
                </button>
                <div class="route-dropdown hidden">
                    <div class="route-option" data-device-id="" data-session-key="${escAttr(stableKey)}" data-pid="${session.pid}">🔊 系统默认</div>
                    ${devOpts}
                </div>
            </div>`}
            ${simpleRouteControl}
            <button class="capture-btn${isRecording ? ' recording' : ''}"
                    data-pid="${session.pid}"
                    title="${escAttr(captureTitle)}"
                    ${captureDisabled ? 'disabled' : ''}>${captureSvg}</button>
            <button class="hide-btn" data-pid="${session.pid}" data-action="${isHidden ? 'unhide' : 'hide'}" title="${isHidden ? '取消隐藏' : '隐藏此应用'}">${hideSvg}</button>
        </div>`;
}

async function refreshCaptureStatus() {
    try {
        state.captureStatus = await AudioAPI.processCaptureStatus();
    } catch (err) {
        console.warn('无法读取进程音频捕获状态', err);
    }
    renderSessionList();
}

function startCaptureStatusPolling() {
    if (state.captureStatusTimer) clearInterval(state.captureStatusTimer);
    state.captureStatusTimer = setInterval(async () => {
        if (!state.captureStatus.active) return;
        const wasActive = state.captureStatus.active;
        await refreshCaptureStatus();
        if (wasActive && !state.captureStatus.active && state.captureStatus.last_error) {
            setStatus(`录制已中断: ${state.captureStatus.last_error}`);
        }
    }, 1000);
}

function formatEqFrequency(frequency) {
    return frequency >= 1000
        ? `${frequency / 1000} kHz`
        : `${frequency} Hz`;
}

function formatEqDb(value) {
    const numeric = Number(value);
    return `${numeric > 0 ? '+' : ''}${numeric.toFixed(1)} dB`;
}

function defaultGlobalEqConfig() {
    const frequencies = [31.5, 63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    return {
        enabled: true,
        preamp_db: 0,
        auto_headroom: true,
        bands: frequencies.map((frequency, index) => ({
            kind: index === 0
                ? 'low_shelf'
                : index === frequencies.length - 1
                    ? 'high_shelf'
                    : 'peaking',
            frequency_hz: frequency,
            gain_db: 0,
            q: index === 0 || index === frequencies.length - 1 ? 0.707 : 1.414,
            enabled: true,
        })),
    };
}

function defaultMicrophoneConfig() {
    return {
        enabled: true,
        gain_db: 8,
        rnnoise_enabled: true,
        rnnoise_mode: 'mono',
    };
}

function isVirtualMicrophone(device) {
    return /voicemeeter|vb-audio|virtual|steam streaming/i.test(device?.name || '');
}

function equalizerApoDevices(devices) {
    return devices.filter(
        (device) => state.equalizerApoEnabledDeviceIds.has(device.device_id),
    );
}

function globalEqEffectivePreamp(config) {
    if (!config?.auto_headroom) return Number(config?.preamp_db || 0);
    const largestBoost = Math.max(
        0,
        ...config.bands
            .filter((band) => band.enabled)
            .map((band) => Number(band.gain_db)),
    );
    return Math.min(Number(config.preamp_db), -largestBoost);
}

// ── VoiceMeeter 流转引擎 ─────────────────────────────
function syncSimpleRouteSessionDevices(previousStatus, nextStatus) {
    const nextKeys = new Set(
        (nextStatus?.applications || []).map((application) => application.key),
    );
    for (const application of previousStatus?.applications || []) {
        if (nextKeys.has(application.key)) continue;
        if (application.original_output_device_id) {
            state.sessionDevices[application.key] = application.original_output_device_id;
        } else {
            delete state.sessionDevices[application.key];
        }
    }
    if (nextStatus?.voicemeeter_input_id) {
        for (const application of nextStatus.applications || []) {
            state.sessionDevices[application.key] = nextStatus.voicemeeter_input_id;
        }
    }
    saveSessionDevices();
}

async function refreshSimpleRouteIntegration() {
    const previousStatus = state.simpleRouteStatus;
    try {
        const [voicemeeterStatus, simpleRouteStatus] = await Promise.all([
            AudioAPI.voicemeeterStatus(),
            AudioAPI.simpleRouteStatus(),
        ]);
        state.voicemeeterStatus = voicemeeterStatus;
        state.voicemeeterConfiguration =
            structuredClone(voicemeeterStatus.configuration);
        state.simpleRouteStatus = simpleRouteStatus;
        syncSimpleRouteSessionDevices(previousStatus, simpleRouteStatus);
        if (simpleRouteStatus.recovery_pending) {
            setStatus('检测到上次未结束的简易流转，可从“声音流转”菜单恢复或全部停止');
        }
        if (
            simpleRouteStatus.active &&
            voicemeeterStatus.connected &&
            simpleRouteStatus.monitor_device_name !== state.defaultOutput?.name
        ) {
            await syncSimpleRouteMonitorToDefault();
        }
    } catch (err) {
        console.warn('无法读取 VoiceMeeter 简易流转状态', err);
    }
    renderVoicemeeterEntry();
    renderSessionList();
}

async function syncSimpleRouteMonitorToDefault() {
    if (!state.simpleRouteStatus.active || !state.voicemeeterStatus?.connected) return;
    if (state.simpleRouteBusy) {
        setTimeout(syncSimpleRouteMonitorToDefault, 200);
        return;
    }
    if (state.simpleRouteMonitorSyncBusy) return;
    state.simpleRouteMonitorSyncBusy = true;
    const previousMonitor = state.simpleRouteStatus.monitor_device_name;
    try {
        const nextStatus = await AudioAPI.syncSimpleRouteMonitor();
        state.simpleRouteStatus = nextStatus;
        const nextMonitor = nextStatus.monitor_device_name || '默认物理扬声器';
        if (nextMonitor !== previousMonitor) {
            setStatus(`默认输出已切换，本地监听已同步到：${nextMonitor}`);
        }
    } catch (err) {
        const currentMonitor = previousMonitor || '原物理扬声器';
        setStatus(`默认输出已切换，但 A1 保持在 ${currentMonitor}：${err}`);
    } finally {
        state.simpleRouteMonitorSyncBusy = false;
        renderVoicemeeterEntry();
    }
}

function simpleRouteButtonsAvailable() {
    return state.voicemeeterMode === 'simple'
        && Boolean(state.voicemeeterStatus?.connected);
}

function renderVoicemeeterEntry() {
    const count = state.simpleRouteStatus.applications.length;
    const connected = Boolean(state.voicemeeterStatus?.connected);
    const installed = Boolean(state.voicemeeterStatus?.installed);
    dom.voicemeeterBtn.classList.toggle('active', connected);
    dom.voicemeeterBtn.classList.toggle('offline', !connected);
    dom.voicemeeterBtn.setAttribute(
        'aria-expanded',
        state.voicemeeterModeMenuOpen ? 'true' : 'false',
    );
    dom.voicemeeterBtnLabel.textContent = !connected
        ? 'VoiceMeeter'
        : state.voicemeeterMode === 'simple'
            ? '简易模式'
            : '高级模式';

    const simpleDescription = !installed
        ? '安装 VoiceMeeter 后可用'
        : !connected
            ? 'VoiceMeeter 未启动，点击启用'
            : count > 0
                ? `本地监听：${esc(state.simpleRouteStatus.monitor_device_name || '默认物理扬声器')}`
                : '使用当前默认物理扬声器进行本地监听';
    dom.voicemeeterModeMenu.innerHTML = `
        <button class="vm-mode-option ${connected && state.voicemeeterMode === 'simple' ? 'selected' : ''}"
            data-vm-entry-action="${connected ? 'select-simple' : installed ? 'start-simple' : 'setup'}">
            <span class="vm-mode-option-icon">↝</span>
            <span><strong>${connected ? '简易模式' : '开启简易模式'}</strong><small>${simpleDescription}</small></span>
            ${connected && state.voicemeeterMode === 'simple' ? '<span class="vm-mode-check">✓</span>' : ''}
        </button>
        <button class="vm-mode-option ${connected && state.voicemeeterMode === 'advanced' ? 'selected' : ''}"
            data-vm-entry-action="advanced">
            <span class="vm-mode-option-icon">⌘</span>
            <span><strong>${connected ? '高级模式' : '开启高级模式'}</strong><small>打开完整 VoiceMeeter 路由工作流</small></span>
            ${connected && state.voicemeeterMode === 'advanced' ? '<span class="vm-mode-check">✓</span>' : ''}
        </button>
        ${connected ? `
            <button class="vm-mode-stop" data-vm-entry-action="close-flow"
                ${state.simpleRouteBusy ? 'disabled' : ''}>
                关闭声音流转并退出 VoiceMeeter
            </button>` : ''}`;
    dom.voicemeeterModeMenu.classList.toggle(
        'hidden',
        !state.voicemeeterModeMenuOpen,
    );
}

async function runVoicemeeterEntryAction(action) {
    if (state.simpleRouteBusy) return;
    if (action === 'select-simple') {
        state.voicemeeterMode = 'simple';
        localStorage.setItem('audio-hub-voicemeeter-mode', 'simple');
        state.voicemeeterModeMenuOpen = false;
        renderVoicemeeterEntry();
        renderSessionList();
        return;
    }
    if (action === 'start-simple') {
        state.simpleRouteBusy = true;
        renderVoicemeeterEntry();
        try {
            state.voicemeeterStatus = await AudioAPI.startVoicemeeter();
            state.voicemeeterMode = 'simple';
            localStorage.setItem('audio-hub-voicemeeter-mode', 'simple');
            state.voicemeeterModeMenuOpen = false;
            setStatus('VoiceMeeter 已启动，可在应用列表中开启简易流转');
        } catch (err) {
            setStatus(`VoiceMeeter 启动失败：${err}`);
        } finally {
            state.simpleRouteBusy = false;
            await refreshSimpleRouteIntegration();
        }
        return;
    }
    if (action === 'setup') {
        state.voicemeeterModeMenuOpen = false;
        renderVoicemeeterEntry();
        await openVoicemeeterEditor();
        return;
    }
    if (action === 'advanced') {
        if (state.simpleRouteStatus.active && !confirm(
            '当前仍有应用正在使用简易模式。是否恢复应用输出和默认麦克风，然后进入高级模式？',
        )) return;
        if (state.simpleRouteStatus.active) {
            await stopAllSimpleRoutes();
            if (state.simpleRouteStatus.active) return;
        }
        if (!state.voicemeeterStatus?.installed) {
            state.voicemeeterMode = 'advanced';
            localStorage.setItem('audio-hub-voicemeeter-mode', 'advanced');
            state.voicemeeterModeMenuOpen = false;
            await openVoicemeeterEditor();
            return;
        }
        if (!state.voicemeeterStatus?.connected) {
            state.simpleRouteBusy = true;
            renderVoicemeeterEntry();
            try {
                state.voicemeeterStatus = await AudioAPI.startVoicemeeter();
                state.voicemeeterConfiguration =
                    structuredClone(state.voicemeeterStatus.configuration);
            } catch (err) {
                setStatus(`VoiceMeeter 启动失败：${err}`);
                state.simpleRouteBusy = false;
                renderVoicemeeterEntry();
                return;
            }
            state.simpleRouteBusy = false;
        }
        state.voicemeeterMode = 'advanced';
        localStorage.setItem('audio-hub-voicemeeter-mode', 'advanced');
        state.voicemeeterModeMenuOpen = false;
        renderVoicemeeterEntry();
        renderSessionList();
        await openVoicemeeterEditor();
        return;
    }
    if (action === 'close-flow') await shutdownVoicemeeterFlow();
}

async function maybeStartVoicemeeter() {
    if (localStorage.getItem('audio-hub-voicemeeter-autostart') !== 'true') return;
    try {
        state.voicemeeterStatus = await AudioAPI.startVoicemeeter();
        state.voicemeeterConfiguration =
            structuredClone(state.voicemeeterStatus.configuration);
        await refreshVoicemeeterApoProcessing();
        setStatus('VoiceMeeter 已在后台启动');
    } catch (err) {
        setStatus(`VoiceMeeter 后台启动失败：${err}`);
    }
}

async function openVoicemeeterEditor() {
    if (state.voicemeeterBusy) return;
    state.voicemeeterBusy = true;
    state.voicemeeterMessage = null;
    state.voicemeeterSourceManagerTarget = null;
    state.voicemeeterDspOpen = false;
    dom.voicemeeterModal.classList.remove('hidden');
    dom.voicemeeterContent.innerHTML =
        '<div class="plugin-loading">正在连接 VoiceMeeter…</div>';
    try {
        state.voicemeeterStatus = await AudioAPI.voicemeeterStatus();
        state.voicemeeterConfiguration =
            structuredClone(state.voicemeeterStatus.configuration);
        await refreshVoicemeeterApoProcessing();
    } catch (err) {
        state.voicemeeterStatus = null;
        state.voicemeeterConfiguration = null;
        state.voicemeeterMessage = `状态读取失败：${err}`;
    } finally {
        state.voicemeeterBusy = false;
        renderVoicemeeterEditor();
        renderVoicemeeterEntry();
        renderSessionList();
    }
}

function closeVoicemeeterEditor() {
    state.voicemeeterSourceManagerTarget = null;
    dom.voicemeeterModal.classList.add('hidden');
}

function voicemeeterMonitorDevices() {
    return state.outputDevices.filter(
        (device) => !/voicemeeter|vb-audio voice|vb-audio voicemeeter/i.test(device.name),
    );
}

function voicemeeterPhysicalInputDevices() {
    return state.inputDevices.filter(
        (device) => !/voicemeeter|vb-audio|virtual|steam streaming/i.test(device.name),
    );
}

function findVoicemeeterDevice(devices, name) {
    const requested = String(name || '').trim().toLocaleLowerCase('zh-CN');
    if (!requested) return null;
    return devices.find(
        (device) => device.name.trim().toLocaleLowerCase('zh-CN') === requested,
    ) || null;
}

async function refreshVoicemeeterApoProcessing() {
    const configuration = state.voicemeeterConfiguration;
    if (!configuration) {
        state.voicemeeterApoProcessing = {
            output_eq_active: false,
            microphone_processing_active: false,
        };
        return;
    }
    const outputDevice = findVoicemeeterDevice(
        state.outputDevices,
        configuration.monitor_device_name,
    );
    const inputDevice = findVoicemeeterDevice(
        state.inputDevices,
        configuration.physical_input?.device_name,
    );
    try {
        state.voicemeeterApoProcessing = await AudioAPI.equalizerApoProcessingState(
            outputDevice?.device_id || null,
            inputDevice?.device_id || null,
        );
    } catch {
        state.voicemeeterApoProcessing = {
            output_eq_active: false,
            microphone_processing_active: false,
        };
    }
}

function voicemeeterPlaybackDevices() {
    const devices = state.outputDevices.filter(
        (device) => /voicemeeter|vb-audio voicemeeter/i.test(device.name),
    );
    const aux = devices.find(
        (device) => /voicemeeter\s+aux\s+input/i.test(device.name),
    ) || devices.find(
        (device) => /aux.*(?:input|vaio)|(?:input|vaio).*aux/i.test(device.name),
    ) || null;
    const main = devices.find(
        (device) => /voicemeeter\s+input\s*\(/i.test(device.name)
            && device.device_id !== aux?.device_id,
    ) || devices.find(
        (device) => /(?:input|vaio)/i.test(device.name)
            && !/aux|vaio3/i.test(device.name),
    ) || null;
    return { main, aux };
}

function voicemeeterApplicationSessions() {
    const sessions = new Map();
    for (const session of state.sessions) {
        if (session.pid === 0) continue;
        const key = sessionKey(session);
        if (!sessions.has(key)) sessions.set(key, session);
    }
    return [...sessions.values()].sort(compareAudioSessions);
}

function voicemeeterRoutedSessions(device) {
    if (!device) return [];
    return voicemeeterApplicationSessions().filter(
        (session) => state.sessionDevices[sessionKey(session)] === device.device_id,
    );
}

function formatVoicemeeterGain(value) {
    const numeric = Number(value || 0);
    return `${numeric > 0 ? '+' : ''}${numeric.toFixed(1)} dB`;
}

function getVoicemeeterField(path) {
    return path.split('.').reduce(
        (value, key) => value?.[key],
        state.voicemeeterConfiguration,
    );
}

function setVoicemeeterField(path, value) {
    const keys = path.split('.');
    let target = state.voicemeeterConfiguration;
    for (const key of keys.slice(0, -1)) {
        if (!target?.[key]) return false;
        target = target[key];
    }
    target[keys.at(-1)] = value;
    return true;
}

function voicemeeterSessionSummary(sessions) {
    if (sessions.length === 0) return '尚未添加应用';
    const names = sessions
        .slice(0, 2)
        .map((session) => esc(session.display_name))
        .join('、');
    return `${names}${sessions.length > 2 ? ` 等 ${sessions.length} 个应用` : ''}`;
}

function renderVoicemeeterMix(key, title, device, sessions, mix) {
    const isMain = key === 'main_mix';
    const virtualBus = isMain ? 'B1' : 'B2';
    const inputLabel = isMain ? 'VAIO' : 'AUX';
    const monitorName = state.voicemeeterConfiguration?.monitor_device_name
        || '尚未选择物理扬声器';
    const virtualMicrophoneName = isMain
        ? 'VoiceMeeter Out B1'
        : 'VoiceMeeter Out B2 / AUX Output';
    const missingDevice = !device;
    const activeRoutes = Number(mix.monitor_enabled)
        + Number(mix.virtual_microphone_enabled);
    return `
        <article class="vm-mix-card ${activeRoutes > 0 ? 'connected' : ''}
            ${mix.input_muted ? 'muted' : ''}">
            <div class="vm-mix-header">
                <div>
                    <span class="vm-node-step">${inputLabel} · 虚拟输入</span>
                    <strong>${title}</strong>
                </div>
                <span class="vm-technical-tag">${inputLabel} → ${virtualBus}</span>
            </div>
            <button class="vm-mix-source ${sessions.length > 0 ? 'active' : ''}
                ${state.voicemeeterSourceManagerTarget === key ? 'selected' : ''}"
                data-vm-action="open-source-manager" data-vm-target="${key}"
                aria-label="为${title}选择应用" ${missingDevice ? 'disabled' : ''}>
                <span aria-hidden="true">▦</span>
                <div>
                    <strong>${voicemeeterSessionSummary(sessions)}</strong>
                    <small>${missingDevice
        ? '未检测到对应的 VoiceMeeter 播放设备'
        : `点击选择应用 · ${esc(device.name)}`}</small>
                </div>
                <span class="vm-mix-source-arrow" aria-hidden="true">›</span>
            </button>
            ${missingDevice ? `
                <p class="vm-inline-warning">
                    无法添加应用，请在 Windows 中确认该虚拟播放设备已启用。
                </p>` : ''}
            <label class="vm-control-label">
                <span>混音输入增益</span>
                <span class="voicemeeter-gain-value"
                    data-vm-value="${key}.input_gain_db">
                    ${formatVoicemeeterGain(mix.input_gain_db)}
                </span>
            </label>
            <input type="range" min="-24" max="12" step="0.5"
                value="${Number(mix.input_gain_db)}"
                data-vm-field="${key}.input_gain_db">
            <button class="btn vm-mute-action ${mix.input_muted ? 'active' : ''}"
                data-vm-action="toggle-field"
                data-vm-field="${key}.input_muted"
                data-vm-on="${title}已静音"
                data-vm-off="${title}已取消静音">
                ${mix.input_muted ? '已静音，点击恢复' : '静音此混音'}
            </button>
            <div class="vm-route-pair">
                <button class="btn vm-route-action
                    ${mix.monitor_enabled ? 'connected' : ''}"
                    data-vm-action="toggle-field"
                    data-vm-field="${key}.monitor_enabled"
                    data-vm-on="${title}已输出到 ${escAttr(monitorName)}"
                    data-vm-off="${title}已停止输出到物理扬声器"
                    aria-label="输出到物理扬声器，A1 当前绑定 ${escAttr(monitorName)}"
                    title="A1 当前绑定：${escAttr(monitorName)}">
                    <span class="vm-route-action-dot"></span>
                    <span class="vm-route-action-content">
                        <strong>输出到物理扬声器</strong>
                        <small>A1 · ${esc(monitorName)}</small>
                    </span>
                </button>
                <button class="btn vm-route-action
                    ${mix.virtual_microphone_enabled ? 'connected' : ''}"
                    data-vm-action="toggle-field"
                    data-vm-field="${key}.virtual_microphone_enabled"
                    data-vm-on="${title}已输出到虚拟麦克风 ${virtualBus}"
                    data-vm-off="${title}已停止输出到虚拟麦克风 ${virtualBus}"
                    aria-label="输出到虚拟麦克风 ${virtualBus}">
                    <span class="vm-route-action-dot"></span>
                    <span class="vm-route-action-content">
                        <strong>输出到虚拟麦克风</strong>
                        <small>${virtualBus} · ${virtualMicrophoneName}</small>
                    </span>
                </button>
            </div>
            <div class="vm-bus-control">
                <label class="vm-control-label">
                    <span>虚拟麦克风输出增益（${virtualBus}）</span>
                    <span class="voicemeeter-gain-value"
                        data-vm-value="${key}.virtual_microphone_gain_db">
                        ${formatVoicemeeterGain(mix.virtual_microphone_gain_db)}
                    </span>
                </label>
                <input type="range" min="-24" max="12" step="0.5"
                    value="${Number(mix.virtual_microphone_gain_db)}"
                    data-vm-field="${key}.virtual_microphone_gain_db">
                <button class="btn vm-compact-mute
                    ${mix.virtual_microphone_muted ? 'active' : ''}"
                    data-vm-action="toggle-field"
                    data-vm-field="${key}.virtual_microphone_muted"
                    data-vm-on="虚拟麦克风 ${virtualBus} 已静音"
                    data-vm-off="虚拟麦克风 ${virtualBus} 已取消静音">
                    ${mix.virtual_microphone_muted
        ? `虚拟麦克风已静音（${virtualBus}）`
        : `静音虚拟麦克风（${virtualBus}）`}
                </button>
            </div>
        </article>`;
}

function formatVoicemeeterFrequency(value) {
    const frequency = Number(value || 0);
    return frequency >= 1000
        ? `${(frequency / 1000).toFixed(frequency >= 10_000 ? 0 : 1)} kHz`
        : `${Math.round(frequency)} Hz`;
}

function renderVoicemeeterEq(path, equalizer, title, description) {
    if (!equalizer) return '';
    const bands = equalizer.bands.map((band, index) => {
        const bandPath = `${path}.bands.${index}`;
        return `
            <div class="vm-eq-band ${band.enabled ? '' : 'disabled'}">
                <label class="vm-eq-band-heading">
                    <input type="checkbox" data-vm-field="${bandPath}.enabled"
                        ${band.enabled ? 'checked' : ''}>
                    <span>${formatVoicemeeterFrequency(band.frequency_hz)}</span>
                </label>
                <span class="voicemeeter-gain-value"
                    data-vm-value="${bandPath}.gain_db">
                    ${formatVoicemeeterGain(band.gain_db)}
                </span>
                <input type="range" min="-12" max="12" step="0.5"
                    value="${Number(band.gain_db)}"
                    data-vm-field="${bandPath}.gain_db"
                    ${band.enabled ? '' : 'disabled'}>
            </div>`;
    }).join('');
    return `
        <article class="vm-dsp-card">
            <div class="vm-dsp-card-heading">
                <div>
                    <span class="vm-workflow-eyebrow">六段参数均衡</span>
                    <strong>${title}</strong>
                </div>
                <button class="btn vm-dsp-toggle ${equalizer.enabled ? 'active' : ''}"
                    data-vm-action="toggle-field" data-vm-field="${path}.enabled"
                    data-vm-on="${title}已启用" data-vm-off="${title}已关闭">
                    ${equalizer.enabled ? '已启用' : '已关闭'}
                </button>
            </div>
            <p>${description}</p>
            <div class="vm-eq-bands">${bands}</div>
        </article>`;
}

function renderVoicemeeterStrength(path, title, value, description) {
    if (value === null || value === undefined) return '';
    return `
        <label class="vm-dsp-strength">
            <span>
                <strong>${title}</strong>
                <small>${description}</small>
            </span>
            <span class="voicemeeter-gain-value" data-vm-value="${path}">
                ${Number(value).toFixed(1)} / 10
            </span>
            <input type="range" min="0" max="10" step="0.1"
                value="${Number(value)}" data-vm-field="${path}"
                data-vm-format="strength">
        </label>`;
}

function renderVoicemeeterDsp(status, configuration) {
    const physical = configuration.physical_input;
    const microphoneControls = [
        renderVoicemeeterStrength(
            'physical_input.audibility',
            '可听度',
            physical.audibility,
            'Standard 的简化增强，会联动压缩与噪声门。',
        ),
        renderVoicemeeterStrength(
            'physical_input.compressor',
            '压缩器',
            physical.compressor,
            '收窄音量差距，让说话声更稳定。',
        ),
        renderVoicemeeterStrength(
            'physical_input.noise_gate',
            '噪声门',
            physical.noise_gate,
            '音量较小时自动压低底噪；过高会吞掉轻声。',
        ),
        renderVoicemeeterStrength(
            'physical_input.denoiser',
            '降噪',
            physical.denoiser,
            'Potato 内置降噪，用于持续的环境噪声。',
        ),
    ].filter(Boolean).join('');
    const vmOutputActive = Boolean(configuration.a1_equalizer?.enabled);
    const vmMicrophoneActive = [
        physical.audibility,
        physical.compressor,
        physical.noise_gate,
        physical.denoiser,
    ].some((value) => Number(value || 0) > 0)
        || Boolean(physical.equalizer?.enabled);
    const apo = state.voicemeeterApoProcessing;
    const activeCount = [
        physical.audibility,
        physical.compressor,
        physical.noise_gate,
        physical.denoiser,
    ].filter((value) => Number(value || 0) > 0).length
        + Number(Boolean(physical.equalizer?.enabled))
        + Number(Boolean(configuration.a1_equalizer?.enabled));
    const hasDoubleProcessing = (apo.output_eq_active && vmOutputActive)
        || (apo.microphone_processing_active && vmMicrophoneActive);
    const notes = [];
    if (apo.output_eq_active) {
        notes.push(`
            <div class="vm-processing-note ${vmOutputActive ? 'warning' : ''}">
                <strong>${vmOutputActive ? '检测到扬声器双重均衡' : '该扬声器正在使用 Equalizer APO'}</strong>
                <span>${vmOutputActive
        ? 'VoiceMeeter A1 均衡与 Equalizer APO 会依次处理声音，增益可能叠加。'
        : '如果再启用下方 A1 均衡，将形成两级处理；Audio Hub 不会自动关闭任何一方。'}</span>
            </div>`);
    }
    if (apo.microphone_processing_active) {
        notes.push(`
            <div class="vm-processing-note ${vmMicrophoneActive ? 'warning' : ''}">
                <strong>${vmMicrophoneActive ? '检测到麦克风双重处理' : '该麦克风正在使用 Equalizer APO'}</strong>
                <span>${vmMicrophoneActive
        ? 'Equalizer APO 的麦克风增强与 VoiceMeeter 增强会串联，可能导致泵动、失真或吞字。'
        : '启用下方增强后会形成两级处理；建议每种效果只在一处开启。'}</span>
            </div>`);
    }
    const microphoneEq = renderVoicemeeterEq(
        'physical_input.equalizer',
        physical.equalizer,
        '物理麦克风均衡',
        '仅 Potato 可用。保留 VoiceMeeter 当前滤波器的频率、类型和 Q 值。',
    );
    const a1Eq = renderVoicemeeterEq(
        'a1_equalizer',
        configuration.a1_equalizer,
        'A1 扬声器均衡',
        'Banana / Potato 可用。处理进入 A1 物理扬声器前的整路混音。',
    );
    return `
        <section class="vm-dsp-section ${state.voicemeeterDspOpen ? 'open' : ''}">
            <div class="vm-section-heading">
                <div>
                    <span class="vm-workflow-eyebrow">可选声音处理</span>
                    <h4>VoiceMeeter DSP · ${esc(status.edition || '')}</h4>
                </div>
                <div class="vm-dsp-heading-actions">
                    <span class="vm-dsp-summary ${hasDoubleProcessing ? 'warning' : ''}">
                        ${hasDoubleProcessing ? '检测到双重处理' : `${activeCount} 项已开启`}
                    </span>
                    <button class="btn eq-secondary-btn" data-vm-action="toggle-dsp"
                        aria-expanded="${state.voicemeeterDspOpen}">
                        ${state.voicemeeterDspOpen ? '收起' : '展开'}
                    </button>
                </div>
            </div>
            ${state.voicemeeterDspOpen ? `
                <div class="vm-dsp-content">
                    <p class="vm-section-copy">
                        这些效果只处理经过 VoiceMeeter 的路径；Equalizer APO 仍适合设备级耳机 EQ 与直接麦克风处理。
                    </p>
                    ${notes.length ? `<div class="vm-processing-notes">${notes.join('')}</div>` : ''}
                    <div class="vm-dsp-grid">
                        <article class="vm-dsp-card">
                            <div class="vm-dsp-card-heading">
                                <div>
                                    <span class="vm-workflow-eyebrow">硬件输入 1</span>
                                    <strong>麦克风增强</strong>
                                </div>
                            </div>
                            <div class="vm-dsp-strengths">${microphoneControls}</div>
                        </article>
                        ${microphoneEq}
                        ${a1Eq}
                    </div>
                </div>` : ''}
        </section>`;
}

function renderVoicemeeterApplicationManager(playbackDevices) {
    const target = state.voicemeeterSourceManagerTarget;
    if (!target) return '';
    const isMain = target === 'main_mix';
    const targetDevice = isMain ? playbackDevices.main : playbackDevices.aux;
    if (!targetDevice) return '';
    const targetTitle = isMain ? '主混音' : 'AUX 混音';
    const sessions = voicemeeterApplicationSessions();
    if (sessions.length === 0) {
        return `
            <div class="vm-source-menu-layer" data-vm-source-dismiss>
            <section class="vm-source-menu" role="dialog" aria-modal="true"
                aria-label="为${targetTitle}选择应用">
                <div class="vm-section-heading">
                    <div>
                        <span class="vm-workflow-eyebrow">高级模式 · 应用来源</span>
                        <h4>为${targetTitle}选择应用</h4>
                    </div>
                    <button class="btn eq-secondary-btn"
                        data-vm-action="close-source-manager">关闭</button>
                </div>
                <p class="vm-empty-copy">当前没有活跃的应用音频会话。先播放一段声音，再刷新状态。</p>
            </section>
            </div>`;
    }
    const rows = sessions.map((session) => {
        const key = sessionKey(session);
        const currentId = state.sessionDevices[key] || '';
        const currentDevice = state.outputDevices.find(
            (device) => device.device_id === currentId,
        );
        const selected = currentId === targetDevice.device_id;
        const routeDescription = selected
            ? `已加入${targetTitle}，点击移除`
            : currentId
                ? `当前输出：${currentDevice?.name || '其他设备'}，点击移到${targetTitle}`
                : `点击加入${targetTitle}`;
        return `
            <button class="vm-app-route-row ${selected ? 'selected' : ''}"
                data-vm-app-route data-session-key="${escAttr(key)}"
                data-pid="${session.pid}"
                data-device-id="${escAttr(targetDevice.device_id)}">
                <span class="vm-app-route-icon">${sessionIcon(session.display_name)}</span>
                <span class="vm-app-route-copy">
                    <strong class="vm-app-route-name" title="${escAttr(session.display_name)}">
                        ${esc(session.display_name)}
                    </strong>
                    <small>${esc(routeDescription)}</small>
                </span>
                <span class="vm-app-route-state">${selected ? '移除' : '加入'}</span>
            </button>`;
    }).join('');
    return `
        <div class="vm-source-menu-layer" data-vm-source-dismiss>
        <section class="vm-source-menu" role="dialog" aria-modal="true"
            aria-label="为${targetTitle}选择应用">
            <div class="vm-section-heading">
                <div>
                    <span class="vm-workflow-eyebrow">高级模式 · 应用来源</span>
                    <h4>为${targetTitle}选择应用</h4>
                </div>
                <button class="btn eq-secondary-btn"
                    data-vm-action="close-source-manager">关闭</button>
            </div>
            <p class="vm-section-copy">
                点击应用即可加入${targetTitle}；再次点击已加入的应用可将其移除。
            </p>
            <div class="vm-app-route-list">${rows}</div>
        </section>
        </div>`;
}

function renderVoicemeeterEditor() {
    const status = state.voicemeeterStatus;
    const autoStart = localStorage.getItem('audio-hub-voicemeeter-autostart') === 'true';
    if (!status) {
        dom.voicemeeterContent.innerHTML = `
            <div class="voicemeeter-body">
                <p class="plugin-error">${esc(state.voicemeeterMessage || '无法读取 VoiceMeeter 状态')}</p>
                <div class="voicemeeter-footer">
                    <span></span>
                    <button class="btn eq-secondary-btn" data-vm-action="refresh">重新检测</button>
                </div>
            </div>`;
        return;
    }

    if (!status.installed) {
        dom.voicemeeterContent.innerHTML = `
            <div class="voicemeeter-body">
                <div class="plugin-state-card">
                    <span class="plugin-state-dot missing"></span>
                    <div>
                        <strong>未检测到 VoiceMeeter</strong>
                        <p>${esc(status.note)}</p>
                    </div>
                    <button class="btn eq-primary-btn plugin-connect-btn"
                        data-vm-action="download">前往官方下载</button>
                </div>
                <p class="plugin-safety-note">
                    VoiceMeeter 需要由用户单独安装并在安装后重启 Windows。
                    Audio Hub 不会静默安装或把它打包进便携版。
                </p>
                <p class="voicemeeter-attribution">
                    VoiceMeeter 是 VB-Audio 提供的 donationware。
                    <a href="https://vb-audio.com/Voicemeeter/">官方网站</a>
                </p>
            </div>`;
        return;
    }

    const stateClass = status.running ? 'connected' : 'ready';
    const stateTitle = status.running
        ? `${status.edition || 'VoiceMeeter'}：已连接`
        : 'VoiceMeeter：已安装，未运行';
    const runningActions = status.running
        ? `<button class="btn eq-secondary-btn" data-vm-action="show">打开原界面</button>`
        : `<button class="btn eq-primary-btn" data-vm-action="start">启动音频引擎</button>`;
    const configuration = state.voicemeeterConfiguration;

    let controls = '';
    if (status.running && configuration) {
        const monitorDevices = voicemeeterMonitorDevices();
        const physicalDevices = voicemeeterPhysicalInputDevices();
        const playbackDevices = voicemeeterPlaybackDevices();
        const mainSessions = voicemeeterRoutedSessions(playbackDevices.main);
        const auxSessions = voicemeeterRoutedSessions(playbackDevices.aux);
        const currentDevice = configuration.monitor_device_name || '';
        const hasCurrentDevice = monitorDevices.some(
            (device) => device.name === currentDevice,
        );
        const currentOption = currentDevice && !hasCurrentDevice
            ? `<option value="${escAttr(currentDevice)}" selected>${esc(currentDevice)}</option>`
            : '';
        const emptyDeviceOption = currentDevice
            ? ''
            : '<option value="" selected disabled>选择耳机或扬声器</option>';
        const monitorOptions = monitorDevices.map((device) => `
            <option value="${escAttr(device.name)}"
                ${device.name === currentDevice ? 'selected' : ''}>
                ${esc(device.name)}${device.is_default ? '（系统默认）' : ''}
            </option>`).join('');
        const physicalInput = configuration.physical_input;
        const physicalDeviceName = physicalInput.device_name || '';
        const hasPhysicalDevice = physicalDevices.some(
            (device) => device.name === physicalDeviceName,
        );
        const missingPhysicalOption = physicalDeviceName && !hasPhysicalDevice
            ? `<option value="${escAttr(physicalDeviceName)}" selected>
                ${esc(physicalDeviceName)}（当前设备）
            </option>`
            : '';
        const physicalOptions = physicalDevices.map((device) => `
            <option value="${escAttr(device.name)}"
                ${device.name === physicalDeviceName ? 'selected' : ''}>
                ${esc(device.name)}${device.is_default ? '（系统默认）' : ''}
            </option>`).join('');
        const activeRoutes = Number(configuration.main_mix.monitor_enabled)
            + Number(configuration.main_mix.virtual_microphone_enabled)
            + Number(configuration.aux_mix?.monitor_enabled)
            + Number(configuration.aux_mix?.virtual_microphone_enabled)
            + Number(physicalInput.monitor_enabled)
            + Number(physicalInput.main_mix_enabled)
            + Number(physicalInput.aux_mix_enabled);
        const maximumRoutes = configuration.aux_mix ? 7 : 4;
        const anyRouteEnabled = activeRoutes > 0;
        const sourceManager = renderVoicemeeterApplicationManager(playbackDevices);

        controls = `
            <section class="vm-workflow ${state.voicemeeterBusy
        ? 'voicemeeter-disabled' : ''}">
                <div class="vm-workflow-heading">
                    <div>
                        <span class="vm-workflow-eyebrow">双混音工作流</span>
                        <h3>组合应用声音与物理麦克风</h3>
                    </div>
                    <div class="vm-workflow-heading-actions">
                        <span class="vm-route-count ${anyRouteEnabled ? 'active' : ''}">
                            ${activeRoutes}/${maximumRoutes} 条路由已连接
                        </span>
                    </div>
                </div>
                <div class="vm-shared-output">
                    <div>
                        <span class="vm-node-step">物理输出 · A1</span>
                        <strong>实际播放声音的扬声器或耳机</strong>
                    </div>
                    <select class="device-select" data-vm-field="monitor_device_name"
                        aria-label="实际播放声音的物理扬声器或耳机，VoiceMeeter A1">
                        ${emptyDeviceOption}
                        ${currentOption}
                        ${monitorOptions}
                    </select>
                </div>
                <div class="vm-mixer-grid">
                    <article class="vm-physical-card
                        ${physicalDeviceName ? 'connected' : 'attention'}
                        ${physicalInput.muted ? 'muted' : ''}">
                        <div class="vm-mix-header">
                            <div>
                                <span class="vm-node-step">硬件输入 1</span>
                                <strong>物理麦克风</strong>
                            </div>
                            <span class="vm-technical-tag">Strip 1</span>
                        </div>
                        <select class="device-select"
                            data-vm-field="physical_input.device_name"
                            aria-label="物理麦克风设备">
                            <option value="" ${physicalDeviceName ? '' : 'selected'}>
                                不使用物理输入
                            </option>
                            ${missingPhysicalOption}
                            ${physicalOptions}
                        </select>
                        <label class="vm-control-label">
                            <span>输入增益</span>
                            <span class="voicemeeter-gain-value"
                                data-vm-value="physical_input.gain_db">
                                ${formatVoicemeeterGain(physicalInput.gain_db)}
                            </span>
                        </label>
                        <input type="range" min="-24" max="12" step="0.5"
                            value="${Number(physicalInput.gain_db)}"
                            data-vm-field="physical_input.gain_db">
                        <button class="btn vm-mute-action
                            ${physicalInput.muted ? 'active' : ''}"
                            data-vm-action="toggle-field"
                            data-vm-field="physical_input.muted"
                            data-vm-on="物理麦克风已静音"
                            data-vm-off="物理麦克风已取消静音">
                            ${physicalInput.muted ? '已静音，点击恢复' : '静音物理麦克风'}
                        </button>
                        <div class="vm-physical-routes">
                            <button class="btn vm-route-action
                                ${physicalInput.monitor_enabled ? 'connected' : ''}"
                                data-vm-action="toggle-field"
                                data-vm-field="physical_input.monitor_enabled"
                                data-vm-on="物理麦克风已输出到 ${escAttr(
        currentDevice || '物理扬声器',
    )}"
                                data-vm-off="物理麦克风已停止输出到物理扬声器"
                                aria-label="输出到物理扬声器，A1 当前绑定 ${escAttr(
        currentDevice || '尚未选择设备',
    )}"
                                title="A1 当前绑定：${escAttr(
        currentDevice || '尚未选择设备',
    )}"
                                ${physicalDeviceName ? '' : 'disabled'}>
                                <span class="vm-route-action-dot"></span>
                                <span class="vm-route-action-content">
                                    <strong>输出到物理扬声器</strong>
                                    <small>A1 · ${esc(currentDevice || '尚未选择设备')}</small>
                                </span>
                            </button>
                            <button class="btn vm-route-action
                                ${physicalInput.main_mix_enabled ? 'connected' : ''}"
                                data-vm-action="toggle-field"
                                data-vm-field="physical_input.main_mix_enabled"
                                data-vm-on="物理麦克风已输出到虚拟麦克风 B1"
                                data-vm-off="物理麦克风已停止输出到虚拟麦克风 B1"
                                aria-label="输出到虚拟麦克风 B1"
                                ${physicalDeviceName ? '' : 'disabled'}>
                                <span class="vm-route-action-dot"></span>
                                <span class="vm-route-action-content">
                                    <strong>输出到虚拟麦克风</strong>
                                    <small>B1 · VoiceMeeter Out B1</small>
                                </span>
                            </button>
                            ${configuration.aux_mix ? `
                                <button class="btn vm-route-action
                                    ${physicalInput.aux_mix_enabled ? 'connected' : ''}"
                                    data-vm-action="toggle-field"
                                    data-vm-field="physical_input.aux_mix_enabled"
                                    data-vm-on="物理麦克风已输出到虚拟麦克风 B2"
                                    data-vm-off="物理麦克风已停止输出到虚拟麦克风 B2"
                                    aria-label="输出到虚拟麦克风 B2"
                                    ${physicalDeviceName ? '' : 'disabled'}>
                                    <span class="vm-route-action-dot"></span>
                                    <span class="vm-route-action-content">
                                        <strong>输出到虚拟麦克风</strong>
                                        <small>B2 · VoiceMeeter Out B2 / AUX Output</small>
                                    </span>
                                </button>` : ''}
                        </div>
                    </article>
                    ${renderVoicemeeterMix(
        'main_mix',
        '主混音',
        playbackDevices.main,
        mainSessions,
        configuration.main_mix,
    )}
                    ${configuration.aux_mix ? renderVoicemeeterMix(
        'aux_mix',
        'AUX 混音',
        playbackDevices.aux,
        auxSessions,
        configuration.aux_mix,
    ) : `
                        <article class="vm-aux-unavailable">
                            <span class="vm-node-step">AUX · 不可用</span>
                            <strong>Standard 版不支持 AUX 混音</strong>
                            <p>安装 VoiceMeeter Banana 或 Potato 后，可使用
                                VoiceMeeter AUX Input 与 B2 独立输出。</p>
                        </article>`}
                </div>
                ${renderVoicemeeterDsp(status, configuration)}
                <div class="vm-workflow-hint">
                    <span>提示</span>
                    <p>聊天软件选择 VoiceMeeter Out B1 使用主混音；
                        选择 VoiceMeeter AUX Output 使用 AUX 混音。</p>
                </div>
            </section>
            ${sourceManager}`;
    }

    dom.voicemeeterContent.innerHTML = `
        <div class="voicemeeter-body">
            <div class="plugin-state-card">
                <span class="plugin-state-dot ${stateClass}"></span>
                <div>
                    <strong>${esc(stateTitle)}</strong>
                    <p>${esc(status.note)}</p>
                </div>
                <div class="voicemeeter-state-actions">${runningActions}</div>
            </div>
            ${controls}
            <p class="plugin-safety-note">
                想让 VoiceMeeter 不显示主窗口：在 VoiceMeeter 菜单中启用
                “System Tray”，并关闭 “Show App On Startup”。它仍会作为音频引擎运行。
            </p>
            ${state.voicemeeterMessage
        ? `<p class="plugin-result">${esc(state.voicemeeterMessage)}</p>` : ''}
            <div class="voicemeeter-footer">
                <label class="voicemeeter-background-row">
                    <input type="checkbox" id="voicemeeter-background-start"
                        ${autoStart ? 'checked' : ''}>
                    <span>Audio Hub 启动时自动启动 VoiceMeeter</span>
                </label>
                <div class="voicemeeter-footer-actions">
                    <button class="btn eq-secondary-btn" data-vm-action="refresh">刷新状态</button>
                    ${status.running
        ? '<button class="btn eq-secondary-btn" data-vm-action="restart">重启音频引擎</button>'
        : ''}
                </div>
            </div>
            <p class="voicemeeter-attribution">
                VoiceMeeter 由 VB-Audio 提供，Audio Hub 仅通过官方 Remote API 控制。
            </p>
        </div>`;
}

async function applyVoicemeeterConfiguration(message = 'VoiceMeeter 参数已更新') {
    if (state.voicemeeterBusy || !state.voicemeeterConfiguration) return;
    const requestedConfiguration = structuredClone(state.voicemeeterConfiguration);
    const previousConfiguration = structuredClone(
        state.voicemeeterStatus?.configuration ?? state.voicemeeterConfiguration,
    );
    state.voicemeeterBusy = true;
    renderVoicemeeterEditor();
    try {
        const updatedStatus = await AudioAPI.applyVoicemeeterConfiguration(
            requestedConfiguration,
        );
        updatedStatus.configuration = structuredClone(requestedConfiguration);
        state.voicemeeterStatus = updatedStatus;
        state.voicemeeterConfiguration = structuredClone(requestedConfiguration);
        await refreshVoicemeeterApoProcessing();
        state.voicemeeterMessage = message;
        setStatus(message);
    } catch (err) {
        state.voicemeeterConfiguration = previousConfiguration;
        if (state.voicemeeterStatus) {
            state.voicemeeterStatus.configuration =
                structuredClone(previousConfiguration);
        }
        state.voicemeeterMessage = `更新失败：${err}`;
        setStatus(`VoiceMeeter 更新失败：${err}`);
    } finally {
        state.voicemeeterBusy = false;
        renderVoicemeeterEditor();
    }
}

async function toggleSimpleRouteApplication(button) {
    if (state.simpleRouteBusy || !state.voicemeeterStatus?.connected) return;
    const pid = Number(button.dataset.pid);
    const key = button.dataset.sessionKey;
    const session = state.sessions.find((item) => item.pid === pid);
    if (!session || !key) return;
    const previousStatus = structuredClone(state.simpleRouteStatus);
    const enabled = previousStatus.applications.some(
        (application) => application.key === key,
    );
    if (enabled && previousStatus.applications.length === 1) {
        await shutdownVoicemeeterFlow();
        return;
    }
    state.simpleRouteBusy = true;
    renderSessionList();
    renderVoicemeeterEntry();
    try {
        const nextStatus = enabled
            ? await AudioAPI.disableSimpleRouteApplication(key, pid)
            : await AudioAPI.enableSimpleRouteApplication(
                pid,
                key,
                session.display_name,
            );
        state.simpleRouteStatus = nextStatus;
        syncSimpleRouteSessionDevices(previousStatus, nextStatus);
        const inputDevices = await AudioAPI.enumerateDevices('Input');
        state.inputDevices = inputDevices;
        state.defaultInput = inputDevices.find((device) => device.is_default) || null;
        renderStatusbar();
        setStatus(enabled
            ? `${session.display_name} 已停止流转并恢复原输出`
            : `${session.display_name} 与物理麦克风已传到默认虚拟麦克风；本地监听：${nextStatus.monitor_device_name || '默认物理扬声器'}`);
    } catch (err) {
        setStatus(`简易流转失败：${err}`);
    } finally {
        state.simpleRouteBusy = false;
        renderSessionList();
        renderVoicemeeterEntry();
    }
}

function inactiveSimpleRouteStatus() {
    return {
        active: false,
        applications: [],
        physical_microphone_name: null,
        monitor_device_name: null,
        virtual_microphone_id: null,
        voicemeeter_input_id: null,
        recovery_pending: false,
    };
}

async function shutdownVoicemeeterFlow() {
    if (state.simpleRouteBusy || state.voicemeeterBusy) return;
    const previousStatus = structuredClone(state.simpleRouteStatus);
    state.simpleRouteBusy = true;
    renderSessionList();
    renderVoicemeeterEntry();
    try {
        const voicemeeterStatus = await AudioAPI.shutdownVoicemeeter();
        const nextStatus = inactiveSimpleRouteStatus();
        state.simpleRouteStatus = nextStatus;
        syncSimpleRouteSessionDevices(previousStatus, nextStatus);
        state.voicemeeterStatus = voicemeeterStatus;
        state.voicemeeterConfiguration = null;
        state.voicemeeterModeMenuOpen = false;
        closeVoicemeeterEditor();
        const inputDevices = await AudioAPI.enumerateDevices('Input');
        state.inputDevices = inputDevices;
        state.defaultInput = inputDevices.find((device) => device.is_default) || null;
        renderStatusbar();
        setStatus('声音流转已关闭，VoiceMeeter 已退出');
    } catch (err) {
        await refreshSimpleRouteIntegration();
        setStatus(`关闭声音流转失败：${err}`);
    } finally {
        state.simpleRouteBusy = false;
        renderSessionList();
        renderVoicemeeterEntry();
    }
}

async function stopAllSimpleRoutes() {
    if (state.simpleRouteBusy || !state.simpleRouteStatus.active) return;
    const previousStatus = structuredClone(state.simpleRouteStatus);
    state.simpleRouteBusy = true;
    renderSessionList();
    renderVoicemeeterEntry();
    try {
        const nextStatus = await AudioAPI.stopAllSimpleRoutes();
        state.simpleRouteStatus = nextStatus;
        syncSimpleRouteSessionDevices(previousStatus, nextStatus);
        const inputDevices = await AudioAPI.enumerateDevices('Input');
        state.inputDevices = inputDevices;
        state.defaultInput = inputDevices.find((device) => device.is_default) || null;
        renderStatusbar();
        state.voicemeeterModeMenuOpen = false;
        setStatus('已退出简易模式，应用输出和默认麦克风已恢复');
    } catch (err) {
        setStatus(`退出简易模式失败：${err}`);
    } finally {
        state.simpleRouteBusy = false;
        renderSessionList();
        renderVoicemeeterEntry();
    }
}

async function routeVoicemeeterApplication(control) {
    if (state.voicemeeterBusy) return;
    const pid = Number(control.dataset.pid);
    const stableKey = control.dataset.sessionKey;
    const requestedDeviceId = control.dataset.deviceId;
    const deviceId = state.sessionDevices[stableKey] === requestedDeviceId
        ? ''
        : requestedDeviceId;
    state.voicemeeterBusy = true;
    renderVoicemeeterEditor();
    try {
        await AudioAPI.setAppOutputDevice(pid, deviceId);
        if (deviceId) {
            state.sessionDevices[stableKey] = deviceId;
        } else {
            delete state.sessionDevices[stableKey];
        }
        saveSessionDevices();
        renderSessionList();
        state.voicemeeterMessage = deviceId
            ? '应用已加入 VoiceMeeter 工作流'
            : '应用已恢复系统默认输出';
        setStatus(state.voicemeeterMessage);
    } catch (err) {
        state.voicemeeterMessage = `应用路由失败：${err}`;
        setStatus(state.voicemeeterMessage);
    } finally {
        state.voicemeeterBusy = false;
        renderVoicemeeterEditor();
    }
}

async function runVoicemeeterAction(action, field, button) {
    if (state.voicemeeterBusy) return;
    if (action === 'download') {
        await AudioAPI.openVoicemeeterDownload()
            .catch((err) => setStatus(`打开 VoiceMeeter 下载页失败：${err}`));
        return;
    }
    if (action === 'open-source-manager') {
        const target = button?.dataset.vmTarget;
        state.voicemeeterSourceManagerTarget =
            state.voicemeeterSourceManagerTarget === target ? null : target;
        renderVoicemeeterEditor();
        return;
    }
    if (action === 'close-source-manager') {
        state.voicemeeterSourceManagerTarget = null;
        renderVoicemeeterEditor();
        return;
    }
    if (action === 'toggle-dsp') {
        state.voicemeeterDspOpen = !state.voicemeeterDspOpen;
        renderVoicemeeterEditor();
        return;
    }
    if (action === 'toggle-field' && field) {
        const nextValue = !getVoicemeeterField(field);
        if (!setVoicemeeterField(field, nextValue)) return;
        await applyVoicemeeterConfiguration(
            nextValue ? button?.dataset.vmOn : button?.dataset.vmOff,
        );
        return;
    }

    state.voicemeeterBusy = true;
    renderVoicemeeterEditor();
    try {
        if (action === 'start') {
            state.voicemeeterStatus = await AudioAPI.startVoicemeeter();
            state.voicemeeterMessage = 'VoiceMeeter 已启动';
        } else if (action === 'show') {
            state.voicemeeterStatus = await AudioAPI.showVoicemeeter();
        } else if (action === 'restart') {
            state.voicemeeterStatus = await AudioAPI.restartVoicemeeterAudioEngine();
            state.voicemeeterMessage = 'VoiceMeeter 音频引擎已重启';
        } else if (action === 'refresh') {
            const [status, outputDevices, inputDevices, sessions] = await Promise.all([
                AudioAPI.voicemeeterStatus(),
                AudioAPI.enumerateDevices('Output'),
                AudioAPI.enumerateDevices('Input'),
                AudioAPI.enumerateSessions(),
            ]);
            state.voicemeeterStatus = status;
            state.outputDevices = outputDevices;
            state.inputDevices = inputDevices;
            state.sessions = sessions;
            renderAll();
            state.voicemeeterMessage = null;
        }
        state.voicemeeterConfiguration =
            structuredClone(state.voicemeeterStatus.configuration);
        await refreshVoicemeeterApoProcessing();
    } catch (err) {
        state.voicemeeterMessage = `操作失败：${err}`;
        setStatus(`VoiceMeeter 操作失败：${err}`);
    } finally {
        state.voicemeeterBusy = false;
        renderVoicemeeterEditor();
    }
}

async function openGlobalEqEditor() {
    if (state.globalEqBusy) return;
    state.globalEqBusy = true;
    state.globalEqMessage = null;
    state.globalEqDirty = false;
    dom.globalEqModal.classList.remove('hidden');
    dom.globalEqContent.innerHTML =
        '<div class="plugin-loading">正在检测 Equalizer APO…</div>';
    let loadError = null;
    try {
        const allDeviceIds = [...state.outputDevices, ...state.inputDevices]
            .map((device) => device.device_id);
        const [status, enabledDeviceIds] = await Promise.all([
            AudioAPI.equalizerApoStatus(),
            AudioAPI.equalizerApoEnabledDevices(allDeviceIds),
        ]);
        state.globalEqStatus = status;
        state.equalizerApoEnabledDeviceIds = new Set(enabledDeviceIds);
        const outputDevices = equalizerApoDevices(state.outputDevices);
        const preferredId = localStorage.getItem('audio-hub-global-eq-device');
        const selected = outputDevices.find(
            (device) => device.device_id === preferredId,
        ) || outputDevices.find((device) => device.is_default)
            || outputDevices[0] || null;
        state.globalEqDeviceId = selected?.device_id || null;
        if (selected) {
            await loadGlobalEqPresetState(selected.device_id);
        } else {
            state.globalEqConfig = null;
            state.globalEqSavedConfig = null;
            state.globalEqPresets = [];
            state.globalEqPresetName = null;
        }

        const preferredMicrophoneId = localStorage.getItem(
            'audio-hub-microphone-processing-device',
        );
        const inputDevices = equalizerApoDevices(state.inputDevices);
        const selectedMicrophone = inputDevices.find(
            (device) => device.device_id === preferredMicrophoneId,
        ) || inputDevices.find((device) => !isVirtualMicrophone(device))
            || inputDevices.find((device) => device.is_default)
            || inputDevices[0] || null;
        state.microphoneDeviceId = selectedMicrophone?.device_id || null;
        if (selectedMicrophone) {
            await loadMicrophoneProcessingState(selectedMicrophone.device_id);
        } else {
            state.microphoneConfig = null;
            state.microphoneSavedConfig = null;
            state.microphoneDirty = false;
            state.microphoneConfigured = false;
        }
    } catch (err) {
        loadError = err;
    } finally {
        state.globalEqBusy = false;
        renderGlobalEqEntry();
        if (loadError !== null) {
            dom.globalEqContent.innerHTML =
                `<p class="plugin-error">插件状态读取失败：${esc(String(loadError))}</p>`;
        } else {
            renderGlobalEqEditor();
        }
    }
}

function closeGlobalEqEditor() {
    if (!confirmDiscardGlobalEqChanges()) return;
    dom.globalEqModal.classList.add('hidden');
    state.globalEqConfig = null;
    state.globalEqSavedConfig = null;
    state.globalEqDirty = false;
    state.microphoneConfig = null;
    state.microphoneSavedConfig = null;
    state.microphoneDirty = false;
    state.microphoneConfigured = false;
}

async function loadGlobalEqDevice(deviceId) {
    const device = equalizerApoDevices(state.outputDevices)
        .find((item) => item.device_id === deviceId);
    if (!device) return;
    state.globalEqBusy = true;
    renderGlobalEqEditor();
    try {
        await loadGlobalEqPresetState(deviceId);
        state.globalEqDeviceId = deviceId;
        localStorage.setItem('audio-hub-global-eq-device', deviceId);
    } catch (err) {
        setStatus(`读取全局 EQ 失败: ${err}`);
    } finally {
        state.globalEqBusy = false;
        renderGlobalEqEditor();
    }
}

async function loadGlobalEqPresetState(deviceId) {
    const catalog = await AudioAPI.listGlobalEqPresets(deviceId);
    const config = await AudioAPI.getGlobalEqPreset(
        deviceId,
        catalog.active_preset,
    );
    state.globalEqPresets = catalog.presets;
    state.globalEqPresetName = catalog.active_preset;
    state.globalEqConfig = config;
    state.globalEqSavedConfig = structuredClone(state.globalEqConfig);
    state.globalEqDirty = false;
}

async function loadMicrophoneProcessingDevice(deviceId) {
    const device = equalizerApoDevices(state.inputDevices)
        .find((item) => item.device_id === deviceId);
    if (!device) return;
    state.microphoneBusy = true;
    renderGlobalEqEditor();
    try {
        await loadMicrophoneProcessingState(deviceId);
        state.microphoneDeviceId = deviceId;
        localStorage.setItem('audio-hub-microphone-processing-device', deviceId);
    } catch (err) {
        setStatus(`读取麦克风处理失败: ${err}`);
    } finally {
        state.microphoneBusy = false;
        renderGlobalEqEditor();
    }
}

async function loadMicrophoneProcessingState(deviceId) {
    const result = await AudioAPI.getMicrophoneProcessing(deviceId);
    state.microphoneConfig = result.config;
    state.microphoneConfigured = result.configured;
    state.microphoneSavedConfig = result.configured
        ? structuredClone(state.microphoneConfig)
        : null;
    state.microphoneDirty = !result.configured;
    state.microphoneMessage = null;
}

function confirmDiscardGlobalEqChanges() {
    const messages = [];
    if (state.globalEqDirty) {
        messages.push(`音色「${state.globalEqPresetName}」`);
    }
    if (state.microphoneDirty) {
        messages.push('麦克风处理');
    }
    return messages.length === 0 || confirm(
        `${messages.join('和')}有未保存的参数修改，确定放弃吗？`,
    );
}

function updateGlobalEqDirty() {
    const previousDirty = state.globalEqDirty;
    const hadMessage = state.globalEqMessage !== null;
    const nextDirty = Boolean(
        state.globalEqConfig
        && state.globalEqSavedConfig
        && JSON.stringify(state.globalEqConfig) !== JSON.stringify(state.globalEqSavedConfig),
    );
    state.globalEqDirty = nextDirty;
    state.globalEqMessage = null;

    const feedback = $('#global-eq-feedback');
    if (feedback && (previousDirty !== nextDirty || hadMessage)) {
        updateGlobalEqFeedback(feedback);
    }

    const saveButton = dom.globalEqContent.querySelector('[data-global-action="save"]');
    if (saveButton) {
        saveButton.disabled = !state.globalEqDirty || state.globalEqBusy;
    }
}

function updateMicrophoneDirty() {
    const previousDirty = state.microphoneDirty;
    const hadMessage = state.microphoneMessage !== null;
    state.microphoneDirty = Boolean(state.microphoneConfig) && (
        !state.microphoneConfigured
        || JSON.stringify(state.microphoneConfig)
            !== JSON.stringify(state.microphoneSavedConfig)
    );
    state.microphoneMessage = null;

    const feedback = $('#microphone-feedback');
    if (feedback && (previousDirty !== state.microphoneDirty || hadMessage)) {
        updateMicrophoneFeedback(feedback);
    }
    const saveButton = dom.globalEqContent.querySelector(
        '[data-global-action="microphone-save"]',
    );
    if (saveButton) {
        saveButton.disabled = !state.microphoneDirty || state.microphoneBusy;
    }
}

function globalEqFeedbackState() {
    if (state.globalEqDirty) {
        return {
            variant: 'pending',
            text: `正在修改「${state.globalEqPresetName || '当前音色'}」；`
                + '调整尚未生效，保存后会立即应用。',
        };
    }
    if (state.globalEqMessage) {
        return { variant: '', text: state.globalEqMessage };
    }
    return { variant: 'neutral', text: '当前音色参数已保存' };
}

function renderGlobalEqFeedback() {
    const feedback = globalEqFeedbackState();
    const variantClass = feedback.variant ? ` ${feedback.variant}` : '';
    return `<p class="plugin-result${variantClass}">${esc(feedback.text)}</p>`;
}

function updateGlobalEqFeedback(container) {
    const result = container.querySelector('.plugin-result');
    if (!result) return;
    const feedback = globalEqFeedbackState();
    result.className = `plugin-result${feedback.variant ? ` ${feedback.variant}` : ''}`;
    result.textContent = feedback.text;
}

function microphoneFeedbackState() {
    if (state.microphoneDirty) {
        return {
            variant: 'pending',
            text: '麦克风参数尚未生效，保存后会立即应用。',
        };
    }
    if (state.microphoneMessage) {
        return { variant: '', text: state.microphoneMessage };
    }
    return { variant: 'neutral', text: '当前麦克风参数已保存' };
}

function renderMicrophoneFeedback() {
    const feedback = microphoneFeedbackState();
    const variantClass = feedback.variant ? ` ${feedback.variant}` : '';
    return `<p class="plugin-result${variantClass}">${esc(feedback.text)}</p>`;
}

function updateMicrophoneFeedback(container) {
    const result = container.querySelector('.plugin-result');
    if (!result) return;
    const feedback = microphoneFeedbackState();
    result.className = `plugin-result${feedback.variant ? ` ${feedback.variant}` : ''}`;
    result.textContent = feedback.text;
}

function renderGlobalEqEditor() {
    const status = state.globalEqStatus;
    if (!status) {
        dom.globalEqContent.innerHTML =
            '<div class="plugin-loading">正在检测 Equalizer APO…</div>';
        return;
    }

    if (!status.installed) {
        dom.globalEqContent.innerHTML = `
            <div class="plugin-state-card">
                <span class="plugin-state-dot missing"></span>
                <div>
                    <strong>未检测到 Equalizer APO</strong>
                    <p>安装官方版本后，用设备配置器勾选需要处理的输出或录制设备，再回到这里连接。</p>
                </div>
            </div>
            <p class="plugin-safety-note">
                Audio Hub 不会静默安装驱动或修改 APO 注册；下载、安装和设备启用都由你明确操作。
            </p>
            <div class="eq-actions">
                <button class="btn eq-secondary-btn" data-global-action="refresh">重新检测</button>
                <div class="eq-actions-spacer"></div>
                <button class="btn btn-sm" data-global-action="download">打开官方下载页</button>
            </div>`;
        return;
    }

    const stateLabel = status.connected
        ? '已连接'
        : '未连接';
    const stateClass = status.connected ? 'connected' : 'ready';
    const managedConfigPath = status.managed_config_path || status.config_path || '尚未生成';
    const backupState = status.backup_exists
        ? '<span class="plugin-backup-state">原配置已备份</span>'
        : '';
    const pluginStateCard = `
        <div class="plugin-state-card">
            <span class="plugin-state-dot ${stateClass}"></span>
            <div>
                <strong>${stateLabel}</strong>
                <div class="plugin-config-location"
                    title="Audio Hub 配置文件保存在 Equalizer APO 的 config 文件夹中">
                    <span>Audio Hub 配置文件</span>
                    <code>${esc(managedConfigPath)}</code>
                    ${backupState}
                </div>
            </div>
            <button class="btn eq-secondary-btn plugin-connect-btn
                ${status.connected ? 'plugin-danger-btn' : ''}"
                data-global-action="${status.connected ? 'disconnect' : 'connect'}">
                ${status.connected ? '断开 Equalizer APO' : '连接 Equalizer APO'}
            </button>
        </div>`;
    const tabBar = `
        <div class="eq-mode-tabs" role="tablist" aria-label="Equalizer APO 功能">
            <button class="eq-mode-tab ${state.equalizerApoTab === 'output' ? 'active' : ''}"
                data-global-action="tab-output" role="tab"
                aria-selected="${state.equalizerApoTab === 'output'}">输出 EQ</button>
            <button class="eq-mode-tab ${state.equalizerApoTab === 'microphone' ? 'active' : ''}"
                data-global-action="tab-microphone" role="tab"
                aria-selected="${state.equalizerApoTab === 'microphone'}">麦克风处理</button>
        </div>`;

    if (state.equalizerApoTab === 'microphone') {
        const config = state.microphoneConfig;
        const microphoneDevices = equalizerApoDevices(state.inputDevices);
        const microphoneOptions = microphoneDevices.map((device) => `
            <option value="${escAttr(device.device_id)}"
                ${device.device_id === state.microphoneDeviceId ? 'selected' : ''}>
                ${esc(device.name)}${device.is_default ? '（系统默认）' : ''}
                ${isVirtualMicrophone(device) ? ' · 虚拟设备' : ' · 物理设备'}
            </option>
        `).join('') || '<option value="">请先在设备配置器中启用录制设备</option>';
        const virtualDeviceNote = microphoneDevices.some(isVirtualMicrophone) ? `
            <p class="microphone-path-note">
                检测到已启用的虚拟麦克风。通常处理物理麦克风即可；只有物理设备驱动绕过
                APO 时，才改选最终使用的 B1/B2 虚拟麦克风。
            </p>
        ` : '';
        const editor = config ? renderMicrophoneControls(config) : (
            '<p class="plugin-error">没有在 Equalizer APO 设备配置器中启用的录制设备。</p>'
        );

        dom.globalEqContent.innerHTML = `
            ${pluginStateCard}
            ${tabBar}
            <label class="device-select-field global-device-field">
                <span>处理的麦克风</span>
                <select id="microphone-device-select" class="device-select"
                    ${state.microphoneBusy || microphoneDevices.length === 0 ? 'disabled' : ''}>
                    ${microphoneOptions}
                </select>
            </label>
            ${virtualDeviceNote}
            ${editor}
            <div id="microphone-feedback" class="global-eq-feedback"
                aria-live="polite">${renderMicrophoneFeedback()}</div>
            <div class="eq-actions">
                <button class="btn eq-secondary-btn" data-global-action="configurator"
                    title="将弹出 Windows 用户账户控制（UAC）确认"
                    ${status.configurator_path ? '' : 'disabled'}>设备配置器（管理员）</button>
                <div class="eq-actions-spacer"></div>
                <button class="btn eq-secondary-btn" data-global-action="microphone-reset"
                    ${config ? '' : 'disabled'}>恢复推荐设置</button>
                <button class="btn btn-sm" data-global-action="microphone-save"
                    ${config && state.microphoneDirty && !state.microphoneBusy ? '' : 'disabled'}>
                    保存并应用
                </button>
            </div>`;
        return;
    }

    const config = state.globalEqConfig;
    const outputDevices = equalizerApoDevices(state.outputDevices);
    const deviceOptions = outputDevices.map((device) => `
        <option value="${escAttr(device.device_id)}"
            ${device.device_id === state.globalEqDeviceId ? 'selected' : ''}>
            ${esc(device.name)}${device.is_default ? '（默认）' : ''}
        </option>
    `).join('') || '<option value="">请先在设备配置器中启用输出设备</option>';
    const editor = config ? renderGlobalEqControls(config) : (
        '<p class="plugin-error">没有在 Equalizer APO 设备配置器中启用的输出设备。</p>'
    );
    const presetOptions = state.globalEqPresets.map((name) => `
        <option value="${escAttr(name)}"
            ${name === state.globalEqPresetName ? 'selected' : ''}>${esc(name)}</option>
    `).join('');
    const presetToolbar = config ? `
        <div class="eq-preset-toolbar">
            <label>
                <span>音色预设</span>
                <select id="global-eq-preset-select" class="device-select">${presetOptions}</select>
            </label>
            <button class="btn eq-secondary-btn preset-icon-btn"
                data-global-action="preset-new" title="将当前参数另存为新音色">＋</button>
            <button class="btn eq-secondary-btn preset-icon-btn plugin-danger-btn"
                data-global-action="preset-delete"
                ${state.globalEqPresets.length <= 1 ? 'disabled' : ''}
                title="删除当前音色预设">🗑</button>
        </div>
    ` : '';
    const resultMessage = renderGlobalEqFeedback();

    dom.globalEqContent.innerHTML = `
        ${pluginStateCard}
        ${tabBar}
        <label class="device-select-field global-device-field">
            <span>应用到输出设备</span>
            <select id="global-eq-device-select" class="device-select"
                ${state.globalEqBusy || outputDevices.length === 0 ? 'disabled' : ''}>
                ${deviceOptions}
            </select>
        </label>
        ${presetToolbar}
        ${editor}
        <div id="global-eq-feedback" class="global-eq-feedback"
            aria-live="polite">${resultMessage}</div>
        <div class="eq-actions">
            <button class="btn eq-secondary-btn" data-global-action="configurator"
                title="将弹出 Windows 用户账户控制（UAC）确认"
                ${status.configurator_path ? '' : 'disabled'}>设备配置器（管理员）</button>
            <div class="eq-actions-spacer"></div>
            <button class="btn eq-secondary-btn" data-global-action="reset"
                ${config ? '' : 'disabled'}>恢复平直</button>
            <button class="btn btn-sm" data-global-action="save"
                title="仅保存当前音色的参数修改；切换预设会立即生效"
                ${config && state.globalEqDirty && !state.globalEqBusy ? '' : 'disabled'}>
                保存音色修改
            </button>
        </div>`;
}

function renderMicrophoneControls(config) {
    const controlsDisabled = !config.enabled;
    const monoPath = state.globalEqStatus?.rnnoise_mono_path;
    const stereoPath = state.globalEqStatus?.rnnoise_stereo_path;
    const pluginDirectory = state.globalEqStatus?.rnnoise_plugin_directory;
    const selectedPluginPath = config.rnnoise_mode === 'stereo'
        ? stereoPath
        : monoPath;
    const rnnoiseUnavailable = !monoPath && !stereoPath;
    const rnnoiseStatus = selectedPluginPath
        ? `<span class="rnnoise-status available" title="${escAttr(selectedPluginPath)}">
            已检测插件
        </span>`
        : '<span class="rnnoise-status missing">未检测到对应插件</span>';
    return `
        <div class="microphone-processing-editor">
            <label class="microphone-enable-row">
                <span>
                    <strong>启用麦克风处理</strong>
                    <small>只关闭当前麦克风，不影响输出 EQ</small>
                </span>
                <input id="microphone-enabled" type="checkbox"
                    ${config.enabled ? 'checked' : ''}>
            </label>
            <div class="microphone-control-row">
                <span class="eq-label-with-help">
                    麦克风增益
                    <span class="eq-help" tabindex="0" role="note"
                        aria-label="麦克风增益说明"
                        data-tooltip="在进入 Voicemeeter 或语音软件前提高麦克风音量。建议让正常说话峰值保持在 -12 至 -6 dB；过高会削波失真并放大底噪。">?</span>
                </span>
                <input id="microphone-gain" type="range" min="-12" max="18" step="0.5"
                    value="${config.gain_db}" ${controlsDisabled ? 'disabled' : ''}>
                <span id="microphone-gain-value" class="eq-value">
                    ${formatEqDb(config.gain_db)}
                </span>
            </div>
            <div class="microphone-rnnoise-row">
                <label class="microphone-rnnoise-switch">
                    <span>
                        <strong class="eq-label-with-help">
                            RNNoise 智能降噪
                            <span class="eq-help" tabindex="0" role="note"
                                aria-label="RNNoise 智能降噪说明"
                                data-tooltip="实时识别人声并抑制风扇、键盘和环境背景声。插件要求麦克风格式为 48 kHz；若出现断音，请先在 Windows 录制设备高级设置中确认采样率。">?</span>
                        </strong>
                        <small>抑制持续底噪与无人声时的键盘声</small>
                    </span>
                    <input id="microphone-rnnoise-enabled" type="checkbox"
                        ${config.rnnoise_enabled ? 'checked' : ''}
                        ${controlsDisabled || rnnoiseUnavailable ? 'disabled' : ''}>
                </label>
                <div class="rnnoise-mode-row">
                    <label for="microphone-rnnoise-mode">处理声道</label>
                    <select id="microphone-rnnoise-mode" class="device-select"
                        ${controlsDisabled || !config.rnnoise_enabled ? 'disabled' : ''}>
                        <option value="mono"
                            ${config.rnnoise_mode === 'mono' ? 'selected' : ''}
                            ${monoPath ? '' : 'disabled'}>单声道（推荐）</option>
                        <option value="stereo"
                            ${config.rnnoise_mode === 'stereo' ? 'selected' : ''}
                            ${stereoPath ? '' : 'disabled'}>立体声</option>
                    </select>
                    ${rnnoiseStatus}
                </div>
                <div class="rnnoise-plugin-location">
                    <span>插件文件夹</span>
                    <code title="${escAttr(pluginDirectory || '尚未选择')}">
                        ${esc(pluginDirectory || '尚未选择')}
                    </code>
                    <button class="btn eq-secondary-btn"
                        data-global-action="rnnoise-browse">选择文件夹</button>
                </div>
                <p class="rnnoise-format-note">
                    物理麦克风通常选择单声道，资源占用更低；只有需要保留左右声道时才选择立体声。
                    输入格式必须为 48 kHz。Audio Hub 不包含 RNNoise，请自行下载 VST2 版本并选择
                    包含 rnnoise_mono.dll / rnnoise_stereo.dll 的文件夹。
                </p>
            </div>
        </div>`;
}

function renderGlobalEqControls(config) {
    const bands = config.bands.map((band, index) => `
        <label class="graphic-eq-band">
            <span class="graphic-eq-value" data-global-band-value="${index}">
                ${Number(band.gain_db) > 0 ? '+' : ''}${Number(band.gain_db).toFixed(1)}
            </span>
            <input type="range" min="-12" max="12" step="0.5"
                value="${band.gain_db}" data-global-band="${index}"
                aria-label="${formatEqFrequency(band.frequency_hz)} 增益">
            <span class="graphic-eq-frequency">${formatEqFrequency(band.frequency_hz)}</span>
        </label>
    `).join('');
    const effectivePreamp = globalEqEffectivePreamp(config);

    return `
        <div class="global-eq-editor">
            <div class="eq-master-row">
                <span class="eq-label-with-help">
                    前级增益
                    <span class="eq-help" tabindex="0" role="note"
                        aria-label="前级增益说明"
                        data-tooltip="在 EQ 滤波前整体提高或降低音量。提升过多可能产生削波失真；开启自动防削波后，系统会根据最高频段增益自动留出安全余量。">?</span>
                </span>
                <input id="global-eq-preamp" type="range" min="-12" max="6" step="0.5"
                    value="${config.preamp_db}" aria-label="全局前级增益">
                <span id="global-eq-preamp-value" class="eq-value">${formatEqDb(config.preamp_db)}</span>
            </div>
            <div class="graphic-eq-panel">
                <div id="global-eq-curve-host">${renderGlobalEqCurve(config)}</div>
                <div class="graphic-eq-band-grid">${bands}</div>
            </div>
            <div class="eq-master-row">
                <label class="eq-switch">
                    <input id="global-auto-headroom" type="checkbox"
                        ${config.auto_headroom ? 'checked' : ''}>
                    <span class="eq-label-with-help">
                        自动防削波余量
                        <span class="eq-help" tabindex="0" role="note"
                            aria-label="自动防削波余量说明"
                            data-tooltip="当某些频段被提升时，自动降低整体增益，避免超过数字音频上限而失真。右侧数值是最终实际生效的前级增益。">?</span>
                    </span>
                </label>
                <span></span>
                <span id="global-effective-preamp" class="eq-value">${formatEqDb(effectivePreamp)}</span>
            </div>
        </div>`;
}

function globalEqCurvePoints(config) {
    const width = 520;
    const height = 116;
    const paddingX = 22;
    const paddingY = 12;
    return config.bands.map((band, index) => ({
        x: paddingX + index * ((width - paddingX * 2) / (config.bands.length - 1)),
        y: paddingY + ((12 - Number(band.gain_db)) / 24) * (height - paddingY * 2),
    }));
}

function smoothCurvePath(points) {
    if (points.length === 0) return '';
    if (points.length === 1) return `M ${points[0].x} ${points[0].y}`;
    let path = `M ${points[0].x.toFixed(1)} ${points[0].y.toFixed(1)}`;
    for (let index = 0; index < points.length - 1; index += 1) {
        const before = points[Math.max(0, index - 1)];
        const current = points[index];
        const next = points[index + 1];
        const after = points[Math.min(points.length - 1, index + 2)];
        const control1X = current.x + (next.x - before.x) / 6;
        const control1Y = current.y + (next.y - before.y) / 6;
        const control2X = next.x - (after.x - current.x) / 6;
        const control2Y = next.y - (after.y - current.y) / 6;
        path += ` C ${control1X.toFixed(1)} ${control1Y.toFixed(1)},`
            + ` ${control2X.toFixed(1)} ${control2Y.toFixed(1)},`
            + ` ${next.x.toFixed(1)} ${next.y.toFixed(1)}`;
    }
    return path;
}

function renderGlobalEqCurve(config) {
    const points = globalEqCurvePoints(config);
    const linePath = smoothCurvePath(points);
    const areaPath = `${linePath} L ${points.at(-1).x.toFixed(1)} 108`
        + ` L ${points[0].x.toFixed(1)} 108 Z`;
    const markers = points.map((point) => `
        <circle cx="${point.x.toFixed(1)}" cy="${point.y.toFixed(1)}" r="3.2"></circle>
    `).join('');
    return `
        <svg class="graphic-eq-curve" viewBox="0 0 520 116"
            role="img" aria-label="EQ 频率响应预览">
            <defs>
                <linearGradient id="eq-curve-fill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0" stop-color="#8b5cf6" stop-opacity="0.32"></stop>
                    <stop offset="1" stop-color="#8b5cf6" stop-opacity="0.02"></stop>
                </linearGradient>
            </defs>
            <g class="graphic-eq-grid">
                <line x1="22" y1="12" x2="498" y2="12"></line>
                <line x1="22" y1="58" x2="498" y2="58"></line>
                <line x1="22" y1="104" x2="498" y2="104"></line>
            </g>
            <text x="2" y="15">+12</text>
            <text x="9" y="61">0</text>
            <text x="2" y="107">−12</text>
            <path class="graphic-eq-area" d="${areaPath}"></path>
            <path class="graphic-eq-line" d="${linePath}"></path>
            <g class="graphic-eq-markers">${markers}</g>
        </svg>`;
}

function updateGlobalEqCurve(config) {
    const host = $('#global-eq-curve-host');
    if (!host) return;

    const points = globalEqCurvePoints(config);
    const linePath = smoothCurvePath(points);
    const areaPath = `${linePath} L ${points.at(-1).x.toFixed(1)} 108`
        + ` L ${points[0].x.toFixed(1)} 108 Z`;

    host.querySelector('.graphic-eq-line')?.setAttribute('d', linePath);
    host.querySelector('.graphic-eq-area')?.setAttribute('d', areaPath);
    host.querySelectorAll('.graphic-eq-markers circle').forEach((marker, index) => {
        const point = points[index];
        if (!point) return;
        marker.setAttribute('cx', point.x.toFixed(1));
        marker.setAttribute('cy', point.y.toFixed(1));
    });
}

function sessionIcon(name) {
    const n = name.toLowerCase();
    if (n.includes('chrome') || n.includes('edge') || n.includes('firefox')) return '🌐';
    if (n.includes('discord')) return '💬';
    if (n.includes('steam')) return '🎮';
    if (n.includes('spotify') || n.includes('qqmusic') || n.includes('music')) return '🎵';
    if (n.includes('system') || n.includes('系统')) return '⚙️';
    if (n.includes('voicemeeter')) return '🎛️';
    if (n.includes('nvcontainer')) return '🖥️';
    return '🔊';
}

// ── 设备列表（抽屉内）────────────────────────────────
function renderDeviceList() {
    if (state.loading) {
        dom.deviceList.innerHTML = Array(4).fill(
            '<div class="skeleton skeleton-row"></div>',
        ).join('');
        return;
    }

    const total = state.outputDevices.length + state.inputDevices.length;
    if (total === 0) {
        dom.deviceList.innerHTML =
            '<div class="empty-state">未检测到音频设备</div>';
        return;
    }

    let html = '';
    if (state.outputDevices.length > 0) {
        html += `<div class="section-divider">🔊 输出设备 (${state.outputDevices.length})</div>`;
        html += renderDrawerVolumeControl('output');
        for (const d of state.outputDevices) {
            html += renderDeviceItem(d);
        }
    }
    if (state.inputDevices.length > 0) {
        html += `<div class="section-divider">🎙️ 输入设备 (${state.inputDevices.length})</div>`;
        html += renderDrawerVolumeControl('input');
        for (const d of state.inputDevices) {
            html += renderDeviceItem(d);
        }
    }
    dom.deviceList.innerHTML = html;
}

function renderDrawerVolumeControl(kind) {
    const isOutput = kind === 'output';
    const device = isOutput ? state.defaultOutput : state.defaultInput;
    const volumeState = state.drawerVolumes[kind];
    const ready = Boolean(
        device
        && volumeState
        && volumeState.deviceId === device.device_id,
    );
    const percentage = ready
        ? Math.round(Number(volumeState.volume) * 100)
        : 0;
    const muted = ready && volumeState.muted;
    const label = isOutput ? '设备主音量' : '麦克风音量';
    const icon = isOutput ? '🔊' : '🎙️';
    const note = isOutput
        ? '影响所有使用该输出设备播放的声音'
        : 'Windows 输入级别，与 APO 麦克风增益相互独立';

    return `
        <div class="drawer-volume-card" data-device-volume-card="${kind}">
            <div class="drawer-volume-heading">
                <div>
                    <strong>${label}</strong>
                    <span title="${escAttr(device?.name || '')}">
                        ${esc(device?.name || '未检测到默认设备')}
                    </span>
                </div>
                <span class="drawer-volume-state" data-device-volume-value="${kind}">
                    ${ready ? `${percentage}%` : '读取中…'}
                </span>
            </div>
            <div class="drawer-volume-row">
                <button class="btn drawer-volume-mute ${muted ? 'muted' : ''}"
                    data-device-volume-mute="${kind}" ${ready ? '' : 'disabled'}
                    title="${muted ? '取消静音' : '静音'}">
                    ${muted ? '🔇' : icon}
                </button>
                <input class="drawer-volume-slider" type="range"
                    min="0" max="100" step="1" value="${percentage}"
                    style="--fill: ${percentage}%"
                    data-device-volume="${kind}" ${ready ? '' : 'disabled'}
                    aria-label="${label}">
            </div>
            <p>${note}</p>
        </div>`;
}

function renderDeviceItem(device) {
    const cls = device.is_default
        ? 'device-item default'
        : 'device-item clickable';
    const tag = device.is_default
        ? '<span class="device-default-tag">默认</span>'
        : '';
    return `
        <div class="${cls}"
             data-device-id="${escAttr(device.device_id)}"
             title="${device.is_default ? '当前默认设备' : '点击设为默认'}">
            <div class="device-radio"></div>
            <span class="device-name">${esc(device.name)}</span>
            ${tag}
        </div>`;
}

// ── 底部状态栏 ───────────────────────────────────────
function renderStatusbar() {
    dom.statusbarOutputName.textContent =
        state.defaultOutput?.name || '未检测到';
    dom.statusbarInputName.textContent =
        state.defaultInput?.name || '未检测到';
}

// ── 错误横幅 ─────────────────────────────────────────
function renderError() {
    if (state.error) {
        dom.errorBanner.classList.remove('hidden');
        dom.errorMsg.textContent = state.error;
    } else {
        dom.errorBanner.classList.add('hidden');
    }
}

// ── 设备抽屉 ─────────────────────────────────────────
function openDrawer() {
    state.drawerOpen = true;
    dom.deviceDrawer.classList.remove('hidden');
    dom.drawerOverlay.classList.remove('hidden');
    refreshDrawerVolumes();
}

function closeDrawer() {
    state.drawerOpen = false;
    dom.deviceDrawer.classList.add('hidden');
    dom.drawerOverlay.classList.add('hidden');
}

async function refreshDrawerVolumes() {
    await Promise.allSettled([
        refreshDrawerVolume('output'),
        refreshDrawerVolume('input'),
    ]);
}

async function refreshDrawerVolume(kind) {
    const device = kind === 'output' ? state.defaultOutput : state.defaultInput;
    const requestId = ++state.drawerVolumeRequestIds[kind];
    if (!device) {
        state.drawerVolumes[kind] = null;
        if (state.drawerOpen) renderDeviceList();
        return;
    }
    if (state.drawerVolumes[kind]?.deviceId !== device.device_id) {
        state.drawerVolumes[kind] = null;
        if (state.drawerOpen) renderDeviceList();
    }

    try {
        const volumeState = await AudioAPI.getDeviceVolume(device.device_id);
        if (requestId !== state.drawerVolumeRequestIds[kind]) return;
        const currentDevice = kind === 'output'
            ? state.defaultOutput
            : state.defaultInput;
        if (currentDevice?.device_id !== device.device_id) return;
        state.drawerVolumes[kind] = {
            deviceId: device.device_id,
            ...volumeState,
        };
        if (state.drawerOpen) renderDeviceList();
    } catch (err) {
        if (requestId !== state.drawerVolumeRequestIds[kind]) return;
        setStatus(`读取${kind === 'output' ? '设备' : '麦克风'}音量失败：${err}`);
    }
}

function updateDrawerVolumeUi(kind) {
    const volumeState = state.drawerVolumes[kind];
    if (!volumeState) return;
    const percentage = Math.round(Number(volumeState.volume) * 100);
    const slider = dom.deviceList.querySelector(
        `[data-device-volume="${kind}"]`,
    );
    const value = dom.deviceList.querySelector(
        `[data-device-volume-value="${kind}"]`,
    );
    const muteButton = dom.deviceList.querySelector(
        `[data-device-volume-mute="${kind}"]`,
    );
    if (slider) {
        slider.value = percentage;
        slider.style.setProperty('--fill', `${percentage}%`);
    }
    if (value) value.textContent = `${percentage}%`;
    if (muteButton) {
        muteButton.classList.toggle('muted', volumeState.muted);
        muteButton.textContent = volumeState.muted
            ? '🔇'
            : kind === 'output' ? '🔊' : '🎙️';
        muteButton.title = volumeState.muted ? '取消静音' : '静音';
    }
}

async function setDrawerDeviceVolume(kind, percentage) {
    const volumeState = state.drawerVolumes[kind];
    if (!volumeState) return;
    const requestId = ++state.drawerVolumeRequestIds[kind];
    volumeState.volume = percentage / 100;
    if (percentage > 0) volumeState.muted = false;
    updateDrawerVolumeUi(kind);
    try {
        const updated = await AudioAPI.setDeviceVolume(
            volumeState.deviceId,
            percentage / 100,
        );
        if (requestId !== state.drawerVolumeRequestIds[kind]) return;
        state.drawerVolumes[kind] = {
            deviceId: volumeState.deviceId,
            ...updated,
        };
        updateDrawerVolumeUi(kind);
    } catch (err) {
        if (requestId !== state.drawerVolumeRequestIds[kind]) return;
        setStatus(`设置${kind === 'output' ? '设备' : '麦克风'}音量失败：${err}`);
        refreshDrawerVolume(kind);
    }
}

async function toggleDrawerDeviceMute(kind) {
    const volumeState = state.drawerVolumes[kind];
    if (!volumeState) return;
    const requestId = ++state.drawerVolumeRequestIds[kind];
    const muted = !volumeState.muted;
    volumeState.muted = muted;
    updateDrawerVolumeUi(kind);
    try {
        const updated = await AudioAPI.setDeviceMute(volumeState.deviceId, muted);
        if (requestId !== state.drawerVolumeRequestIds[kind]) return;
        state.drawerVolumes[kind] = {
            deviceId: volumeState.deviceId,
            ...updated,
        };
        updateDrawerVolumeUi(kind);
    } catch (err) {
        if (requestId !== state.drawerVolumeRequestIds[kind]) return;
        setStatus(`设置设备静音失败：${err}`);
        refreshDrawerVolume(kind);
    }
}

// ── Windows 通知与低频兜底刷新 ───────────────────────
async function setupAudioNotifications() {
    try {
        const eventApi = window.__TAURI__.event;
        const sessionUnlisten = await eventApi.listen(
            'audio-sessions-changed',
            () => scheduleAudioRefresh({ sessions: true }),
        );
        const devicesUnlisten = await eventApi.listen(
            'audio-devices-changed',
            () => scheduleAudioRefresh({ sessions: true, devices: true }),
        );
        const unfocusedMuteUnlisten = await eventApi.listen(
            'unfocused-mute-changed',
            refreshUnfocusedMuteStatus,
        );
        state.notificationUnlisteners.push(
            sessionUnlisten,
            devicesUnlisten,
            unfocusedMuteUnlisten,
        );

        const available = await AudioAPI.audioNotificationsAvailable();
        if (available) {
            setStatus('就绪 · 系统事件监听');
        }
        return available;
    } catch (err) {
        console.warn('系统音频通知不可用，使用轮询：', err);
        return false;
    }
}

function scheduleAudioRefresh({ sessions = false, devices = false }) {
    state.pendingSessionRefresh ||= sessions;
    state.pendingDeviceRefresh ||= devices;
    if (state.notificationRefreshTimer) {
        clearTimeout(state.notificationRefreshTimer);
    }
    state.notificationRefreshTimer = setTimeout(() => {
        state.notificationRefreshTimer = null;
        const refreshRequest = {
            sessions: state.pendingSessionRefresh,
            devices: state.pendingDeviceRefresh,
        };
        state.pendingSessionRefresh = false;
        state.pendingDeviceRefresh = false;
        refreshRuntimeState(refreshRequest);
    }, 80);
}

function startAutoRefresh(intervalMs) {
    if (state.autoRefreshId) clearInterval(state.autoRefreshId);
    state.autoRefreshId = setInterval(
        () => refreshRuntimeState({ sessions: true, devices: true }),
        intervalMs,
    );
}

async function refreshRuntimeState({ sessions: refreshSessions, devices: refreshDevices }) {
    if (state.loading || state.refreshing) {
        scheduleAudioRefresh({
            sessions: refreshSessions,
            devices: refreshDevices,
        });
        return;
    }

    state.refreshing = true;
    const previousDefaultOutput = state.defaultOutput;
    const previousSessions = state.sessions;
    const routeDropdownOpen = Boolean(
        dom.sessionList.querySelector('.route-dropdown:not(.hidden)'),
    );

    try {
        const [
            sessionsResult,
            outputResult,
            inputResult,
            voicemeeterResult,
            globalEqResult,
        ] =
            await Promise.allSettled([
                refreshSessions
                    ? AudioAPI.enumerateSessions()
                    : Promise.resolve(null),
                refreshDevices
                    ? AudioAPI.enumerateDevices('Output')
                    : Promise.resolve(null),
                refreshDevices
                    ? AudioAPI.enumerateDevices('Input')
                    : Promise.resolve(null),
                refreshDevices
                    ? AudioAPI.voicemeeterStatus()
                    : Promise.resolve(null),
                refreshDevices
                    ? AudioAPI.equalizerApoStatus()
                    : Promise.resolve(null),
            ]);

        let devicesChanged = false;
        let sessionsChanged = false;
        let defaultOutputChanged = false;
        let voicemeeterReconnected = false;

        if (outputResult.status === 'fulfilled' && outputResult.value) {
            devicesChanged =
                devicesChanged ||
                deviceStateSignature(state.outputDevices) !==
                    deviceStateSignature(outputResult.value);
            const nextDefaultOutput =
                outputResult.value.find((device) => device.is_default) || null;
            defaultOutputChanged =
                (state.defaultOutput?.device_id || null) !==
                (nextDefaultOutput?.device_id || null);
            state.outputDevices = outputResult.value;
            state.defaultOutput = nextDefaultOutput;
        }

        if (inputResult.status === 'fulfilled' && inputResult.value) {
            devicesChanged =
                devicesChanged ||
                deviceStateSignature(state.inputDevices) !==
                    deviceStateSignature(inputResult.value);
            state.inputDevices = inputResult.value;
            state.defaultInput =
                inputResult.value.find((device) => device.is_default) || null;
        }

        if (voicemeeterResult.status === 'fulfilled' && voicemeeterResult.value) {
            const connectionChanged =
                Boolean(state.voicemeeterStatus?.connected) !==
                Boolean(voicemeeterResult.value.connected);
            if (connectionChanged) {
                voicemeeterReconnected = Boolean(voicemeeterResult.value.connected);
                state.voicemeeterStatus = voicemeeterResult.value;
                state.voicemeeterConfiguration =
                    structuredClone(voicemeeterResult.value.configuration);
                renderVoicemeeterEntry();
                renderSessionList();
            }
        }

        if (globalEqResult.status === 'fulfilled' && globalEqResult.value) {
            state.globalEqStatus = globalEqResult.value;
            renderGlobalEqEntry();
        }

        if (sessionsResult.status === 'fulfilled' && sessionsResult.value) {
            state.sessions = sessionsResult.value;
            sessionsChanged = true;
        }

        if (state.deviceVolumeFollowEnabled && defaultOutputChanged) {
            if (previousDefaultOutput) {
                saveDeviceVolumeSnapshot(
                    previousDefaultOutput.device_id,
                    previousDefaultOutput.name,
                    previousSessions,
                );
            }
            if (state.defaultOutput) {
                const followResult = await activateDeviceVolumeSnapshot(
                    state.defaultOutput,
                );
                sessionsChanged = true;
                if (followResult.applied > 0) {
                    setStatus(
                        `已恢复 ${followResult.applied} 个应用在 ${state.defaultOutput.name} 的音量`,
                    );
                }
            }
        } else if (state.deviceVolumeFollowEnabled && sessionsChanged) {
            saveCurrentDeviceVolumeSnapshot();
        }

        if (devicesChanged) {
            renderDeviceList();
            renderStatusbar();
            if (state.drawerOpen) refreshDrawerVolumes();
        }

        if (defaultOutputChanged || voicemeeterReconnected) {
            await syncSimpleRouteMonitorToDefault();
        }

        if (devicesChanged || (sessionsChanged && !routeDropdownOpen)) {
            renderSessionList();
        }
    } catch {
        // 事件刷新失败时由下一事件或低频轮询重试。
    } finally {
        state.refreshing = false;
    }
}

function deviceStateSignature(devices) {
    return JSON.stringify(
        devices
            .map((device) => ({
                id: device.device_id,
                name: device.name,
                default: device.is_default,
            }))
            .sort((left, right) => left.id.localeCompare(right.id)),
    );
}

// ── 路由设备持久化 ───────────────────────────────────
function initSessionDevices() {
    try {
        state.sessionDevices = JSON.parse(
            localStorage.getItem('audio-hub-devices-v2') || '{}',
        );
    } catch {
        state.sessionDevices = {};
    }
}

function saveSessionDevices() {
    localStorage.setItem(
        'audio-hub-devices-v2',
        JSON.stringify(state.sessionDevices),
    );
}

// ── 隐藏应用 ─────────────────────────────────────────
function initHiddenState() {
    try {
        const saved = JSON.parse(localStorage.getItem('audio-hub-hidden-v2') || '[]');
        state.hiddenSessions = new Set(saved);
    } catch {
        state.hiddenSessions = new Set();
    }
}

function saveHiddenState() {
    localStorage.setItem(
        'audio-hub-hidden-v2',
        JSON.stringify([...state.hiddenSessions]),
    );
}

function migrateLegacyLocalState() {
    if (localStorage.getItem('audio-hub-devices-v2') === null) {
        try {
            const legacyDevices = JSON.parse(
                localStorage.getItem('audio-hub-devices') || '{}',
            );
            for (const session of state.sessions) {
                const deviceId = legacyDevices[session.pid];
                if (deviceId) {
                    state.sessionDevices[sessionKey(session)] = deviceId;
                }
            }
            saveSessionDevices();
        } catch {
            state.sessionDevices = {};
        }
    }

    if (localStorage.getItem('audio-hub-hidden-v2') === null) {
        try {
            const legacyPids = new Set(
                JSON.parse(localStorage.getItem('audio-hub-hidden') || '[]'),
            );
            state.hiddenSessions = new Set(
                state.sessions
                    .filter((session) => legacyPids.has(session.pid))
                    .map(sessionKey),
            );
            saveHiddenState();
        } catch {
            state.hiddenSessions = new Set();
        }
    }
}

function hideSession(pid) {
    const session = state.sessions.find((item) => item.pid === pid);
    if (!session) return;
    state.hiddenSessions.add(sessionKey(session));
    saveHiddenState();
    renderSessionList();
}

function unhideSession(pid) {
    const session = state.sessions.find((item) => item.pid === pid);
    if (!session) return;
    state.hiddenSessions.delete(sessionKey(session));
    saveHiddenState();
    renderSessionList();
}

function sessionKey(session) {
    return session.display_name.trim().toLocaleLowerCase();
}

function unfocusedMuteKey(displayName) {
    return String(displayName || '')
        .trim()
        .toLocaleLowerCase()
        .replace(/\.exe$/i, '');
}

function unfocusedMuteSessionKey(session) {
    return unfocusedMuteKey(session.process_name || session.display_name);
}

function unfocusedMuteApplicationKeys() {
    return new Set(
        state.unfocusedMuteStatus.applications.map((application) => application.key),
    );
}

async function refreshUnfocusedMuteStatus() {
    try {
        state.unfocusedMuteStatus = await AudioAPI.getUnfocusedMuteStatus();
        state.unfocusedMuteMessage = null;
    } catch (err) {
        state.unfocusedMuteMessage = `读取失败：${err}`;
    }
    renderUnfocusedMuteEntry();
    if (!dom.unfocusedMuteModal.classList.contains('hidden')) {
        renderUnfocusedMuteEditor();
    }
}

function renderUnfocusedMuteEntry() {
    const count = state.unfocusedMuteStatus.applications.length;
    const paused = Boolean(state.unfocusedMuteStatus.paused);
    dom.unfocusedMuteBtn.classList.toggle('active', count > 0 && !paused);
    dom.unfocusedMuteBtn.classList.toggle('offline', count === 0 || paused);
    if (paused && count > 0) {
        dom.unfocusedMuteBtnLabel.textContent = '未聚焦静音 · 暂停';
    } else if (count > 0) {
        dom.unfocusedMuteBtnLabel.textContent = `未聚焦静音 · ${count}`;
    } else {
        dom.unfocusedMuteBtnLabel.textContent = '未聚焦静音';
    }
    dom.unfocusedMuteBtn.title = paused && count > 0
        ? '自动静音已从系统托盘暂停，点击管理应用列表'
        : '选择失去前台焦点后自动静音的应用';
}

async function openUnfocusedMuteEditor() {
    dom.unfocusedMuteModal.classList.remove('hidden');
    state.unfocusedMuteMessage = null;
    renderUnfocusedMuteEditor();
    await refreshUnfocusedMuteStatus();
}

function closeUnfocusedMuteEditor() {
    dom.unfocusedMuteModal.classList.add('hidden');
}

function renderUnfocusedMuteEditor() {
    const selected = new Map(
        state.unfocusedMuteStatus.applications.map((application) => [
            application.key,
            application,
        ]),
    );
    const current = new Map();
    for (const session of [...state.sessions].sort(compareAudioSessions)) {
        if (session.pid === 0) continue;
        const key = unfocusedMuteSessionKey(session);
        if (!current.has(key)) current.set(key, session);
    }

    const rows = [];
    for (const [key, session] of current) {
        const checked = selected.has(key);
        const autoMuted = state.unfocusedMuteStatus.auto_muted_keys.includes(key);
        rows.push(`
            <label class="focus-mute-app-row ${autoMuted ? 'auto-muted' : ''}">
                <span class="focus-mute-app-icon">${sessionIcon(session.display_name)}</span>
                <span class="focus-mute-app-copy">
                    <strong>${esc(session.display_name)}</strong>
                    <small>${autoMuted
        ? '当前未聚焦，已自动静音'
        : checked
            ? '已加入自动静音列表'
            : '未加入'}</small>
                </span>
                <span class="toggle-switch">
                    <input type="checkbox" data-unfocused-mute-app
                        data-app-key="${escAttr(key)}"
                        data-display-name="${escAttr(session.display_name)}"
                        ${checked ? 'checked' : ''}
                        ${state.unfocusedMuteBusy ? 'disabled' : ''}>
                    <span class="toggle-switch-track"></span>
                </span>
            </label>`);
        selected.delete(key);
    }

    for (const application of selected.values()) {
        rows.push(`
            <label class="focus-mute-app-row unavailable">
                <span class="focus-mute-app-icon">◌</span>
                <span class="focus-mute-app-copy">
                    <strong>${esc(application.display_name)}</strong>
                    <small>当前没有活跃音频会话，设置仍会保留</small>
                </span>
                <span class="toggle-switch">
                    <input type="checkbox" data-unfocused-mute-app
                        data-app-key="${escAttr(application.key)}"
                        data-display-name="${escAttr(application.display_name)}" checked
                        ${state.unfocusedMuteBusy ? 'disabled' : ''}>
                    <span class="toggle-switch-track"></span>
                </span>
            </label>`);
    }

    dom.unfocusedMuteContent.innerHTML = `
        ${state.unfocusedMuteStatus.paused
        ? '<p class="focus-mute-message">自动静音已从系统托盘暂停，重新启用后继续按此列表运行。</p>'
        : ''}
        ${state.unfocusedMuteMessage
        ? `<p class="focus-mute-message error">${esc(state.unfocusedMuteMessage)}</p>`
        : ''}
        <div class="focus-mute-app-list">
            ${rows.length > 0
        ? rows.join('')
        : '<div class="empty-state">没有活跃的应用音频会话</div>'}
        </div>`;
}

async function setUnfocusedMuteApplication(input) {
    const enabled = input.checked;
    state.unfocusedMuteBusy = true;
    state.unfocusedMuteMessage = null;
    renderUnfocusedMuteEditor();
    try {
        state.unfocusedMuteStatus = await AudioAPI.setUnfocusedMuteApplication(
            input.dataset.appKey,
            input.dataset.displayName,
            enabled,
        );
        setStatus(enabled
            ? `已为 ${input.dataset.displayName} 启用未聚焦自动静音`
            : `已将 ${input.dataset.displayName} 移出未聚焦静音列表`);
    } catch (err) {
        state.unfocusedMuteMessage = `修改失败：${err}`;
    } finally {
        state.unfocusedMuteBusy = false;
        renderUnfocusedMuteEntry();
        renderUnfocusedMuteEditor();
        renderSessionList();
    }
}

// ── 主题 ─────────────────────────────────────────────
function initTheme() {
    const saved = localStorage.getItem('audio-hub-theme');
    if (saved === 'light') {
        document.documentElement.classList.add('light');
        updateThemeIcon(true);
    }
}

function toggleTheme() {
    const root = document.documentElement;
    const isLight = root.classList.toggle('light');
    updateThemeIcon(isLight);
    localStorage.setItem('audio-hub-theme', isLight ? 'light' : 'dark');
}

function updateThemeIcon(isLight) {
    dom.themeIcon.innerHTML = isLight
        ? '<circle cx="12" cy="12" r="5" fill="currentColor" stroke="none"/><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/>'
        : '<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>';
}

async function loadAboutAutostart() {
    state.autostartBusy = true;
    state.autostartMessage = '正在读取启动设置…';
    renderAboutAutostart();
    try {
        state.autostartEnabled = await AudioAPI.getAutostartEnabled();
        state.autostartMessage = '';
    } catch (err) {
        state.autostartMessage = `读取失败：${err}`;
    } finally {
        state.autostartBusy = false;
        renderAboutAutostart();
    }
}

function renderAboutAutostart() {
    const checkbox = $('#about-autostart');
    const status = $('#about-autostart-status');
    if (checkbox) {
        checkbox.checked = state.autostartEnabled;
        checkbox.disabled = state.autostartBusy;
    }
    if (status) {
        status.textContent = state.autostartMessage || '';
        status.classList.toggle(
            'error',
            Boolean(state.autostartMessage?.includes('失败')),
        );
    }
}

async function setAboutAutostart(enabled) {
    state.autostartBusy = true;
    state.autostartMessage = '正在更新…';
    renderAboutAutostart();
    try {
        state.autostartEnabled = await AudioAPI.setAutostartEnabled(enabled);
        state.autostartMessage = state.autostartEnabled
            ? '已启用'
            : '已关闭';
        setStatus(`开机自启动${state.autostartMessage}`);
    } catch (err) {
        state.autostartMessage = `修改失败：${err}`;
    } finally {
        state.autostartBusy = false;
        renderAboutAutostart();
    }
}

async function loadCloseBehavior() {
    state.closeBehaviorBusy = true;
    state.closeBehaviorMessage = '正在读取设置…';
    renderAboutCloseBehavior();
    try {
        const data = await AudioAPI.getCloseBehavior();
        state.closeBehavior = data.behavior;
        state.closeBehaviorChosen = data.chosen;
        state.closeBehaviorMessage = '';
    } catch (err) {
        state.closeBehaviorMessage = `读取失败：${err}`;
    } finally {
        state.closeBehaviorBusy = false;
        renderAboutCloseBehavior();
    }
}

function renderAboutCloseBehavior() {
    const button = $('#about-close-behavior-btn');
    const status = $('#about-close-behavior-status');
    if (button) {
        const label =
            state.closeBehavior === 'minimize' ? '最小化到托盘' : '退出程序';
        button.textContent = state.closeBehaviorBusy ? '读取中…' : `${label} · 更改`;
        button.disabled = state.closeBehaviorBusy;
    }
    if (status) {
        status.textContent = state.closeBehaviorMessage || '';
        status.classList.toggle(
            'error',
            Boolean(state.closeBehaviorMessage?.includes('失败')),
        );
    }
}

function openCloseBehaviorDialog(isFirstTime) {
    state.closeBehaviorFirstChoice = isFirstTime;
    state.closeBehaviorMessage = '';
    const desc = $('#close-behavior-desc');
    if (desc) {
        desc.textContent = isFirstTime
            ? '第一次使用：点击右上角 X 时，希望 Audio Hub 怎么做？选择后可在“关于”中随时更改。'
            : '点击右上角 X 时，希望 Audio Hub 怎么做？';
    }
    $('#close-behavior-modal').classList.remove('hidden');
    renderCloseBehaviorDialog();
}

function renderCloseBehaviorDialog() {
    const minimizeBtn = $('#close-behavior-minimize-btn');
    const quitBtn = $('#close-behavior-quit-btn');
    const message = $('#close-behavior-message');
    if (minimizeBtn) {
        minimizeBtn.disabled = state.closeBehaviorBusy;
    }
    if (quitBtn) {
        quitBtn.disabled = state.closeBehaviorBusy;
    }
    if (message) {
        message.textContent = state.closeBehaviorMessage || '';
        message.classList.toggle(
            'error',
            Boolean(state.closeBehaviorMessage?.includes('失败')),
        );
    }
}

async function chooseCloseBehavior(behavior) {
    if (state.closeBehaviorBusy) {
        return;
    }
    const wasFirstTime = state.closeBehaviorFirstChoice;
    state.closeBehaviorBusy = true;
    state.closeBehaviorMessage = '正在保存…';
    renderCloseBehaviorDialog();
    try {
        const saved = await AudioAPI.setCloseBehavior(behavior);
        state.closeBehavior = saved.behavior;
        state.closeBehaviorChosen = true;
        state.closeBehaviorMessage = '';
        $('#close-behavior-modal').classList.add('hidden');
        renderAboutCloseBehavior();
        setStatus(
            `关闭按钮：${
                saved.behavior === 'minimize' ? '最小化到托盘' : '退出程序'
            }`,
        );
        if (wasFirstTime) {
            await applyCloseBehavior();
        }
    } catch (err) {
        state.closeBehaviorMessage = `保存失败：${err}`;
        renderCloseBehaviorDialog();
    } finally {
        state.closeBehaviorBusy = false;
    }
}

function cancelCloseBehaviorDialog() {
    if (state.closeBehaviorBusy) {
        return;
    }
    if (state.closeBehaviorFirstChoice) {
        // 首次未明确选择时按默认“最小化到托盘”处理，避免每次点击都询问。
        chooseCloseBehavior('minimize');
        return;
    }
    $('#close-behavior-modal').classList.add('hidden');
}

async function handleCloseClicked() {
    try {
        const data = await AudioAPI.getCloseBehavior();
        state.closeBehavior = data.behavior;
        state.closeBehaviorChosen = data.chosen;
        if (!data.chosen) {
            openCloseBehaviorDialog(true);
            return;
        }
        await applyCloseBehavior();
    } catch (err) {
        console.error('读取关闭按钮行为失败，按退出处理：', err);
        window.__TAURI__.core.invoke('win_close');
    }
}

async function applyCloseBehavior() {
    if (state.closeBehavior === 'minimize') {
        await window.__TAURI__.core.invoke('win_hide_to_tray');
    } else {
        await window.__TAURI__.core.invoke('win_close');
    }
}

// ── 降级对话框（Win11 API 受限时）─────────────────────
function showFallbackDialog() {
    const msg =
        'Windows 11 限制了程序化切换默认设备。\n是否打开 Windows 声音设置面板手动切换？';
    if (confirm(msg)) {
        AudioAPI.openSoundSettings().catch(() => {});
    }
}

// ── 事件绑定 ─────────────────────────────────────────
function setupEventListeners() {
    // About 弹窗
    $('#about-btn')?.addEventListener('click', async () => {
        $('#about-modal').classList.remove('hidden');
        await loadAboutAutostart();
        await loadCloseBehavior();
    });
    $('#about-close-btn')?.addEventListener('click', () => {
        $('#about-modal').classList.add('hidden');
    });
    $('#about-github-btn')?.addEventListener('click', async () => {
        try {
            await AudioAPI.openProjectHomepage();
        } catch (err) {
            setStatus(`无法打开 GitHub：${err}`, true);
        }
    });
    $('#about-modal')?.addEventListener('click', (e) => {
        if (e.target === $('#about-modal')) {
            $('#about-modal').classList.add('hidden');
        }
    });
    $('#about-autostart')?.addEventListener('change', (e) => {
        setAboutAutostart(e.target.checked);
    });
    $('#about-close-behavior-btn')?.addEventListener('click', () => {
        openCloseBehaviorDialog(false);
    });
    $('#close-behavior-minimize-btn')?.addEventListener('click', () => {
        chooseCloseBehavior('minimize');
    });
    $('#close-behavior-quit-btn')?.addEventListener('click', () => {
        chooseCloseBehavior('quit');
    });
    $('#close-behavior-cancel-btn')?.addEventListener('click', () => {
        cancelCloseBehaviorDialog();
    });
    $('#close-behavior-modal')?.addEventListener('click', (e) => {
        if (e.target === $('#close-behavior-modal')) {
            cancelCloseBehaviorDialog();
        }
    });

    dom.unfocusedMuteBtn.addEventListener('click', openUnfocusedMuteEditor);
    dom.unfocusedMuteCloseBtn.addEventListener('click', closeUnfocusedMuteEditor);
    dom.unfocusedMuteModal.addEventListener('click', (e) => {
        if (e.target === dom.unfocusedMuteModal) closeUnfocusedMuteEditor();
    });
    dom.unfocusedMuteContent.addEventListener('change', (e) => {
        const input = e.target.closest('[data-unfocused-mute-app]');
        if (input && !input.disabled) setUnfocusedMuteApplication(input);
    });

    dom.voicemeeterBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        state.voicemeeterModeMenuOpen = !state.voicemeeterModeMenuOpen;
        renderVoicemeeterEntry();
    });
    dom.voicemeeterModeMenu.addEventListener('click', async (e) => {
        e.stopPropagation();
        const option = e.target.closest('[data-vm-entry-action]');
        if (!option || option.disabled) return;
        await runVoicemeeterEntryAction(option.dataset.vmEntryAction);
    });
    dom.deviceVolumeFollowBtn.addEventListener('click', toggleDeviceVolumeFollow);
    dom.voicemeeterCloseBtn.addEventListener('click', closeVoicemeeterEditor);
    dom.voicemeeterModal.addEventListener('click', (e) => {
        if (e.target === dom.voicemeeterModal) closeVoicemeeterEditor();
    });
    dom.voicemeeterContent.addEventListener('input', (e) => {
        const field = e.target.dataset.vmField;
        if (!field || !state.voicemeeterConfiguration) return;
        if (e.target.type === 'range') {
            setVoicemeeterField(field, Number(e.target.value));
            const value = dom.voicemeeterContent.querySelector(
                `[data-vm-value="${field}"]`,
            );
            if (value) {
                value.textContent = e.target.dataset.vmFormat === 'strength'
                    ? `${Number(e.target.value).toFixed(1)} / 10`
                    : formatVoicemeeterGain(e.target.value);
            }
        }
    });
    dom.voicemeeterContent.addEventListener('change', async (e) => {
        if (e.target.id === 'voicemeeter-background-start') {
            localStorage.setItem(
                'audio-hub-voicemeeter-autostart',
                e.target.checked ? 'true' : 'false',
            );
            state.voicemeeterMessage = e.target.checked
                ? '已启用随 Audio Hub 后台启动'
                : '已关闭随 Audio Hub 后台启动';
            renderVoicemeeterEditor();
            return;
        }
        const field = e.target.dataset.vmField;
        if (!field || !state.voicemeeterConfiguration) return;
        if (e.target.type === 'checkbox') {
            setVoicemeeterField(field, e.target.checked);
        } else if (e.target.type === 'range') {
            setVoicemeeterField(field, Number(e.target.value));
        } else {
            setVoicemeeterField(field, e.target.value || null);
        }
        await applyVoicemeeterConfiguration();
    });
    dom.voicemeeterContent.addEventListener('click', async (e) => {
        if (e.target.matches('[data-vm-source-dismiss]')) {
            state.voicemeeterSourceManagerTarget = null;
            renderVoicemeeterEditor();
            return;
        }
        const appRoute = e.target.closest('[data-vm-app-route]');
        if (appRoute) {
            await routeVoicemeeterApplication(appRoute);
            return;
        }
        const button = e.target.closest('[data-vm-action]');
        if (!button) return;
        await runVoicemeeterAction(
            button.dataset.vmAction,
            button.dataset.vmField,
            button,
        );
    });

    dom.globalEqBtn.addEventListener('click', openGlobalEqEditor);
    dom.globalEqCloseBtn.addEventListener('click', closeGlobalEqEditor);
    dom.globalEqModal.addEventListener('click', (e) => {
        if (e.target === dom.globalEqModal) closeGlobalEqEditor();
    });
    dom.globalEqContent.addEventListener('change', async (e) => {
        if (e.target.id === 'microphone-rnnoise-mode') {
            state.microphoneConfig.rnnoise_mode = e.target.value;
            updateMicrophoneDirty();
            renderGlobalEqEditor();
            return;
        }
        if (e.target.id === 'microphone-device-select') {
            if (state.microphoneConfigured && state.microphoneDirty && !confirm(
                '当前麦克风参数尚未保存，确定放弃修改并切换设备吗？',
            )) {
                renderGlobalEqEditor();
                return;
            }
            await loadMicrophoneProcessingDevice(e.target.value);
            return;
        }
        if (e.target.id === 'global-eq-device-select') {
            if (!confirmDiscardGlobalEqChanges()) {
                renderGlobalEqEditor();
                return;
            }
            await loadGlobalEqDevice(e.target.value);
            return;
        }
        if (e.target.id === 'global-eq-preset-select') {
            const presetName = e.target.value;
            if (!state.globalEqDeviceId || !presetName) return;
            if (!confirmDiscardGlobalEqChanges()) {
                renderGlobalEqEditor();
                return;
            }
            state.globalEqBusy = true;
            renderGlobalEqEditor();
            try {
                state.globalEqConfig = await AudioAPI.activateGlobalEqPreset(
                    state.globalEqDeviceId,
                    presetName,
                );
                state.globalEqSavedConfig = structuredClone(state.globalEqConfig);
                state.globalEqPresetName = presetName;
                state.globalEqDirty = false;
                state.globalEqMessage = state.globalEqStatus.connected
                    ? `已切换并应用音色：${presetName}`
                    : `已切换音色：${presetName}；连接 Equalizer APO 后生效`;
                setStatus(state.globalEqStatus.connected
                    ? `已立即应用音色预设「${presetName}」`
                    : `已切换音色预设「${presetName}」；连接后生效`);
            } catch (err) {
                setStatus(`切换音色预设失败: ${err}`);
            } finally {
                state.globalEqBusy = false;
                renderGlobalEqEditor();
            }
            return;
        }
    });
    dom.globalEqContent.addEventListener('input', (e) => {
        const microphoneConfig = state.microphoneConfig;
        if (microphoneConfig) {
            if (e.target.id === 'microphone-enabled') {
                microphoneConfig.enabled = e.target.checked;
                updateMicrophoneDirty();
                renderGlobalEqEditor();
                return;
            }
            if (e.target.id === 'microphone-rnnoise-enabled') {
                microphoneConfig.rnnoise_enabled = e.target.checked;
                updateMicrophoneDirty();
                renderGlobalEqEditor();
                return;
            }
            if (e.target.id === 'microphone-gain') {
                microphoneConfig.gain_db = Number(e.target.value);
                updateMicrophoneDirty();
                $('#microphone-gain-value').textContent =
                    formatEqDb(microphoneConfig.gain_db);
                return;
            }
        }

        const config = state.globalEqConfig;
        if (!config) return;
        if (e.target.id === 'global-auto-headroom') {
            config.auto_headroom = e.target.checked;
            updateGlobalEqDirty();
            renderGlobalEqEditor();
            return;
        }
        if (e.target.id === 'global-eq-preamp') {
            config.preamp_db = Number(e.target.value);
            updateGlobalEqDirty();
            $('#global-eq-preamp-value').textContent =
                formatEqDb(config.preamp_db);
            $('#global-effective-preamp').textContent =
                formatEqDb(globalEqEffectivePreamp(config));
            return;
        }
        const bandIndex = Number(e.target.dataset.globalBand);
        if (Number.isInteger(bandIndex) && config.bands[bandIndex]) {
            config.bands[bandIndex].gain_db = Number(e.target.value);
            updateGlobalEqDirty();
            const value = dom.globalEqContent.querySelector(
                `[data-global-band-value="${bandIndex}"]`,
            );
            if (value) {
                const gain = Number(e.target.value);
                value.textContent = `${gain > 0 ? '+' : ''}${gain.toFixed(1)}`;
            }
            updateGlobalEqCurve(config);
            $('#global-effective-preamp').textContent =
                formatEqDb(globalEqEffectivePreamp(config));
        }
    });
    dom.globalEqContent.addEventListener('click', async (e) => {
        const button = e.target.closest('[data-global-action]');
        if (!button || button.disabled || state.globalEqBusy || state.microphoneBusy) return;
        const action = button.dataset.globalAction;
        if (action === 'tab-output') {
            if (state.equalizerApoTab === 'microphone' && state.microphoneDirty
                && !confirm('麦克风参数尚未保存，确定放弃修改并切换页面吗？')) {
                return;
            }
            if (state.microphoneDirty && state.microphoneSavedConfig) {
                state.microphoneConfig = structuredClone(state.microphoneSavedConfig);
                state.microphoneDirty = false;
            }
            state.equalizerApoTab = 'output';
            renderGlobalEqEditor();
            return;
        }
        if (action === 'tab-microphone') {
            if (state.equalizerApoTab === 'output' && state.globalEqDirty
                && !confirm('当前音色参数尚未保存，确定放弃修改并切换页面吗？')) {
                return;
            }
            if (state.globalEqDirty && state.globalEqSavedConfig) {
                state.globalEqConfig = structuredClone(state.globalEqSavedConfig);
                state.globalEqDirty = false;
            }
            state.equalizerApoTab = 'microphone';
            renderGlobalEqEditor();
            return;
        }
        if (action === 'download') {
            await AudioAPI.openEqualizerApoDownload()
                .catch((err) => setStatus(`打开下载页失败: ${err}`));
            return;
        }
        if (action === 'configurator') {
            await AudioAPI.openEqualizerApoConfigurator()
                .catch((err) => setStatus(`启动 Configurator 失败: ${err}`));
            return;
        }
        if (action === 'rnnoise-browse') {
            state.microphoneBusy = true;
            renderGlobalEqEditor();
            try {
                const updatedStatus = await AudioAPI.chooseRnnoisePluginDirectory();
                if (updatedStatus) {
                    state.globalEqStatus = updatedStatus;
                    state.microphoneMessage = '已更新 RNNoise 插件文件夹。';
                    setStatus('已更新 RNNoise 插件文件夹');
                }
            } catch (err) {
                setStatus(`选择 RNNoise 插件文件夹失败: ${err}`);
            } finally {
                state.microphoneBusy = false;
                renderGlobalEqEditor();
            }
            return;
        }
        if (action === 'refresh') {
            state.globalEqStatus = null;
            await openGlobalEqEditor();
            return;
        }
        if (action === 'reset') {
            state.globalEqConfig = defaultGlobalEqConfig();
            updateGlobalEqDirty();
            renderGlobalEqEditor();
            return;
        }
        if (action === 'microphone-reset') {
            state.microphoneConfig = defaultMicrophoneConfig();
            updateMicrophoneDirty();
            renderGlobalEqEditor();
            return;
        }
        let newPresetName = null;
        if (action === 'preset-new') {
            const entered = prompt('输入新音色名称（例如“FPS 脚步增强”）：');
            if (!entered || !entered.trim()) return;
            newPresetName = entered.trim();
            if (state.globalEqPresets.includes(newPresetName)) {
                setStatus(`音色预设「${newPresetName}」已存在`);
                return;
            }
        }
        if (action === 'preset-delete') {
            const dirtyWarning = state.globalEqDirty ? '\n未保存的参数修改也会丢失。' : '';
            if (!confirm(
                `确定删除音色预设「${state.globalEqPresetName}」吗？${dirtyWarning}`,
            )) return;
        }
        if (action === 'disconnect' && !confirm(
            '确定断开 Equalizer APO 吗？\n\n'
            + '这会停止 Audio Hub 的输出 EQ 和麦克风处理。'
            + '不会卸载 Equalizer APO，也不会删除已保存的参数。',
        )) return;

        state.globalEqBusy = true;
        state.microphoneBusy = true;
        renderGlobalEqEditor();
        try {
            if (action === 'connect') {
                state.globalEqStatus = await AudioAPI.connectEqualizerApo();
                state.globalEqMessage = '已连接 Equalizer APO，配置会在后台自动重载。';
                state.microphoneMessage = state.globalEqMessage;
                setStatus('已连接 Equalizer APO；配置将在后台自动重载');
            } else if (action === 'disconnect') {
                state.globalEqStatus = await AudioAPI.disconnectEqualizerApo();
                state.globalEqMessage = '已断开 Equalizer APO；所有设备的已保存参数仍然保留。';
                state.microphoneMessage = state.globalEqMessage;
                setStatus('已断开 Equalizer APO，Audio Hub EQ 参数仍保留');
            } else if (action === 'microphone-save') {
                const device = state.inputDevices.find(
                    (item) => item.device_id === state.microphoneDeviceId,
                );
                if (!device || !state.microphoneConfig) {
                    throw new Error('所选麦克风已断开');
                }
                state.microphoneConfig = await AudioAPI.setMicrophoneProcessing(
                    device.device_id,
                    device.name,
                    state.microphoneConfig,
                );
                state.microphoneSavedConfig = structuredClone(state.microphoneConfig);
                state.microphoneDirty = false;
                state.microphoneConfigured = true;
                state.microphoneMessage = state.globalEqStatus.connected
                    ? `已保存并应用到 ${device.name}`
                    : '麦克风参数已保存；连接 Equalizer APO 后生效。';
                localStorage.setItem(
                    'audio-hub-microphone-processing-device',
                    device.device_id,
                );
                setStatus(state.microphoneMessage);
            } else if (action === 'save' || action === 'preset-new') {
                const device = state.outputDevices.find(
                    (item) => item.device_id === state.globalEqDeviceId,
                );
                const presetName = action === 'preset-new'
                    ? newPresetName
                    : state.globalEqPresetName;
                if (!device || !state.globalEqConfig || !presetName) {
                    throw new Error('所选输出设备已断开');
                }
                state.globalEqConfig = await AudioAPI.saveGlobalEqPreset(
                    device.device_id,
                    device.name,
                    presetName,
                    state.globalEqConfig,
                );
                const catalog = await AudioAPI.listGlobalEqPresets(device.device_id);
                state.globalEqPresets = catalog.presets;
                state.globalEqPresetName = presetName;
                state.globalEqSavedConfig = structuredClone(state.globalEqConfig);
                state.globalEqDirty = false;
                state.globalEqMessage = state.globalEqStatus.connected
                    ? `已保存并应用「${presetName}」的参数修改`
                    : '参数已保存；连接 Equalizer APO 后才会生效。';
                setStatus(state.globalEqStatus.connected
                    ? `已保存「${presetName}」的修改并应用到 ${device.name}`
                    : '全局 EQ 已保存；接入 Equalizer APO 后才会生效');
            } else if (action === 'preset-delete') {
                const deletedName = state.globalEqPresetName;
                const catalog = await AudioAPI.deleteGlobalEqPreset(
                    state.globalEqDeviceId,
                    deletedName,
                );
                state.globalEqPresets = catalog.presets;
                state.globalEqPresetName = catalog.active_preset;
                state.globalEqDirty = false;
                state.globalEqConfig = await AudioAPI.getGlobalEqPreset(
                    state.globalEqDeviceId,
                    catalog.active_preset,
                );
                state.globalEqSavedConfig = structuredClone(state.globalEqConfig);
                state.globalEqMessage =
                    `已删除音色「${deletedName}」，当前切换到「${catalog.active_preset}」`;
                setStatus(state.globalEqMessage);
            }
        } catch (err) {
            setStatus(`Equalizer APO 插件操作失败: ${err}`);
        } finally {
            state.globalEqBusy = false;
            state.microphoneBusy = false;
            renderGlobalEqEntry();
            renderGlobalEqEditor();
        }
    });

    // 主题切换
    dom.themeToggleBtn.addEventListener('click', toggleTheme);

    // 窗口控制（走自定义 Tauri 命令）
    $('#minimize-btn')?.addEventListener('click', () => {
        window.__TAURI__.core.invoke('win_minimize');
    });
    $('#maximize-btn')?.addEventListener('click', () => {
        window.__TAURI__.core.invoke('win_toggle_maximize');
    });
    $('#close-btn')?.addEventListener('click', () => {
        handleCloseClicked();
    });

    // 底部默认设备点击 → 打开设备抽屉
    dom.statusbarOutput.addEventListener('click', openDrawer);
    dom.statusbarInput.addEventListener('click', openDrawer);

    // 设备抽屉关门
    dom.drawerCloseBtn.addEventListener('click', closeDrawer);
    dom.drawerOverlay.addEventListener('click', closeDrawer);
    dom.deviceList.addEventListener('input', (e) => {
        const slider = e.target.closest('[data-device-volume]');
        if (!slider) return;
        setDrawerDeviceVolume(
            slider.dataset.deviceVolume,
            Number(slider.value),
        );
    });
    dom.deviceList.addEventListener('click', (e) => {
        const muteButton = e.target.closest('[data-device-volume-mute]');
        if (!muteButton || muteButton.disabled) return;
        toggleDrawerDeviceMute(muteButton.dataset.deviceVolumeMute);
    });
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && state.drawerOpen) {
            closeDrawer();
        }
        if (e.key === 'Escape' && !dom.globalEqModal.classList.contains('hidden')) {
            closeGlobalEqEditor();
        }
        if (e.key === 'Escape' && !dom.unfocusedMuteModal.classList.contains('hidden')) {
            closeUnfocusedMuteEditor();
        }
        if (e.key === 'Escape' && !dom.voicemeeterModal.classList.contains('hidden')) {
            if (state.voicemeeterSourceManagerTarget) {
                state.voicemeeterSourceManagerTarget = null;
                renderVoicemeeterEditor();
            } else {
                closeVoicemeeterEditor();
            }
        }
    });

    // WebView2 滚动接管
    const dashboard = document.querySelector('.session-card .card-body');
    if (dashboard) {
        dashboard.addEventListener('wheel', (e) => {
            e.preventDefault();
            dashboard.scrollTop += e.deltaY;
        }, { passive: false });
    }

    // 音量滑块：事件委托
    dom.sessionList.addEventListener('input', (e) => {
        const slider = e.target.closest('.volume-slider');
        if (!slider) return;

        const pid = parseInt(slider.dataset.pid, 10);
        const volume = parseInt(slider.value, 10) / 100;
        updateLocalVolume(pid, volume);
        scheduleCurrentDeviceVolumeSnapshot();

        AudioAPI.setSessionVolume(pid, volume).catch(() => {
            loadAllData();
        });
    });

    dom.sessionList.addEventListener('click', async (e) => {
        const button = e.target.closest('[data-simple-route]');
        if (!button || button.disabled) return;
        e.stopPropagation();
        await toggleSimpleRouteApplication(button);
    });

    dom.sessionList.addEventListener('click', async (e) => {
        const button = e.target.closest('.capture-btn');
        if (!button || button.disabled || state.captureBusy) return;

        const pid = parseInt(button.dataset.pid, 10);
        const session = state.sessions.find((item) => item.pid === pid);
        state.captureBusy = true;
        renderSessionList();

        try {
            if (state.captureStatus.active && state.captureStatus.pid === pid) {
                const result = await AudioAPI.stopProcessCapture();
                state.captureStatus = await AudioAPI.processCaptureStatus();
                setStatus(
                    `录制完成：${Math.round(result.duration_ms / 1000)} 秒`,
                );
                if (confirm(`录制已保存到：\n${result.output_path}\n\n是否在文件夹中显示？`)) {
                    await AudioAPI.revealCaptureFile(result.output_path);
                }
            } else {
                state.captureStatus = await AudioAPI.startProcessCapture(pid);
                setStatus(`正在录制 ${session?.display_name || `PID ${pid}`}，再次点击红色按钮停止`);
            }
        } catch (err) {
            setStatus(`录制失败: ${err}`);
            await refreshCaptureStatus();
        } finally {
            state.captureBusy = false;
            renderSessionList();
        }
    });

    // 自定义路由下拉：点击触发按钮 → 展开/收起
    dom.sessionList.addEventListener('click', (e) => {
        const trigger = e.target.closest('.route-trigger');
        if (!trigger) return;
        e.stopPropagation();
        const wrapper = trigger.closest('.route-wrapper');
        if (!wrapper) return;
        const dropdown = wrapper.querySelector('.route-dropdown');
        // 关闭其他已展开的下拉
        dom.sessionList
            .querySelectorAll('.route-dropdown:not(.hidden)')
            .forEach((d) => d.classList.add('hidden'));
        dropdown.classList.toggle('hidden');
    });

    // 自定义路由下拉：点击选项 → 路由
    dom.sessionList.addEventListener('click', (e) => {
        const opt = e.target.closest('.route-option');
        if (!opt) return;
        e.stopPropagation();
        const pid = parseInt(opt.dataset.pid, 10);
        const deviceId = opt.dataset.deviceId;
        const stableKey = opt.dataset.sessionKey;
        AudioAPI.setAppOutputDevice(pid, deviceId)
            .then(() => {
                if (deviceId) {
                    state.sessionDevices[stableKey] = deviceId;
                } else {
                    delete state.sessionDevices[stableKey];
                }
                saveSessionDevices();
                // 更新触发按钮文字和锁图标
                const wrapper = opt.closest('.route-wrapper');
                if (wrapper) {
                    const label = wrapper.querySelector('.route-label');
                    const trigger = wrapper.querySelector('.route-trigger');
                    if (label) label.textContent = opt.textContent.trim();
                    if (trigger) {
                        if (deviceId) {
                            trigger.classList.add('locked');
                            trigger.innerHTML = trigger.innerHTML.replace(
                                /<svg.*?<\/svg>/,
                                '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/><circle cx="12" cy="16" r="1"/></svg>',
                            );
                        } else {
                            trigger.classList.remove('locked');
                            trigger.innerHTML = trigger.innerHTML.replace(
                                /<svg.*?<\/svg>/,
                                '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>',
                            );
                        }
                    }
                    wrapper.querySelector('.route-dropdown')?.classList.add('hidden');
                }
            })
            .catch((err) => setStatus(`路由失败: ${err}`));
    });

    // 点击页面任意位置关闭下拉
    document.addEventListener('click', () => {
        if (state.voicemeeterModeMenuOpen) {
            state.voicemeeterModeMenuOpen = false;
            renderVoicemeeterEntry();
        }
        dom.sessionList
            .querySelectorAll('.route-dropdown:not(.hidden)')
            .forEach((d) => d.classList.add('hidden'));
    });

    // 隐藏/取消隐藏按钮
    dom.sessionList.addEventListener('click', (e) => {
        const btn = e.target.closest('.hide-btn');
        if (!btn) return;

        const pid = parseInt(btn.dataset.pid, 10);
        const action = btn.dataset.action;

        if (action === 'hide') {
            hideSession(pid);
        } else {
            unhideSession(pid);
        }
    });

    // 隐藏徽章：切换显示/隐藏
    dom.hiddenBadge.addEventListener('click', () => {
        state.showHidden = !state.showHidden;
        renderSessionList();
    });

    // 静音按钮：事件委托
    dom.sessionList.addEventListener('click', (e) => {
        const btn = e.target.closest('.mute-btn');
        if (!btn) return;

        const pid = parseInt(btn.dataset.pid, 10);
        const newMuted = !btn.classList.contains('muted');
        updateLocalMute(pid, newMuted);
        scheduleCurrentDeviceVolumeSnapshot();

        AudioAPI.setSessionMute(pid, newMuted).catch(() => {
            loadAllData();
        });
    });

    // 设备列表点击：切换默认设备（抽屉内）
    dom.deviceList.addEventListener('click', async (e) => {
        const item = e.target.closest('.device-item');
        if (!item) return;
        if (item.classList.contains('default')) return;

        const deviceId = item.dataset.deviceId;
        if (!deviceId) return;

        // 判断是输出还是输入设备
        const isInput = deviceId.includes('{0.0.1.');
        const prevDefaultId = isInput
            ? state.defaultInput?.device_id
            : state.defaultOutput?.device_id;

        item.style.opacity = '0.5';
        item.style.pointerEvents = 'none';

        try {
            await AudioAPI.setDefaultDevice(deviceId);
            await loadAllData();
            await refreshDrawerVolumes();

            const newId = isInput
                ? state.defaultInput?.device_id
                : state.defaultOutput?.device_id;
            if (newId === prevDefaultId && prevDefaultId) {
                showFallbackDialog();
            }
        } catch (err) {
            console.error('切换默认设备失败：', err);
            item.style.opacity = '1';
            item.style.pointerEvents = 'auto';
        }
    });
}

// ── 本地状态即时更新 ─────────────────────────────────
function updateLocalVolume(pid, volume) {
    const session = state.sessions.find((s) => s.pid === pid);
    if (session) {
        session.volume = volume;
        if (session.muted && volume > 0) {
            session.muted = false;
            AudioAPI.setSessionMute(pid, false).catch(() => {});
        }
    }
    const pct = Math.round(volume * 100);
    dom.sessionList.querySelectorAll(`[data-pid="${pid}"]`).forEach((el) => {
        if (el.classList.contains('volume-slider')) {
            el.value = pct;
            el.style.setProperty('--fill', `${pct}%`);
        }
        if (el.classList.contains('volume-pct')) {
            el.textContent = `${pct}%`;
        }
        if (el.classList.contains('mute-btn')) {
            if (session?.muted) {
                el.classList.add('muted');
                el.innerHTML = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="23" y1="9" x2="17" y2="15"/><line x1="17" y1="9" x2="23" y2="15"/></svg>';
            } else {
                el.classList.remove('muted');
                el.innerHTML = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>';
            }
        }
    });
}

function updateLocalMute(pid, muted) {
    const session = state.sessions.find((s) => s.pid === pid);
    if (session) session.muted = muted;
    dom.sessionList.querySelectorAll(`[data-pid="${pid}"]`).forEach((el) => {
        if (el.classList.contains('mute-btn')) {
            if (muted) {
                el.classList.add('muted');
                el.innerHTML = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="23" y1="9" x2="17" y2="15"/><line x1="17" y1="9" x2="23" y2="15"/></svg>';
            } else {
                el.classList.remove('muted');
                el.innerHTML = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>';
            }
        }
    });
}

// ── 状态栏 ───────────────────────────────────────────
function setStatus(text) {
    dom.statusText.textContent = text;
}

// ── HTML 转义 ────────────────────────────────────────
function esc(str) {
    const div = document.createElement('div');
    div.textContent = String(str);
    return div.innerHTML;
}

function escAttr(str) {
    return esc(str).replaceAll('"', '&quot;').replaceAll("'", '&#39;');
}
