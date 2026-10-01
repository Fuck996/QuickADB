use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, RECT},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    System::{
        Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegCreateKeyExW,
            RegDeleteValueW, RegSetValueExW,
        },
        Threading::CreateMutexW,
    },
    UI::WindowsAndMessaging::{
        FindWindowW, GetCursorPos, MB_ICONERROR, MB_OK, MessageBoxW, SW_SHOW, SetForegroundWindow,
        ShowWindow,
    },
};

pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub fn data_directory() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("Windows 未提供 LOCALAPPDATA")?)
            .join("QuickADB"),
    )
}

pub fn set_startup(enabled: bool) -> Result<()> {
    let mut key: HKEY = ptr::null_mut();
    let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            ptr::null(),
            0,
            KEY_SET_VALUE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        );
        ensure!(status == 0, "无法打开开机启动配置：{status}");
        let name = wide("QuickADB");
        let status = if enabled {
            let executable = std::env::current_exe()?;
            let command = wide(&format!("\"{}\" --tray", executable.display()));
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, name.as_ptr())
        };
        RegCloseKey(key);
        ensure!(
            status == 0 || (!enabled && status == 2),
            "无法修改开机启动配置：{status}"
        );
    }
    Ok(())
}

pub struct Instance(HANDLE);
impl Instance {
    pub fn acquire() -> Result<Option<Self>> {
        let name = wide("Local\\QuickADB.Native.Application");
        unsafe {
            let mutex = CreateMutexW(ptr::null(), 0, name.as_ptr());
            ensure!(
                !mutex.is_null(),
                "无法创建单实例锁：{}",
                std::io::Error::last_os_error()
            );
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(mutex);
                let hwnd = FindWindowW(ptr::null(), wide("QuickADB").as_ptr());
                if !hwnd.is_null() {
                    ShowWindow(hwnd, SW_SHOW);
                    SetForegroundWindow(hwnd);
                }
                return Ok(None);
            }
            Ok(Some(Self(mutex)))
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn show_error(error: &str) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(error).as_ptr(),
            wide("QuickADB 启动失败").as_ptr(),
            MB_ICONERROR | MB_OK,
        );
    }
}

pub fn window_visible() -> bool {
    unsafe {
        let window = FindWindowW(ptr::null(), wide("QuickADB").as_ptr());
        !window.is_null()
            && windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(window) != 0
    }
}

pub fn monitor_work_area() -> RECT {
    unsafe {
        let mut point = std::mem::zeroed();
        GetCursorPos(&mut point);
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT::default(),
            rcWork: RECT::default(),
            dwFlags: 0,
        };
        GetMonitorInfoW(monitor, &mut info);
        info.rcWork
    }
}

pub fn clamp_position(position: [f32; 2], size: [f32; 2], dpi: f32) -> Result<[f32; 2]> {
    unsafe {
        let point = windows_sys::Win32::Foundation::POINT {
            x: (position[0] * dpi) as i32,
            y: (position[1] * dpi) as i32,
        };
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT::default(),
            rcWork: RECT::default(),
            dwFlags: 0,
        };
        ensure!(
            GetMonitorInfoW(monitor, &mut info) != 0,
            "无法获取显示器工作区域：{}",
            std::io::Error::last_os_error()
        );
        let left = info.rcWork.left as f32 / dpi;
        let top = info.rcWork.top as f32 / dpi;
        Ok([
            position[0].clamp(left, (info.rcWork.right as f32 / dpi - size[0]).max(left)),
            position[1].clamp(top, (info.rcWork.bottom as f32 / dpi - size[1]).max(top)),
        ])
    }
}
