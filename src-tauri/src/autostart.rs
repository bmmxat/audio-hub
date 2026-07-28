use std::{
    ffi::{OsStr, c_void},
    os::windows::ffi::OsStrExt,
};

use windows::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    },
    core::PCWSTR,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Audio Hub";

pub fn is_enabled() -> Result<bool, String> {
    let Some(registered) = read_registered_command()? else {
        return Ok(false);
    };
    Ok(registered.eq_ignore_ascii_case(&current_command()?))
}

pub fn set_enabled(enabled: bool) -> Result<bool, String> {
    if enabled {
        write_registered_command(&current_command()?)?;
    } else {
        delete_registered_command()?;
    }
    is_enabled()
}

fn current_command() -> Result<String, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("无法确定程序路径：{error}"))?;
    let path = executable.to_string_lossy();
    if path.contains('"') {
        return Err("程序路径包含不受支持的双引号".to_string());
    }
    Ok(format!("\"{path}\""))
}

fn read_registered_command() -> Result<Option<String>, String> {
    let subkey = wide_null(RUN_KEY);
    let value = wide_null(VALUE_NAME);
    let mut byte_count = 0_u32;
    let first = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut byte_count),
        )
    };
    if first.0 == 2 {
        return Ok(None);
    }
    if first != ERROR_SUCCESS || byte_count < 2 {
        return Err(format!("无法读取开机启动设置（Windows 错误 {}）", first.0));
    }

    let mut buffer = vec![0_u16; (byte_count as usize).div_ceil(2)];
    let second = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            Some(&mut byte_count),
        )
    };
    if second != ERROR_SUCCESS {
        return Err(format!("无法读取开机启动设置（Windows 错误 {}）", second.0));
    }
    let length = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    Ok(Some(String::from_utf16_lossy(&buffer[..length])))
}

fn write_registered_command(command: &str) -> Result<(), String> {
    let subkey = wide_null(RUN_KEY);
    let value = wide_null(VALUE_NAME);
    let data = wide_null(command);
    let byte_count =
        u32::try_from(data.len() * size_of::<u16>()).map_err(|_| "开机启动命令过长".to_string())?;
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast::<c_void>()),
            byte_count,
        )
    };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("无法启用开机启动（Windows 错误 {}）", result.0))
    }
}

fn delete_registered_command() -> Result<(), String> {
    let subkey = wide_null(RUN_KEY);
    let value = wide_null(VALUE_NAME);
    let result = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
        )
    };
    if result == ERROR_SUCCESS || result.0 == 2 {
        Ok(())
    } else {
        Err(format!("无法关闭开机启动（Windows 错误 {}）", result.0))
    }
}

fn wide_null(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
