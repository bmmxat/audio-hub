mod audio;

use audio::{
    device::{AudioDevice, AudioSession, DeviceDirection},
    wasapi,
};

fn main() {
    println!("=== Audio Hub ===\n");

    // Phase 1 & 2：默认设备
    match wasapi::get_default_device_id() {
        Ok(id) => println!("默认设备 ID：\n  {}\n", id),
        Err(e) => eprintln!("获取设备 ID 失败：{:?}\n", e),
    }

    match wasapi::get_default_device_friendly_name() {
        Ok(name) => println!("默认设备名称：\n  {}\n", name),
        Err(e) => eprintln!("获取友好名称失败：{:?}\n", e),
    }

    // Phase 3：枚举所有设备
    println!("═════════════════════════════════════");
    print_devices("输出设备", wasapi::enumerate_devices(DeviceDirection::Output));
    print_devices("输入设备", wasapi::enumerate_devices(DeviceDirection::Input));

    // Phase 4：枚举音频会话
    println!("═════════════════════════════════════");
    print_sessions(wasapi::enumerate_sessions());
}

fn print_devices(title: &str, result: Result<Vec<AudioDevice>, windows::core::Error>) {
    println!("\n{}：", title);
    match result {
        Ok(devices) => {
            if devices.is_empty() {
                println!("  (无)");
            } else {
                for d in &devices {
                    let marker = if d.is_default { " ★ 默认" } else { "" };
                    println!("  • {}{}", d.name, marker);
                }
                println!("  ── 共 {} 个设备 ──\n", devices.len());
                for d in &devices {
                    println!("    名称：{}", d.name);
                    println!("    默认：{}", if d.is_default { "是" } else { "否" });
                    println!("    ID  ：{}\n", d.device_id);
                }
            }
        }
        Err(e) => eprintln!("  枚举失败：{:?}", e),
    }
}

fn print_sessions(result: Result<Vec<AudioSession>, windows::core::Error>) {
    println!("\n音频会话（所有输出设备，已去重）：");
    match result {
        Ok(sessions) => {
            if sessions.is_empty() {
                println!("  (无活跃会话)");
            } else {
                for s in &sessions {
                    let mute = if s.muted { " 🔇" } else { "" };
                    let bar = volume_bar(s.volume);
                    println!(
                        "  {:<40} PID {:>6}  {} {:3.0}%{}",
                        truncate(&s.display_name, 38),
                        s.pid,
                        bar,
                        s.volume * 100.0,
                        mute,
                    );
                }
                println!("  ── 共 {} 个会话 ──\n", sessions.len());
                for s in &sessions {
                    println!("    名称：{}", s.display_name);
                    println!("    PID ：{}", s.pid);
                    println!("    音量：{:.1}%", s.volume * 100.0);
                    println!("    静音：{}\n", if s.muted { "是" } else { "否" });
                }
            }
        }
        Err(e) => eprintln!("  枚举失败：{:?}", e),
    }
}

/// 生成简易音量条（10 格）。
fn volume_bar(volume: f32) -> String {
    let filled = (volume * 10.0).round() as usize;
    let empty = 10 - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

/// 截断过长的字符串（按字符边界安全截断）。
fn truncate(s: &str, max: usize) -> &str {
    if s.chars().count() > max {
        let end = s
            .char_indices()
            .nth(max)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        &s[..end]
    } else {
        s
    }
}
