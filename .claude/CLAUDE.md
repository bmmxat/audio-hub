请始终使用简体中文回复。
分析过程使用中文。思考步骤使用中文。技术解释使用中文。
代码、命令、变量名保持英文。

# Audio Hub 项目上下文

## 开发运行

```bash
npx tauri dev    # 必须用这个！热重载前端，免 WebView2 缓存问题
```

不用 `cargo run` 或直接双击 exe——后者会被 WebView2 缓存旧版 JS/CSS。

## 定位
Windows 专用轻量级音频管理工具，类似 VoiceMeeter 简化版，面向普通游戏玩家。
- 不实现虚拟声卡、ASIO、复杂音频路由
- 核心：查看音频设备、枚举应用会话、调整单个应用音量、Profile 一键切换

## 技术栈
- Rust + Tauri 2（GUI）
- Windows WASAPI（windows-rs 0.62，edition 2024）
- 前端：纯 HTML/CSS/JS（零构建工具，`withGlobalTauri: true` 模式）

## 目录结构
```
audio-hub/
├── Cargo.toml                         # workspace 根
├── package.json                       # @tauri-apps/cli
├── frontend/                          # 纯静态前端
│   ├── index.html
│   ├── css/style.css                  # 暗色/浅色双主题
│   └── js/
│       ├── api.js                     # window.__TAURI__.core.invoke 封装
│       └── app.js                     # 状态管理 + UI 渲染
├── src-tauri/
│   ├── Cargo.toml                     # tauri + windows + serde
│   ├── build.rs
│   ├── tauri.conf.json                # frontendDist: ../frontend
│   ├── capabilities/default.json
│   └── src/
│       ├── main.rs                    # 薄层入口
│       ├── lib.rs                     # Tauri 命令注册
│       └── audio/
│           ├── mod.rs
│           ├── device.rs              # AudioDevice, AudioSession, DeviceDirection
│           ├── wasapi.rs              # 所有 WASAPI COM 交互
│           ├── profile.rs             # 配置文件读写（%APPDATA%/audio-hub/profiles/）
│           └── policy_config.rs       # 默认设备切换 COM 接口
└── src/                               # 旧代码（命令行版，已不参与编译）
```

## Tauri 命令清单

| 命令名 | 参数 | 功能 |
|--------|------|------|
| `get_default_device_id` | — | 默认输出设备 GUID ID |
| `get_default_device_name` | — | 默认输出设备友好名称 |
| `enumerate_devices` | `direction: "Output"\|"Input"` | 枚举设备 |
| `enumerate_sessions` | — | 枚举所有输出设备的活跃会话（ToolHelp 快照解析进程名） |
| `set_session_volume` | `pid: u32, volume: f32` | 设置应用音量 |
| `set_session_mute` | `pid: u32, muted: bool` | 设置应用静音 |
| `set_default_device` | `deviceId: string` | 切换默认设备（Win11 可能不生效） |
| `open_sound_settings` | — | 打开 Windows 声音设置（降级方案） |
| `save_profile` | `name: string, sessions: AudioSession[]` | 保存 Profile |
| `list_profiles` | — | 列出所有 Profile 名称 |
| `apply_profile` | `name: string` | 应用 Profile |
| `delete_profile` | `name: string` | 删除 Profile |

## 关键设计决策

### 会话名称解析（wasapi.rs: get_process_name）
使用 `CreateToolhelp32Snapshot` + `Process32First/Next` 遍历进程快照。
**不用 `OpenProcess`**——后者对反作弊保护进程（英雄联盟、LEDKeeper）返回 ACCESS_DENIED。

### Profile 匹配策略
应用 Profile 时先按 PID 精确匹配，PID 不存在时按 `display_name` 匹配（处理应用重启后 PID 变化）。

### 默认设备切换
`policy_config.rs` 定义 IPolicyConfig COM 接口（vtable 对齐 AudioSwitcher 源码）。
- Win11 已知限制：`SetDefaultEndpoint` 返回 S_OK 但不生效，COM 类可能被移除
- 降级方案：前端检测切换失败后弹出 `ms-settings:sound`

### 前端滚动问题
WebView2 的滚轮事件传递不可靠，CSS `overflow-y: auto` + `min-height: 0` 组合在 Tauri 中不生效。
已尝试 JS 接管 wheel 事件（`passive: false`）但仍有问题，待修复。

## COM 踩坑记录

1. **windows-rs 0.62**：`CoInitializeEx` 返回 `HRESULT` 而非 `Result<()>`，不能 `?`
2. **COM 生命周期**：`CoUninitialize()` 必须在 COM 对象 Drop 之前调用，否则 `0xc0000005 ACCESS_VIOLATION`
3. **Rust 2024**：`unsafe fn` 内的 unsafe 调用需要显式 `unsafe {}` 包裹
4. **PROPVARIANT**：需 `Win32_System_Com_StructuredStorage` + `Win32_System_Variant` feature
5. **PKEY_Device_FriendlyName**：在 `Win32_Devices_FunctionDiscovery` 中

## 前端状态
- `state` 对象：`{ defaultOutput, defaultInput, outputDevices, inputDevices, sessions, loading, error, drawerOpen, hiddenPids, showHidden, profiles }`
- 隐藏列表持久化：`localStorage` key `audio-hub-hidden`
- 主题持久化：`localStorage` key `audio-hub-theme`
- 设备抽屉：右侧滑出 380px，ESC / 点击遮罩关闭

## v0.2 新增（2026-06-25）

### 默认设备切换
- `IPolicyConfigWin7` COM 接口，vtable **12 方法**（对齐 EarTrumpet 源码）
- CLSID: `870AF99C-171D-4F9E-AF0D-E63DF40C2BC9`
- IID: `F8679F50-850A-41CF-9C72-430F290290C8`
- 关键：8 个 Unused 方法 + GetPropertyValue + SetPropertyValue + SetDefaultEndpoint + SetEndpointVisibility = 12
- 输出/输入方向自动检测（device ID 含 `{0.0.1.` 为输入）
- 不再弹出 Win11 降级弹窗

### Per-app 音频路由
- `RoGetActivationFactory("Windows.Media.Internal.AudioPolicyConfig")`
- IID: `ab3d4648-e242-459f-b02f-541c70306324` (21H2+, IInspectable-based, 27 槽 vtable)
- `SetPersistedDefaultAudioEndpoint(pid, flow, role, HSTRING)`
- HSTRING 创建用 `WindowsCreateString`，COM 接管所有权（`forget`）
- 设备 ID 格式化：`\?\SWD#MMDEVAPI#{id}#{interface GUID}`
- 空设备 ID = 传 null HSTRING = 清除单个 PID 的路由

### 无边框窗口
- `tauri.conf.json`: `decorations: false`
- 自定义最小化/最大化/关闭按钮
- Rust 端 `win_minimize` / `win_toggle_maximize` / `win_close` 命令
- 顶栏 CSS `-webkit-app-region: drag` 可拖拽

### 前端
- CSS Grid `grid-template-columns: 28px 180px 1fr 44px 34px 120px 28px` 精确对齐
- 路由按钮固定 120px 列宽，`margin: 0 auto` 居中
- 路由锁定/默认双状态图标（🔒锁 / 显示器）
- 路由下拉打开时暂停自动刷新
- About 弹窗（Juice & Claude）
