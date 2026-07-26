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
    sessionDevices: {},         // 稳定会话标识 → deviceId 映射
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
};

// ── 生命周期 ─────────────────────────────────────────
document.addEventListener('DOMContentLoaded', async () => {
    initTheme();
    initHiddenState();
    initSessionDevices();
    await loadAllData(true);
    setupEventListeners();
    const notificationsAvailable = await setupAudioNotifications();
    startAutoRefresh(notificationsAvailable ? 30000 : 3000);
});

window.addEventListener('beforeunload', () => {
    for (const unlisten of state.notificationUnlisteners) {
        unlisten();
    }
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
            <button class="hide-btn" data-pid="${session.pid}" data-action="${isHidden ? 'unhide' : 'hide'}" title="${isHidden ? '取消隐藏' : '隐藏此应用'}">${hideSvg}</button>
        </div>`;
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
        for (const d of state.outputDevices) {
            html += renderDeviceItem(d);
        }
    }
    if (state.inputDevices.length > 0) {
        html += `<div class="section-divider">🎙️ 输入设备 (${state.inputDevices.length})</div>`;
        for (const d of state.inputDevices) {
            html += renderDeviceItem(d);
        }
    }
    dom.deviceList.innerHTML = html;
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
}

function closeDrawer() {
    state.drawerOpen = false;
    dom.deviceDrawer.classList.add('hidden');
    dom.drawerOverlay.classList.add('hidden');
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
    $('#about-btn')?.addEventListener('click', () => {
        $('#about-modal').classList.remove('hidden');
    });
    $('#about-close-btn')?.addEventListener('click', () => {
        $('#about-modal').classList.add('hidden');
    });
    $('#about-modal')?.addEventListener('click', (e) => {
        if (e.target === $('#about-modal')) {
            $('#about-modal').classList.add('hidden');
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
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && state.drawerOpen) {
            closeDrawer();
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

    // Profile 新建（不允许重名）
    dom.profileNewBtn.addEventListener('click', async () => {
        const name = prompt('输入配置名称（例如"游戏模式"）：');
        if (!name || !name.trim()) return;
        if (state.profiles.includes(name.trim())) {
            setStatus(`配置「${name.trim()}」已存在`);
            return;
        }
        try {
            await AudioAPI.saveProfile(name.trim(), state.sessions);
            state.profiles = await AudioAPI.listProfiles();
            dom.profileSelect.value = name.trim();
            renderProfiles();
            setStatus(`已保存「${name.trim()}」`);
        } catch (err) {
            console.error('保存配置失败：', err);
            setStatus('保存失败');
        }
    });

    // Profile 选择即切换（记住上次选择）
    dom.profileSelect.addEventListener('change', async () => {
        const name = dom.profileSelect.value;
        if (!name) return;
        localStorage.setItem('audio-hub-profile', name);
        try {
            await AudioAPI.applyProfile(name);
            await loadAllData();
            setStatus(`已切换至「${name}」`);
        } catch (err) {
            console.error('应用配置失败：', err);
            setStatus('切换失败');
        }
    });

    // Profile 删除（仅剩一个时不可删除）
    dom.profileDeleteBtn.addEventListener('click', async () => {
        const name = dom.profileSelect.value;
        if (!name) return;
        if (state.profiles.length <= 1) {
            setStatus('最后一个配置不可删除');
            return;
        }
        if (!confirm(`确定删除配置「${name}」？`)) return;
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
            console.error('删除配置失败：', err);
            setStatus('删除失败');
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
