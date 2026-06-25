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
    hiddenPids: new Set(),
    showHidden: false,
    profiles: [],
    autoRefreshId: null,        // 定时刷新 ID
    autoSaveTimer: null,        // 防抖定时器
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
    await loadAllData();
    setupEventListeners();
});

// ── 数据加载 ─────────────────────────────────────────
async function loadAllData() {
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
        await ensureDefaultProfile();
        // 启动时自动应用当前选中配置，恢复上次保存的音量
        const cp = dom.profileSelect.value;
        if (cp) {
            try {
                await AudioAPI.applyProfile(cp);
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
    const visible = state.sessions.filter((s) => !state.hiddenPids.has(s.pid));
    const hidden = state.sessions.filter((s) => state.hiddenPids.has(s.pid));

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
    const muteIcon = session.muted ? '🔇' : '🔊';
    const icon = sessionIcon(session.display_name);
    const hiddenCls = isHidden ? ' hidden-item' : '';

    return `
        <div class="session-item${hiddenCls}" data-pid="${session.pid}">
            <span class="session-icon">${icon}</span>
            <span class="session-name" title="${esc(session.display_name)}">${esc(session.display_name)}</span>
            <div class="volume-slider-wrapper">
                <input type="range"
                       class="volume-slider"
                       min="0" max="100"
                       value="${volPct}"
                       style="--fill: ${volPct}%"
                       data-pid="${session.pid}">
            </div>
            <span class="volume-pct" data-pid="${session.pid}">${volPct}%</span>
            <button class="${muteCls}" data-pid="${session.pid}">${muteIcon}</button>
            <span class="session-pid">PID ${session.pid}</span>
            <button class="hide-btn" data-pid="${session.pid}" data-action="${isHidden ? 'unhide' : 'hide'}" title="${isHidden ? '取消隐藏' : '隐藏此应用'}">${isHidden ? '↩' : '×'}</button>
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
             data-device-id="${esc(device.device_id)}"
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
    const selected = sel.value;
    if (state.profiles.length === 0) {
        sel.innerHTML = '<option value="">(无配置)</option>';
    } else {
        sel.innerHTML = state.profiles
            .map(
                (name) =>
                    `<option value="${esc(name)}"${name === selected ? ' selected' : ''}>${esc(name)}</option>`,
            )
            .join('');
    }
    sel.value = selected || state.profiles[0] || '';
}

// ── 设备抽屉 ─────────────────────────────────────────
function openDrawer() {
    state.drawerOpen = true;
    dom.deviceDrawer.classList.remove('hidden');
    dom.drawerOverlay.classList.remove('hidden');
    dom.deviceToggleBtn.classList.add('active');
}

function closeDrawer() {
    state.drawerOpen = false;
    dom.deviceDrawer.classList.add('hidden');
    dom.drawerOverlay.classList.add('hidden');
    dom.deviceToggleBtn.classList.remove('active');
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

// ── 自动刷新 ─────────────────────────────────────────
function startAutoRefresh() {
    state.autoRefreshId = setInterval(async () => {
        if (state.loading) return;
        try {
            const sessions = await AudioAPI.enumerateSessions();

            // 检测新出现的进程，应用已保存的音量
            const knownPids = new Set(state.sessions.map((s) => s.pid));
            const newPids = sessions
                .filter((s) => !knownPids.has(s.pid))
                .map((s) => s.pid);

            if (newPids.length > 0) {
                const cp = dom.profileSelect.value;
                if (cp) {
                    // 只对新 PID 应用当前配置
                    await AudioAPI.applyProfile(cp);
                    // 重新读取以获取应用后的实际音量
                    const updated = await AudioAPI.enumerateSessions();
                    state.sessions = updated;
                } else {
                    state.sessions = sessions;
                }
            } else {
                state.sessions = sessions;
            }

            renderSessionList();
        } catch {
            // 静默失败，下次重试
        }
    }, 3000);
}

// ── 隐藏应用 ─────────────────────────────────────────
function initHiddenState() {
    try {
        const saved = JSON.parse(localStorage.getItem('audio-hub-hidden') || '[]');
        state.hiddenPids = new Set(saved);
    } catch {
        state.hiddenPids = new Set();
    }
}

function saveHiddenState() {
    localStorage.setItem(
        'audio-hub-hidden',
        JSON.stringify([...state.hiddenPids]),
    );
}

function hideSession(pid) {
    state.hiddenPids.add(pid);
    saveHiddenState();
    renderSessionList();
}

function unhideSession(pid) {
    state.hiddenPids.delete(pid);
    saveHiddenState();
    renderSessionList();
}

// ── 主题 ─────────────────────────────────────────────
function initTheme() {
    const saved = localStorage.getItem('audio-hub-theme');
    if (saved === 'light') {
        document.documentElement.classList.add('light');
        dom.themeIcon.textContent = '☀️';
    }
}

function toggleTheme() {
    const root = document.documentElement;
    const isLight = root.classList.toggle('light');
    dom.themeIcon.textContent = isLight ? '☀️' : '🌙';
    localStorage.setItem('audio-hub-theme', isLight ? 'light' : 'dark');
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
    // 主题切换
    dom.themeToggleBtn.addEventListener('click', toggleTheme);

    // 自动刷新会话列表（每 3 秒）
    startAutoRefresh();

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

    // Profile 选择即切换
    dom.profileSelect.addEventListener('change', async () => {
        const name = dom.profileSelect.value;
        if (!name) return;
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
            // 删除后自动选中第一个配置
            if (dom.profileSelect.value !== name) {
                // 删的不是当前选中，保持原样
            }
            renderProfiles();
            // 删的是当前选中，切换至第一个可用配置
            if (dom.profileSelect.value === '') {
                const first = state.profiles[0];
                if (first) {
                    dom.profileSelect.value = first;
                    await AudioAPI.applyProfile(first);
                    await loadAllData();
                }
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

        // 记住切换前的默认设备 ID
        const prevDefaultId = state.defaultOutput?.device_id;

        item.style.opacity = '0.5';
        item.style.pointerEvents = 'none';

        try {
            await AudioAPI.setDefaultDevice(deviceId);
            await loadAllData();

            // 验证是否真的切换了
            if (state.defaultOutput?.device_id === prevDefaultId) {
                // 没变化——Win11 锁死了 API
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
            if (volume === 0) {
                el.classList.add('muted');
                el.textContent = '🔇';
            } else {
                el.classList.remove('muted');
                el.textContent = '🔊';
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
                el.textContent = '🔇';
            } else {
                el.classList.remove('muted');
                el.textContent = '🔊';
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
    div.textContent = str;
    return div.innerHTML;
}
