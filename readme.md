# Audio Hub 项目开发上下文

## 项目目标

开发一个 Windows 专用的轻量级音频管理工具 Audio Hub。

定位：

* 类似 VoiceMeeter 的简化版
* 面向普通游戏玩家
* 不实现虚拟声卡
* 不实现 ASIO
* 不实现复杂音频路由
* 重点实现：

  * 查看音频设备
  * 查看默认设备
  * 枚举应用音频会话
  * 调整单个应用音量
  * 音量配置文件(Profile)
  * 快捷键切换

未来技术栈：

* Rust
* cpal
* windows-rs
* Tauri（后续 GUI）

---

# 当前环境

## Cargo.toml

```toml
[package]
name = "audio-hub"
version = "0.1.0"
edition = "2024"

[dependencies]
cpal = "0.16"

windows = { version = "0.62.2", features = [
    "Win32_Media_Audio",
    "Win32_System_Com",
    "Win32_UI_Shell_PropertiesSystem",
    "Win32_Media_KernelStreaming"
] }
```

---

# 当前目录结构

```text
src/
├── main.rs
└── audio/
    ├── mod.rs
    └── device.rs
```

---

# 当前代码

## src/audio/mod.rs

```rust
pub mod device;
```

---

## src/audio/device.rs

```rust
use cpal::traits::{DeviceTrait, HostTrait};

pub struct AudioDevice {
    pub name: String,
}

pub fn get_output_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();

    let mut devices = Vec::new();

    for device in host.output_devices().unwrap() {
        if let Ok(name) = device.name() {
            devices.push(AudioDevice { name });
        }
    }

    devices
}
```

---

## src/main.rs

```rust
mod audio;

fn main() {
    let devices = audio::device::get_output_devices();

    for device in devices {
        println!("{}", device.name);
    }
}
```

---

# 已完成功能

## CPAL

成功实现：

* 获取 Host
* 枚举所有输出设备
* 获取默认输出设备

示例：

```rust
let host = cpal::default_host();

let default_device =
    host.default_output_device();
```

---

# Windows Core Audio (WASAPI)

已验证：

```rust
CoInitializeEx(None, COINIT_MULTITHREADED);
```

成功执行。

返回：

```text
HRESULT(0x00000000)
```

即：

```text
S_OK
```

---

# 重要发现

## windows-rs 0.62.2

当前环境下：

```rust
CoInitializeEx(...)
```

返回：

```rust
HRESULT
```

不是：

```rust
Result<()>
```

因此不能：

```rust
CoInitializeEx(...)?
```

也不能：

```rust
CoInitializeEx(...).expect(...)
```

正确写法：

```rust
let _ = CoInitializeEx(
    None,
    COINIT_MULTITHREADED,
);
```

---

# 已验证 WASAPI

成功获取：

```rust
IMMDeviceEnumerator
IMMDevice
```

示例：

```rust
let enumerator: IMMDeviceEnumerator =
    CoCreateInstance(
        &MMDeviceEnumerator,
        None,
        CLSCTX_ALL,
    )?;

let device =
    enumerator.GetDefaultAudioEndpoint(
        eRender,
        eConsole,
    )?;
```

运行成功：

```text
Got default audio device!
```

---

# 已踩坑

## COM 生命周期问题

错误示例：

```rust
let enumerator = ...;
let device = ...;

CoUninitialize();
```

程序退出：

```text
0xc0000005
STATUS_ACCESS_VIOLATION
```

原因：

```text
CoUninitialize()
先执行

device Drop
enumerator Drop
后执行
```

COM 已经被卸载。

对象 Drop 时访问 COM 导致崩溃。

---

## 正确做法

```rust
{
    let enumerator = ...;
    let device = ...;
}

CoUninitialize();
```

或者开发阶段直接：

```rust
不调用 CoUninitialize()
```

---

# Rust 学习成果

## struct

```rust
pub struct AudioDevice {
    pub name: String,
}
```

---

## Vec

```rust
let mut devices = Vec::new();
```

---

## push

```rust
devices.push(
    AudioDevice {
        name
    }
);
```

---

## Option

已学习：

```rust
Option<T>

Some(...)
None
```

例如：

```rust
host.default_output_device()
```

返回：

```rust
Option<Device>
```

---

## Result

已学习：

```rust
Result<T, E>

Ok(...)
Err(...)
```

---

## if let

```rust
if let Ok(name) = device.name() {
    ...
}
```

---

## 生命周期与 Drop

实验代码：

```rust
struct Test;

impl Drop for Test {
    fn drop(&mut self) {
        println!("Drop");
    }
}
```

理解内容：

* 离开作用域自动 Drop
* 后创建先销毁
* RAII
* COM 对象必须在 CoUninitialize 前销毁

---

# 下一阶段开发计划

## Phase 1

扩展 AudioDevice

```rust
pub struct AudioDevice {
    pub name: String,
    pub is_default: bool,
}
```

实现：

```rust
get_output_devices()
```

返回：

```text
* 耳机 (KZ Acoustics M2)
  Voicemeeter Input
  Voicemeeter AUX Input
```

---

## Phase 2

WASAPI

实现：

```text
获取设备 Friendly Name
```

使用：

```text
IMMDevice
↓
OpenPropertyStore
↓
IPropertyStore
↓
PKEY_Device_FriendlyName
```

目标：

```text
耳机 (KZ Acoustics M2)
```

而不是设备 ID。

---

## Phase 3

枚举所有音频设备

获取：

* 名称
* 默认状态
* 设备 ID

---

## Phase 4

Session 管理

使用：

```text
IAudioSessionManager2
```

枚举：

```text
Chrome
Discord
Steam
Spotify
CS2
```

获取：

* PID
* 应用名称
* 音量
* 静音状态

设计模型：

```rust
pub struct AudioSession {
    pub process_name: String,
    pub volume: f32,
    pub muted: bool,
}
```

---

## Phase 5

控制应用音量

使用：

```text
ISimpleAudioVolume
```

实现：

```text
设置音量
静音
取消静音
```

---

## Phase 6

Profile

```rust
pub struct Profile {
    pub name: String,
}
```

保存：

```text
游戏模式
会议模式
音乐模式
```

---

# 重要约束

* 仅 Windows
* 不考虑 Linux
* 不考虑 macOS
* 不实现虚拟音频驱动
* 不实现 ASIO
* 不实现 OBS 音频路由
* 优先实现 Windows 音量混合器功能
* 优先实现应用级音量控制
