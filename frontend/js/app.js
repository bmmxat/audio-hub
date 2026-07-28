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
    profiles: [],
    autoRefreshId: null,
    refreshing: false,
    notificationRefreshTimer: null,
    notificationUnlisteners: [],
    pendingSessionRefresh: false,
    pendingDeviceRefresh: false,
    autoSaveTimer: null,
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
    profileSelect: $('#profile-select'),
    profileNewBtn: $('#profile-new-btn'),
    profileDeleteBtn: $('#profile-delete-btn'),
    statusText: $('#status-text'),
    statusbarOutput: $('#statusbar-output'),
    statusbarInput: $('#statusbar-input'),
    statusbarOutputName: $('#statusbar-output-name'),
    statusbarInputName: $('#statusbar-input-name'),
    globalEqBtn: $('#global-eq-btn'),
    globalEqModal: $('#global-eq-modal'),
    globalEqCloseBtn: $('#global-eq-close-btn'),
    globalEqContent: $('#global-eq-content'),
};

// ── 生命周期 ─────────────────────────────────────────
document.addEventListener('DOMContentLoaded', async () => {
    initTheme();
    initHiddenState();
    initSessionDevices();
    await loadAllData(true);
    await refreshCaptureStatus();
    setupEventListeners();
    const notificationsAvailable = await setupAudioNotifications();
    startAutoRefresh(notificationsAvailable ? 30000 : 3000);
    startCaptureStatusPolling();
});

window.addEventListener('beforeunload', () => {
    for (const unlisten of state.notificationUnlisteners) {
        unlisten();
    }
    if (state.captureStatusTimer) clearInterval(state.captureStatusTimer);
});

// ── 数据加载 ─────────────────────────────────────────
async function loadAllData(restoreSelectedProfile = false) {
    state.loading = true;
    state.error = null;
    setStatus('加载中…');

    try {
        const [outputDevices, inputDevices, sessions, profiles] =
            await Promise.all([
                AudioAPI.enumerateDevices('Output'),
                AudioAPI.enumerateDevices('Input'),
                AudioAPI.enumerateSessions(),
                AudioAPI.listProfiles(),
            ]);

        state.outputDevices = outputDevices;
        state.inputDevices = inputDevices;
        state.sessions = sessions;
        state.profiles = profiles;
        migrateLegacyLocalState();
        await ensureDefaultProfile();
        renderProfiles();
        // 仅在启动时应用上次选中的配置，普通刷新不覆盖用户刚做的调整。
        const selectedProfile = dom.profileSelect.value;
        if (restoreSelectedProfile && selectedProfile) {
            try {
                await AudioAPI.applyProfile(selectedProfile);
                const updated = await AudioAPI.enumerateSessions();
                state.sessions = updated;
            } catch { /* 静默跳过 */ }
        }
        state.defaultOutput = outputDevices.find((d) => d.is_default) || null;
        state.defaultInput = inputDevices.find((d) => d.is_default) || null;
        state.error = null;

        setStatus('就绪');
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
    renderDeviceList();
    renderProfiles();
    renderStatusbar();
    renderError();
}

// ── 会话列表（主体）──────────────────────────────────
function renderSessionList() {
    if (state.loading) {
        dom.sessionList.innerHTML = Array(5).fill(
            '<div class="skeleton skeleton-row"></div>',
        ).join('');
        dom.sessionCount.textContent = '—';
        dom.hiddenBadge.classList.add('hidden');
        return;
    }

    // 分离可见和隐藏
    const visible = state.sessions.filter((s) => !state.hiddenSessions.has(sessionKey(s)));
    const hidden = state.sessions.filter((s) => state.hiddenSessions.has(sessionKey(s)));

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
        <div class="session-item${hiddenCls}" data-pid="${session.pid}">
            <span class="session-icon">${icon}</span>
            <span class="session-name" title="${escAttr(session.display_name)}">${esc(session.display_name)}</span>
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
            ${session.pid === 0 ? '<span class="route-label-fixed">系统默认</span>' : `
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

// ── Profile 渲染 ─────────────────────────────────────
function renderProfiles() {
    const sel = dom.profileSelect;
    // 优先用 localStorage 记住的上次配置，否则选第一个
    const lastProfile = localStorage.getItem('audio-hub-profile');
    const selected = sel.value || lastProfile || state.profiles[0] || '';
    if (state.profiles.length === 0) {
        sel.innerHTML = '<option value="">(无配置)</option>';
    } else {
        sel.innerHTML = state.profiles
            .map(
                (name) =>
                    `<option value="${escAttr(name)}"${name === selected ? ' selected' : ''}>${esc(name)}</option>`,
            )
            .join('');
    }
    sel.value = state.profiles.includes(selected) ? selected : state.profiles[0] || '';
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

// ── 自动保存 ─────────────────────────────────────────
function triggerAutoSave() {
    const name = dom.profileSelect.value;
    if (!name) return;

    const snapshot = state.sessions.map((s) => ({ ...s }));

    if (state.autoSaveTimer) clearTimeout(state.autoSaveTimer);
    state.autoSaveTimer = setTimeout(async () => {
        try {
            await AudioAPI.saveProfile(name, snapshot);
            setStatus(`已保存至「${name}」`);
        } catch {
            // 静默失败，不影响用户体验
        }
    }, 500);
}

// 确保"默认配置"始终存在
async function ensureDefaultProfile() {
    // 仅在没有任何配置时创建初始默认配置
    if (state.profiles.length === 0) {
        const DEFAULT = '默认配置';
        try {
            await AudioAPI.saveProfile(DEFAULT, state.sessions);
            state.profiles = await AudioAPI.listProfiles();
        } catch {
            // 静默失败
        }
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
        state.notificationUnlisteners.push(sessionUnlisten, devicesUnlisten);

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
    const routeDropdownOpen = Boolean(
        dom.sessionList.querySelector('.route-dropdown:not(.hidden)'),
    );

    try {
        const [sessionsResult, outputResult, inputResult] =
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
            ]);

        let devicesChanged = false;
        let sessionsChanged = false;

        if (outputResult.status === 'fulfilled' && outputResult.value) {
            devicesChanged =
                devicesChanged ||
                deviceStateSignature(state.outputDevices) !==
                    deviceStateSignature(outputResult.value);
            state.outputDevices = outputResult.value;
            state.defaultOutput =
                outputResult.value.find((device) => device.is_default) || null;
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

        if (sessionsResult.status === 'fulfilled' && sessionsResult.value) {
            let nextSessions = sessionsResult.value;
            const knownPids = new Set(state.sessions.map((session) => session.pid));
            const hasNewSession = nextSessions.some(
                (session) => !knownPids.has(session.pid),
            );

            if (hasNewSession) {
                const selectedProfile = dom.profileSelect.value;
                if (selectedProfile) {
                    try {
                        await AudioAPI.applyProfile(selectedProfile);
                        nextSessions = await AudioAPI.enumerateSessions();
                    } catch {
                        // 使用本轮结果；后续事件或兜底刷新会再次同步。
                    }
                }
            }

            state.sessions = nextSessions;
            sessionsChanged = true;
        }

        if (devicesChanged) {
            renderDeviceList();
            renderStatusbar();
            if (state.drawerOpen) refreshDrawerVolumes();
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
    });
    $('#about-close-btn')?.addEventListener('click', () => {
        $('#about-modal').classList.add('hidden');
    });
    $('#about-modal')?.addEventListener('click', (e) => {
        if (e.target === $('#about-modal')) {
            $('#about-modal').classList.add('hidden');
        }
    });
    $('#about-autostart')?.addEventListener('change', (e) => {
        setAboutAutostart(e.target.checked);
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
        window.__TAURI__.core.invoke('win_close');
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

        AudioAPI.setSessionVolume(pid, volume).catch(() => {
            loadAllData();
        });
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

    // 场景新建（不允许重名）
    dom.profileNewBtn.addEventListener('click', async () => {
        const name = prompt('输入场景名称（例如“FPS”或“电影”）：');
        if (!name || !name.trim()) return;
        if (state.profiles.includes(name.trim())) {
            setStatus(`场景「${name.trim()}」已存在`);
            return;
        }
        try {
            await AudioAPI.saveProfile(name.trim(), state.sessions);
            state.profiles = await AudioAPI.listProfiles();
            localStorage.setItem('audio-hub-profile', name.trim());
            renderProfiles();
            dom.profileSelect.value = name.trim();
            setStatus(`已创建场景「${name.trim()}」`);
        } catch (err) {
            console.error('创建场景失败：', err);
            setStatus('创建场景失败');
        }
    });

    // 场景选择即切换（记住上次选择）
    dom.profileSelect.addEventListener('change', async () => {
        const name = dom.profileSelect.value;
        if (!name) return;
        localStorage.setItem('audio-hub-profile', name);
        try {
            await AudioAPI.applyProfile(name);
            await loadAllData();
            setStatus(`已切换至场景「${name}」`);
        } catch (err) {
            console.error('应用场景失败：', err);
            setStatus('场景切换失败');
        }
    });

    // 场景删除（仅剩一个时不可删除）
    dom.profileDeleteBtn.addEventListener('click', async () => {
        const name = dom.profileSelect.value;
        if (!name) return;
        if (state.profiles.length <= 1) {
            setStatus('最后一个场景不可删除');
            return;
        }
        if (!confirm(`确定删除场景「${name}」？`)) return;
        try {
            await AudioAPI.deleteProfile(name);
            state.profiles = await AudioAPI.listProfiles();
            const nextProfile = state.profiles[0] || '';
            localStorage.setItem('audio-hub-profile', nextProfile);
            renderProfiles();
            if (nextProfile) {
                dom.profileSelect.value = nextProfile;
                await AudioAPI.applyProfile(nextProfile);
                await loadAllData();
            }
            setStatus(`已删除「${name}」`);
        } catch (err) {
            console.error('删除场景失败：', err);
            setStatus('删除场景失败');
        }
    });

    // 静音按钮：事件委托
    dom.sessionList.addEventListener('click', (e) => {
        const btn = e.target.closest('.mute-btn');
        if (!btn) return;

        const pid = parseInt(btn.dataset.pid, 10);
        const newMuted = !btn.classList.contains('muted');
        updateLocalMute(pid, newMuted);

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
        triggerAutoSave();
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
    triggerAutoSave();
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
