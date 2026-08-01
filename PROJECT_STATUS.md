# Audio Hub 项目状态

> 更新时间：2026-08-01
> 当前版本：`0.2.0`
> 当前分支：`master`
> 远程仓库：https://github.com/bmmxat/audio-hub

## 项目定位

Audio Hub 是面向 Windows 游戏玩家的轻量级音频管理工具，使用 Rust、Tauri 2、
Windows WASAPI 和原生 HTML/CSS/JavaScript 构建。

项目将以下能力集中在一个界面中：

- Windows 输出、输入设备查看和默认设备切换。
- 系统音效及单个应用音量、静音控制。
- 单个应用输出设备路由。
- 应用音量预设保存与快速切换。
- Equalizer APO 输出 EQ 和麦克风处理。
- VoiceMeeter 应用声音流转控制。
- 进程级音频录制验证。
- 深色/浅色主题、开机自启动和便携版构建。

基础音频管理不依赖第三方软件；EQ、麦克风处理和虚拟麦克风流转属于可选扩展。

## 已完成的稳定功能

### 基础音频管理

- 枚举输出设备、输入设备和当前活跃音频会话。
- 调整系统主音量、麦克风音量和单个应用音量。
- 控制系统、设备和应用静音。
- 使用 Windows 原生通知监听设备与音频会话变化。
- 插拔设备后自动刷新设备和会话状态。
- 在设备抽屉中切换默认输入、输出设备。
- 将单个应用固定到指定输出设备，或恢复系统默认路由。
- 隐藏不需要显示的应用会话。
- 应用列表稳定排序：系统声音置顶，其余按应用名称进行中文和数字自然排序。

### 音量预设

- 保存当前应用音量和静音状态。
- 按进程名在应用重启后重新匹配会话。
- 从应用音量卡片的次级菜单创建、切换、显式保存和删除预设。
- 调整应用音量不再自动覆盖当前预设。
- 音量预设与应用路由及 EQ 音色预设相互独立。

### Equalizer APO 集成

- 检测、连接和断开 Equalizer APO。
- 显示 Audio Hub 配置文件在 Equalizer APO 目录中的位置。
- 仅显示已在 Equalizer APO Configurator 中启用的设备。
- 为每个输出设备保存独立的十段 EQ 参数。
- 提供平滑频响曲线、前级增益和自动防削波余量。
- 支持创建、切换、覆盖和删除音色预设。
- 预设切换立即生效，参数修改后再由用户保存。
- 支持麦克风增益。
- 支持用户选择外部 RNNoise VST2 插件目录，并配置单声道或立体声处理。
- 保留并备份用户原有 Equalizer APO 配置。

### 其他能力

- 使用 Windows Process Loopback Capture 录制单个应用及其子进程的原始声音。
- 输出 `48 kHz / 双声道 / 16-bit` WAV 文件。
- 支持登录 Windows 后自动启动。
- 自定义无边框窗口、主题切换和应用图标。
- 已提供 x64 便携版 Release。
- 项目使用 MIT License 开源。

## 当前正在开发：VoiceMeeter 路由工作流

当前工作区新增了 VoiceMeeter 官方 Remote API 集成，该部分尚未提交 Git。

### 后端能力

- 动态查找并加载已安装的 `VoicemeeterRemote64.dll`。
- 自动检测 VoiceMeeter Standard、Banana 和 Potato。
- 从 Audio Hub 启动 VoiceMeeter。
- 打开 VoiceMeeter 原始界面。
- 重启 VoiceMeeter 音频引擎。
- 读取和控制主 VAIO 输入，以及 Banana/Potato 的 AUX VAIO 输入：
  - 每条虚拟输入到 A1 和对应 B1/B2 的路由。
  - 两条虚拟输入各自的增益与静音。
  - B1、B2 输出总线各自的增益与静音。
- 控制第一个硬件输入条带：
  - 选择或清除 WDM 物理麦克风。
  - 调整物理输入增益与静音。
  - 将麦克风独立发送到 A1、B1 和 B2。
- 选择 A1 WDM 本地监听设备。
- 按版本读取和控制 VoiceMeeter DSP：
  - Standard：物理输入 `Audibility` 简化增强。
  - Banana：物理输入压缩器、噪声门，以及 A1 输出六段参数均衡。
  - Potato：在 Banana 能力上增加内置降噪和物理输入六段参数均衡。
- 六段均衡读取 VoiceMeeter 现有滤波器并保留类型、频率和 Q 值；只有用户修改
  均衡时才同步各声道，普通路由更新不会重写均衡配置。
- 支持随 Audio Hub 在后台启动 VoiceMeeter。
- 新增简易流转状态管理器：
  - 读取并保存应用开始流转前的持久化输出设备。
  - 首个应用启用时自动路由到 `VoiceMeeter Input`，将默认物理麦克风送入 B1，
    并把 B1 虚拟麦克风设为 Windows 默认输入。
  - 简易流转开始时强制将 A1 绑定到当时的 Windows 默认物理扬声器；后续应用共用
    同一 A1，应用各自原先的输出设备只用于停止流转后的恢复。
  - 流转期间监听默认输出变化：切换到其他物理输出时自动同步 A1；切换到虚拟输出
    时保留原 A1 并提示，避免回路。
  - 支持多个应用同时加入；移除单个应用时恢复其原输出，移除最后一个应用时恢复
    原默认麦克风并退出 VoiceMeeter，不回滚 VoiceMeeter 内部配置。
  - 使用 `simple-route-session.json` 保存恢复信息，重新启动 Audio Hub 后仍可识别
    未结束的流转会话。

VoiceMeeter Remote API 的参数写入是异步的。当前实现采用乐观界面更新：

- API 命令成功后保留用户刚选择的目标状态。
- API 明确报错时恢复原状态。
- 用户点击“刷新状态”时重新读取 VoiceMeeter 的实际状态。

该逻辑已经解决“实际切换成功，但 Audio Hub 界面又显示旧状态”的问题。

### 双混音工作流界面

原有的单路 A1/B1 工作流已扩展为受约束的双混音工作流：

```text
活跃应用 ─→ 主混音（VoiceMeeter Input）────→ A1 / B1
        └→ AUX 混音（VoiceMeeter AUX Input）→ A1 / B2
物理麦克风 ───────────────────────────────→ A1 / B1 / B2
```

- 主混音与 AUX 卡片的应用来源框可直接点击，打开覆盖在高级模式之上的第三级
  应用选择弹层；应用列表不再插入工作流顶部，也不再保留单独的“添加/管理应用”按钮。
- 弹层支持加入、移除、滚动列表、关闭按钮、点击外部关闭和 Esc 返回。
- 两张混音卡片分别显示已分配应用、输入增益、输入静音及输出路由。
- B1、B2 各自提供输出增益和静音。
- “物理麦克风”卡片可选择真实输入设备并设置 A1、B1、B2 发送。
- “本地监听设备”由主混音、AUX 和物理麦克风共享。
- A1/B1/B2 不再作为按钮主文案：界面改用“输出到物理扬声器”和
  “输出到虚拟麦克风”，副标题显示技术标识及 A1 当前绑定的实际设备名称。
- Standard 自动隐藏 AUX 控制，并提示升级 Banana 或 Potato。
- Banana、Potato 同时显示 `VoiceMeeter Input` 和 `VoiceMeeter AUX Input`。
- 新增“可选声音处理”区域，依据 Standard、Banana、Potato 的能力自动显示控件。
- “可选声音处理”默认折叠，折叠状态仍显示已开启项目数量或双重处理警告摘要。
- 精确匹配当前 A1 扬声器和物理麦克风的 Equalizer APO 配置；两套处理同时启用时
  显示双重处理警告，但不自动修改任一配置。
- 深色、浅色主题和窄窗口响应式样式均已覆盖。
- 深色、浅色主题均已适配。

### 简易模式界面

- 原“路由工作流”入口改为统一的“声音流转”菜单，在同一按钮中切换简易和高级模式。
- 入口采用“未启用 / 简易模式 / 高级模式”三态结构；选择任一模式会启动
  VoiceMeeter，关闭声音流转会安全撤回虚拟端点后通过 Remote API 退出 VoiceMeeter。
- 简易模式下，VoiceMeeter 已安装且 Remote API 已连接时，每个非系统应用行显示
  “传到麦克风”按钮；未安装或未连接时仅隐藏这些快捷按钮，顶部统一入口始终保留。
- 已启用应用显示“流转中”，支持逐个停止；关闭最后一个应用会直接关闭整个声音流转。
- 流转开始后的状态提示及顶部菜单会显示 A1 实际绑定的本地监听扬声器名称。
- VoiceMeeter 未启动或未连接时，顶部统一入口显示“未启用”。
- 未启用时简易和高级模式均不显示选中背景或勾选，顶部按钮采用中性样式；只有
  VoiceMeeter 实际运行后才标识当前模式。
- 关闭时只恢复应用输出和 Windows 默认麦克风，不恢复 A1/B1/DSP 等 VoiceMeeter
  内部参数，使下一次打开简易或高级模式时保持上次配置。
- 简易流转生效时锁定相同应用的手动输出路由，并阻止高级工作流覆盖其配置；切换到
  高级模式前会提示并结束当前简易流转。
- 应用行网格已针对默认窗口宽度压缩，模式菜单向左展开，避免快捷按钮和菜单越界。

### 当前工作流边界

- 自定义应用只能从当前活跃音频会话中选择，应用需要先开始发声。
- 当前只管理第一个 VoiceMeeter 硬件输入条带。
- 尚未实现完全自由的拖拽节点和任意矩阵连接。
- “已路由应用”根据 Audio Hub 保存的应用路由状态判断；在其他程序中设置的持久化
  路由可能不会出现在来源卡片中。
- AUX 混音依赖 VoiceMeeter Banana 或 Potato；Standard 仅支持主混音。
- VoiceMeeter 必须由用户单独安装，Audio Hub 不捆绑或静默安装。
- VoiceMeeter 可以隐藏到系统托盘运行，但音频引擎必须保持启动。

## 本轮开发改动

与 VoiceMeeter 集成及工作流直接相关的文件：

- `src-tauri/src/plugins/voicemeeter.rs`：Remote API 动态加载、状态读取和参数控制。
- `src-tauri/src/audio/simple_route.rs`：简易流转会话、回滚和崩溃恢复状态管理。
- `src-tauri/src/plugins/mod.rs`：注册 VoiceMeeter 插件模块。
- `src-tauri/src/lib.rs`：注册 VoiceMeeter Tauri 命令和管理器。
- `frontend/js/api.js`：新增 VoiceMeeter invoke 封装。
- `frontend/js/app.js`：状态管理、工作流渲染和交互逻辑。
- `frontend/css/style.css`：工作流节点、连线、双主题和响应式样式。
- `frontend/index.html`：新增“路由工作流”入口和弹窗。
- `readme.md`：补充 VoiceMeeter 功能与使用步骤。
- `PROJECT_STATUS.md`：本状态文档。

当前仓库中还有以下本地文件，不属于本轮产品代码，提交时继续排除：

- `.claude/settings.local.json`
- `AGENTS.md`
- `README.pdf`
- `README.png`

## 验证状态

最近一次完整检查结果：

- `node --check frontend/js/app.js`：通过。
- `node --check frontend/js/api.js`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo test -p audio-hub --lib`：22 项测试全部通过。
- `cargo clippy -p audio-hub --all-targets -- -D warnings`：通过。
- `npx.cmd tauri build --debug --no-bundle`：通过（使用独立验证目标目录）。
- `npx.cmd tauri build --no-bundle`：通过（x64 Release 便携版）。
- `git diff --check`：通过，仅有 Git 的 LF/CRLF 提示。
- VoiceMeeter Remote API 实机测试：通过。
- A1 监听设备切换：通过。
- A1、B1 连接和断开状态同步：通过。
- 输入、虚拟麦克风增益及静音：通过。
- 工作流深色、浅色主题视觉检查：通过。
- `1020 × 900` 默认窗口尺寸检查：通过。
- 新增双混音、应用管理、物理麦克风、版本化 DSP 与简易流转仍需在真实设备上完成
  最终音频回归。

当前开发版使用以下命令运行：

```powershell
npx tauri dev
```

不要使用 `cargo run` 或直接运行调试 EXE，避免 WebView2 加载旧的前端缓存。

## Git 状态

- 当前分支：`master`
- 最近提交：`3525b5b 完善 README 截图与许可证署名 (#2)`
- VoiceMeeter Remote API、简易/高级流转、三级应用菜单和本状态文档已纳入本地提交。
- 本轮代码及 Release 便携版尚未推送远程仓库，计划先进行一段时间实机试用。

## 建议的下一步

1. 在真实 VoiceMeeter 环境中完整测试工作流：
   - 把不同游戏或播放器分别分配到主混音与 AUX 混音。
   - 添加、切换并移除一个物理麦克风。
   - 切换实际播放声音的物理输出设备（A1）。
   - 分别连接、断开 A1、B1 和 B2。
   - 检查两条虚拟输入、物理输入和 B1/B2 输出的增益及静音。
   - 分别验证 Standard 可听度、Banana 压缩器/噪声门/A1 EQ，以及 Potato
     降噪/物理麦克风 EQ 的版本门控与实际听感。
   - 在同一设备启用 Equalizer APO，确认双重处理提示出现且两侧配置均未被改动。
   - 点击刷新后确认界面与 VoiceMeeter 状态一致。
2. 根据双混音实测反馈调整三栏卡片尺寸、术语和操作顺序。
3. 使用本地 x64 便携版进行一段时间实机试用，记录路由恢复、设备切换和异常退出问题。
4. 试用稳定后推送本地提交，并更新 GitHub Release。
5. 后续可继续实现：
   - 拖拽式来源和输出连接。
   - “游戏语音”“直播”“只监听”等路由模板。
   - Banana、Potato 更多输入条和输出总线。
   - 工作流配置导入、导出和设备缺失恢复。
