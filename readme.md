# Audio Hub

> 面向 Windows 游戏玩家的轻量级音频控制中心。

> 由 **Juice** 发起，并借助 **OpenAI Codex** 与 **Claude Code** 完成设计、开发与迭代。

Audio Hub 使用 Rust、Tauri 2 和 Windows WASAPI 构建，将应用音量、设备切换、
应用输出路由、按扬声器记忆音量、未聚焦自动静音、Equalizer APO 调音以及
可选的 VoiceMeeter 流转控制集中在一个界面中。
基础音频管理无需安装任何虚拟声卡或第三方插件。

当前版本：`0.3.1`

## 主要功能

- **应用音量管理**：查看活跃音频会话，独立调整音量、静音和输出设备。
- **设备快速切换**：管理默认扬声器、耳机和麦克风，并控制设备主音量。
- **智能音量状态**：按不同扬声器记忆应用音量，支持应用未聚焦时自动静音。
- **VoiceMeeter 流转**：提供面向普通用户的简易模式，以及双混音、物理麦克风和
  DSP 控制的高级模式。
- **Equalizer APO 调音**：按设备保存十段输出 EQ，并支持麦克风增益和 RNNoise。
- **应用声音录制**：使用 Windows Process Loopback 录制指定应用及其子进程。
- **本地与轻量**：无账号、无云端同步、无遥测；基础功能不依赖虚拟声卡或插件。

## v0.3.1 更新

- **改进 VoiceMeeter 简易模式待命逻辑**：进入待命后即可切换到虚拟麦克风，方便在
  启动游戏或语音软件前完成输入设备准备；首次使用会提示相关行为及应用内输入设备
  可能需要手动切换。
- **新增单实例运行保护**：重复启动时恢复并聚焦已有 Audio Hub 窗口，避免多个实例
  同时监听设备并写入音量状态。
- **修复音量随扬声器**：由 Windows 默认输出设备变化直接触发保存和恢复，后台或
  托盘状态下仍然生效；应用会话按实际输出端点隔离，不再跨设备误改音量。

## v0.3.0 更新

以下内容以远端 `origin/master`（`3525b5b`）为基线整理：

- **新增 VoiceMeeter 完整工作流**：通过官方 Remote API 自动检测 Standard、Banana
  和 Potato，通过API控制VoiceMeeter行为，支持**简易流转模式**、**高级路由模式**，支持启动、关闭、打开原界面及重启音频引擎；Audio Hub 不捆绑或静默
  安装 VoiceMeeter。
- **新增简易流转模式**：应用列表可直接将一个或多个程序“传到麦克风”，自动配置
  `VoiceMeeter Input`、B1 虚拟麦克风、物理麦克风混入和 A1 本地监听；停止时恢复
  应用输出及默认麦克风，异常退出后也能依据恢复记录继续安全收尾。
- **新增高级路由模式**：提供主混音、AUX 混音、物理麦克风、A1/B1/B2 路由、增益
  和静音控制，并按 VoiceMeeter 版本开放压缩器、噪声门、降噪、可听度和六段均衡；
  同一设备同时启用 Equalizer APO 时显示双重处理提示。
- **新增未聚焦静音**：可选择应用在失去 Windows 前台焦点后自动静音，重新聚焦、
  移出列表或退出 Audio Hub 时恢复原状态，并提供异常退出恢复保护。
- **新增音量随扬声器**：按默认输出设备分别记忆应用音量和手动静音，切换扬声器时
  自动保存与恢复；未聚焦产生的临时静音不会污染设备快照。旧“音量预设”入口及
  自动套用逻辑已移除，但旧预设文件仍会保留。
- **新增精简系统托盘与关闭行为**：可快速打开主界面、暂停或恢复未聚焦静音，以及
  切换默认输出设备和麦克风；关闭按钮可选择隐藏到托盘或退出，最小化按钮固定进入
  任务栏，并修复托盘菜单刷新导致的设备切换失效问题。
- **统一主界面状态入口**：未启用时使用中性样式，实际生效后使用强调色。VoiceMeeter
  根据状态显示“VoiceMeeter”“简易模式”或“高级模式”，Equalizer APO 启用后
  同步显示强调状态。

## 界面预览

### 应用音量与快捷状态

集中调节系统与应用音量，并从顶部查看和控制未聚焦静音、VoiceMeeter、
Equalizer APO 及音量随扬声器。功能启用后使用紫色强调状态。

![Audio Hub 应用音量与快捷控制](docs/images/audio-hub-main.png)

### 应用输出路由与音频设备

#### 单个应用输出设备

为单个应用选择独立的输出设备，或随时恢复使用系统默认设备。

![Audio Hub 应用输出路由](docs/images/audio-hub-app-routing.png)

#### 默认设备与设备音量

切换默认输出设备和麦克风，并直接调节当前默认设备的主音量。

![Audio Hub 音频设备与设备音量](docs/images/audio-hub-device-volume.png)

### 未聚焦自动静音

从当前正在发声的应用中选择管理对象；应用失去 Windows 前台焦点后自动静音，
重新聚焦后恢复声音。

![Audio Hub 未聚焦自动静音应用列表](docs/images/audio-hub-unfocused-mute.png)

### VoiceMeeter 简易流转

在主界面直接把应用声音传到麦克风，并清楚显示当前流转状态和监听设备。

![Audio Hub VoiceMeeter 简易流转](docs/images/audio-hub-voicemeeter-simple.png)

### VoiceMeeter 高级路由

分别管理物理麦克风、主混音与 AUX 混音的应用来源、增益、监听和虚拟麦克风输出。

![Audio Hub VoiceMeeter 高级路由工作流](docs/images/audio-hub-voicemeeter-advanced.png)

主混音与 AUX 混音的应用来源使用独立选择弹层，点击即可加入或移除应用。

![Audio Hub VoiceMeeter 应用来源选择](docs/images/audio-hub-voicemeeter-source-picker.png)

### Equalizer APO 音频处理

#### 输出 EQ

为不同输出设备保存独立的十段 EQ 音色预设，并提供前级增益和自动防削波余量。

![Audio Hub Equalizer APO 输出 EQ](docs/images/audio-hub-output-eq.png)

#### 麦克风处理与 RNNoise

为麦克风设置输入增益，并调用用户提供的 RNNoise VST2 插件进行智能降噪。

![Audio Hub 麦克风处理与 RNNoise](docs/images/audio-hub-microphone-processing.png)

## 功能详解

### 应用音量管理

- 枚举所有活跃输出设备上的 Windows 音频会话。
- 独立调节每个应用的音量和静音状态。
- 通过进程快照识别应用名称，兼容无法直接读取进程信息的受保护程序。
- 支持隐藏不常用的会话，并在应用重启后保持隐藏状态。
- 设备、会话和音量变化优先使用 Windows 系统事件通知刷新。
- 系统事件不可用时自动降级为低频轮询。
- 可选择应用加入“未聚焦静音”列表：应用失去 Windows 前台焦点时自动静音，
  重新聚焦后恢复自动静音前的状态。
- 未聚焦静音按真实进程名匹配，应用重启后仍然有效；移出列表、退出程序或异常恢复时
  会尽力还原原有静音状态。

### 音频设备与应用路由

- 查看所有活跃输出设备和输入设备。
- 切换 Windows 默认输出设备和默认麦克风。
- 在设备抽屉中调节当前默认输出设备主音量和麦克风输入音量。
- 支持设备级静音；拖动到非零音量时自动解除静音。
- 为单个应用指定输出设备，或恢复使用系统默认设备。
- 应用输出路由按稳定应用名称保存，应用重启后仍可恢复。
- 插拔耳机、音箱和麦克风时自动刷新设备列表。

### 应用音量随扬声器

- 顶部“音量随扬声器”按钮控制功能启停，启用后显示强调色。
- 按 Windows 默认输出设备分别记住应用音量和手动静音状态。
- 切换扬声器时先保存旧设备的当前状态，再恢复新设备上次使用的状态。
- 首次使用某个扬声器时，以当前应用音量建立该设备的初始快照。
- 后续在 Audio Hub 中调整应用音量或静音，会自动更新当前扬声器的快照。
- 使用稳定的进程名匹配应用，因此应用重启、PID 变化后仍可恢复。
- “未聚焦静音”产生的临时静音不会写入扬声器快照。

原“音量预设”前端入口和启动时自动套用逻辑已经移除。旧版本创建的预设文件仍会
保留在应用数据目录中，升级不会主动删除用户数据。

### 应用声音录制

- 使用 Windows Process Loopback 捕获指定应用及其子进程的声音。
- 同一时间只录制一个应用。
- 输出 `48 kHz / 双声道 / 16-bit` WAV 文件。
- 录制完成后可直接在资源管理器中定位文件。
- 需要 Windows Build `20348` 或更高版本；低版本系统会自动禁用录制按钮。

### 系统托盘与关闭行为

- 系统托盘菜单可打开 Audio Hub，以及切换默认输出设备和默认麦克风。
- 已配置未聚焦静音应用时，托盘会显示运行状态，并可一键暂停或恢复；暂停会立即
  恢复由该功能自动静音的应用，重新启动 Audio Hub 后默认恢复运行。
- 左键单击托盘图标可恢复并聚焦主窗口。
- 主窗口的最小化按钮始终最小化到任务栏，不会隐藏到托盘。
- 第一次点击主窗口右上角关闭按钮时，可选择“最小化到托盘”或“退出程序”。
- 关闭按钮行为保存在本地，并可随时从“关于”中更改。

### VoiceMeeter 音频流转（可选）

Audio Hub 通过 [VoiceMeeter](https://vb-audio.com/Voicemeeter/) 官方 Remote API
控制常用流转参数，不捆绑或静默安装 VoiceMeeter。VoiceMeeter 由 VB-Audio
以 donationware 方式提供。

- 自动检测 VoiceMeeter Standard、Banana 和 Potato。
- 可从 Audio Hub 启动 VoiceMeeter 音频引擎并打开其原始界面。
- 顶部入口在未启用时显示“VoiceMeeter”并采用中性样式；启动后显示“简易模式”或
  “高级模式”并使用强调色。VoiceMeeter 未安装或未连接时，主界面不会显示单个
  应用的快捷流转按钮，但统一入口仍保留安装与启动引导。
- 简易模式可直接在应用列表中点击“传到麦克风”，自动将一个或多个应用送入
  `VoiceMeeter Input`，同时把当前物理麦克风混入 B1 虚拟麦克风，并将 B1 设为
  Windows 默认麦克风。首个应用启动流转时，A1 固定为当时的 Windows 默认物理
  扬声器，后续应用共用该监听设备；流转期间切换 Windows 默认物理输出时，A1 会
  自动同步。应用原输出仅用于停止时分别恢复。
- 如果新的默认输出是 VoiceMeeter 等虚拟设备，简易模式会保留原 A1 并提示用户，
  避免形成音频回路。VoiceMeeter 未运行时，顶部入口显示“未启用”。
- 顶部入口采用“未启用 / 简易模式 / 高级模式”三态结构。关闭声音流转时先恢复
  应用输出和 Windows 默认麦克风，再退出 VoiceMeeter；VoiceMeeter 内部配置不回滚，
  下次打开任一模式时继续沿用。
- 简易模式会把恢复信息保存在本地；Audio Hub 意外退出后，再次启动仍可识别并停止
  上次流转。
- 点击主混音或 AUX 混音卡片中的应用来源框，打开第三级弹出菜单，选择要分配到
  `VoiceMeeter Input` 或 `VoiceMeeter AUX Input` 的活跃应用；列表不再占用工作流正文。
- 分别控制主混音（VAIO/B1）和 AUX 混音（AUX/B2）的增益、静音与输出。
- 选择一个物理麦克风作为硬件输入，并独立发送到 A1、B1 或 B2。
- 选择实际播放声音的扬声器或耳机（VoiceMeeter A1），以“连接/断开”方式控制路由。
- 路由按钮使用“输出到物理扬声器”“输出到虚拟麦克风”等用途说明，并在副标题中
  保留 A1/B1/B2 技术标识和实际设备名称。
- VoiceMeeter Standard 提供主混音；Banana 和 Potato 同时提供 AUX 混音。
- 按安装版本开放可选 DSP：Standard 提供简化“可听度”；Banana 提供压缩器、
  噪声门和 A1 六段均衡；Potato 另提供内置降噪与物理麦克风六段均衡。
- 可选 DSP 默认折叠，通过状态摘要旁的“展开”按钮显示详细控件。
- VoiceMeeter 六段均衡保留其现有滤波器频率、类型和 Q 值，只在 Audio Hub 中
  开关频段并调整增益，避免覆盖原界面的高级设置。
- 当同一扬声器或麦克风同时启用 Equalizer APO 与 VoiceMeeter DSP 时显示
  双重处理提示，但不会自动关闭用户的任何配置。
- 可随 Audio Hub 自动启动 VoiceMeeter。
- 可在 Audio Hub 中重启 VoiceMeeter 音频引擎。

VoiceMeeter 必须保持运行，但可在其菜单中启用 `System Tray` 并关闭
`Show App On Startup`，使主窗口不在启动时显示。

### Equalizer APO 输出 EQ（可选）

Audio Hub 不捆绑或静默安装 Equalizer APO。未安装时，其余功能仍可正常使用。

- 自动检测 Equalizer APO 安装位置和连接状态。
- 可打开官方下载页和需要 UAC 确认的设备配置器。
- 输出、输入选择框只显示已在 Equalizer APO 设备配置器中启用的端点。
- 使用独立的 `audio-hub.txt` 管理配置。
- 首次连接时备份原始 `config.txt`。
- 断开连接只停止 Audio Hub 的处理，不卸载 Equalizer APO，也不删除已保存参数。
- Audio Hub 关闭后，已保存的处理仍由 Equalizer APO 在后台生效。

输出 EQ 功能：

- 按输出设备分别保存参数。
- 10 段图形均衡器：
  `31.5 / 63 / 125 / 250 / 500 / 1k / 2k / 4k / 8k / 16k Hz`。
- 实时频率响应曲线。
- 前级增益和自动防削波余量。
- 一键恢复平直。
- 每个输出设备可创建、切换和删除多个音色预设。
- 切换预设立即生效；只有修改预设参数后才需要保存。
- 切换页面、设备或预设时保护未保存的修改。

### 麦克风处理（可选）

- 按输入设备分别保存处理参数。
- 调节麦克风增益，范围为 `-12 dB` 至 `+18 dB`。
- 可单独关闭某个麦克风的处理，不影响输出 EQ。
- 支持 RNNoise VST2 智能降噪。
- 支持单声道 `rnnoise_mono.dll` 和立体声 `rnnoise_stereo.dll`。
- 可在界面中选择 RNNoise DLL 所在文件夹。
- 自动标识物理麦克风和常见虚拟麦克风。
- 只有检测到已启用 APO 的虚拟麦克风时才显示相关使用提示。

RNNoise 需要用户自行下载，Audio Hub 不分发插件文件。使用 RNNoise 时，请将
Windows 麦克风格式设置为 `48 kHz`。

### 其他

- 深色、浅色双主题。
- 无边框窗口和自定义窗口控制。
- 顶部功能入口统一使用中性未启用样式；未聚焦静音、VoiceMeeter、Equalizer APO
  和音量随扬声器启用后使用强调色显示状态。
- 可在“关于”中启用或关闭当前 Windows 用户的登录自启动。
- 可在“关于”中修改关闭按钮行为。
- 本地运行，无账号、云端同步或遥测。

## 系统要求

| 功能 | 要求 |
|---|---|
| 基础音量和设备管理 | Windows 10 或 Windows 11 |
| 界面运行 | Microsoft WebView2 Runtime，Windows 10/11 通常已预装 |
| 应用声音录制 | Windows Build 20348 或更高 |
| 应用声音流转到虚拟麦克风 | 用户自行安装 VoiceMeeter |
| 输出 EQ / 麦克风增益 | 用户自行安装 Equalizer APO |
| RNNoise 智能降噪 | Equalizer APO、48 kHz 输入格式及用户提供的 VST2 DLL |

## 快速开始

### 基础音频管理

1. 安装并启动 Audio Hub。
2. 启动需要管理的游戏、浏览器或音乐播放器。
3. 在主页面调整应用音量、静音或输出设备。
4. 点击底部输出或输入设备，打开设备抽屉。
5. 在抽屉中切换默认设备，并调整当前默认设备或麦克风音量。
6. 如果希望不同扬声器使用不同的应用音量，开启顶部“音量随扬声器”。
7. 如果希望某个应用退到后台后自动静音，打开“未聚焦静音”并将它加入列表。
8. 需要后台常驻时，在第一次点击关闭按钮时选择“最小化到托盘”。

### 使用 VoiceMeeter 流转应用声音

1. 从 VB-Audio 官方网站安装 VoiceMeeter，并按安装程序要求重启 Windows。
2. 点击顶部“VoiceMeeter”，选择“简易模式”；如 VoiceMeeter 尚未运行，可在这里启动。
3. 让目标应用开始发声，然后在应用行末点击“传到麦克风”。Audio Hub 会自动完成
   应用路由、物理麦克风混入和默认麦克风切换；可以同时启用多个应用。顶部菜单及
   状态栏会显示当前实际使用的本地监听扬声器。
4. 再次点击“流转中”可移除单个应用；关闭最后一个应用或点击菜单中的
   “关闭声音流转并退出 VoiceMeeter”，会恢复应用输出和默认麦克风并退出
   VoiceMeeter，但保留其内部混音配置供下次打开继续使用。
5. 如需自行控制主混音、AUX、A1/B1/B2 或声音处理，在同一入口选择“高级模式”。
6. 高级模式中点击主混音或 AUX 混音卡片的应用来源框，在弹出的第三级菜单中将
   正在发声的应用加入或移出混音；点击菜单外部、关闭按钮或按 Esc 均可返回。
7. 如需加入人声，在“物理麦克风”中选择输入设备，并决定发送到 A1、B1 或 B2。
8. 为主混音和 AUX 混音分别调整输入增益、静音及 A1/B1/B2 路由。
9. 在“本地监听设备”中选择实际使用的耳机或扬声器。
10. 按需在“可选声音处理”中调整麦克风增强或 A1 均衡；如出现双重处理提示，
   建议同一种效果只在 VoiceMeeter 或 Equalizer APO 其中一处启用。
11. 在游戏语音、Discord 或录音软件中选择 `VoiceMeeter Out B1` 使用主混音；
   选择 `VoiceMeeter Out B2`（旧版名称为 `VoiceMeeter AUX Output`）使用 AUX 混音。

如果希望静默启动，请先在 VoiceMeeter 菜单中启用 `System Tray`，关闭
`Show App On Startup`，再在 Audio Hub 中启用随程序启动。

### 接入 Equalizer APO

1. 安装 Equalizer APO。
2. 在 Equalizer APO 设备配置器中勾选需要处理的输出设备或麦克风。
3. 根据安装程序提示重启 Windows 或音频设备。
4. 打开 Audio Hub 的“Equalizer APO”页面。
5. 确认设备列表只包含已启用 APO 的端点。
6. 点击“连接 Equalizer APO”。
7. 在“输出 EQ”或“麦克风处理”页面保存并应用参数。

连接后，界面会显示 Audio Hub 配置文件的实际保存位置和原配置备份状态。

### 使用 RNNoise

1. 准备 `rnnoise_mono.dll` 或 `rnnoise_stereo.dll`。
2. 将麦克风的 Windows 输入格式设置为 `48 kHz`。
3. 进入“Equalizer APO → 麦克风处理”。
4. 点击“选择文件夹”，选择直接包含 DLL 的目录。
5. 选择可用的处理声道，启用 RNNoise 后保存并应用。

## 配置与安全

- 旧版音量预设仍保存在 `%APPDATA%\audio-hub\profiles\`，当前界面不再自动套用，
  也不会在升级时删除这些文件。
- 未聚焦静音列表和关闭按钮行为保存在 `%APPDATA%\audio-hub\settings.json`。
- 隐藏会话、主题、应用路由和各扬声器的应用音量快照保存在 WebView 本地数据中。
- Equalizer APO 参数保存在 Audio Hub 应用数据目录，并渲染到
  Equalizer APO 的 `audio-hub.txt`。
- Audio Hub 只在 Equalizer APO 主配置中维护一段带明确标记的
  `Include: audio-hub.txt`。
- 第一次连接时创建 `config.audio-hub.backup.txt`。
- 自启动使用
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，
  只影响当前 Windows 用户，不需要管理员权限。

如果准备卸载 Audio Hub，建议先在插件页面点击“断开 Equalizer APO”。

## 当前功能边界

- Audio Hub 自身不提供虚拟声卡、ASIO 或复杂音频矩阵。
- 应用声音流转依赖用户单独安装并运行 VoiceMeeter。
- 自定义应用列表来自当前活跃的 Windows 音频会话；应用需要先开始发声。
- 当前只管理第一个 VoiceMeeter 硬件输入条带，AUX 混音需要 Banana 或 Potato。
- 不提供单应用实时 EQ；Equalizer APO EQ 作用于启用的输出端点，VoiceMeeter
  A1 EQ 作用于整条 A1 混音路径。
- 不捆绑 VoiceMeeter、Equalizer APO 或 RNNoise。
- 当前正式打包目标为 NSIS 安装包；免安装版本需要作为独立发布物构建。

## 开发

### 环境

- Windows 10/11
- Rust stable
- Node.js 与 npm
- Microsoft WebView2 Runtime

### 安装依赖

```powershell
npm install
```

### 开发运行

```powershell
npx tauri dev
```

请使用 `npx tauri dev` 启动开发版，以避免直接运行调试 EXE 时读取旧的 WebView2
前端缓存。

### 检查

```powershell
node --check frontend/js/app.js
node --check frontend/js/api.js
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

### 构建安装包

```powershell
npx tauri build
```

### 构建便携版

```powershell
npx tauri build --no-bundle
```

该命令只生成 Release 可执行文件，不生成安装程序。便携版仍需要系统已安装
Microsoft WebView2 Runtime；VoiceMeeter、Equalizer APO 等可选组件不会被捆绑。

## 技术栈

- Rust 2024
- Tauri 2
- `windows-rs` / WASAPI / Windows Core Audio
- 原生 HTML、CSS 和 JavaScript
- Equalizer APO 文本配置集成
- VoiceMeeter Remote API 动态集成

## 项目结构

```text
audio-hub/
├─ frontend/                    # HTML、CSS、JavaScript 主界面
├─ src-tauri/src/audio/         # WASAPI、通知、旧版预设和进程录制
├─ src-tauri/src/plugins/       # Equalizer APO 与 VoiceMeeter 集成
├─ src-tauri/src/autostart.rs   # Windows 当前用户自启动
├─ src-tauri/src/settings.rs    # 关闭行为与未聚焦静音列表
├─ src-tauri/src/tray.rs        # 系统托盘、设备菜单与快捷状态控制
├─ src-tauri/src/unfocused_mute.rs # 前台进程监听和自动静音恢复
├─ archive/                     # 已归档、未参与编译的实验模块
└─ src-tauri/tauri.conf.json    # Tauri 窗口和 NSIS 打包配置
```

## 开源协议

Audio Hub 使用 [MIT License](LICENSE) 开源。

VoiceMeeter、Equalizer APO、RNNoise 插件及其他第三方组件保留各自的许可证和版权；
Audio Hub 不捆绑分发这些可选组件。
