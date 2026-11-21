//! Windows 권한 확인, 권한 상승 재실행, 탐색기/레지스트리 편집기 연동

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

use crate::reg::{RegKey, RegRoot};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 현재 프로세스가 관리자 권한으로 실행 중인지 확인
pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut out_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut out_len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

pub const ELEVATED_ARG: &str = "--elevated";

/// UAC 프롬프트를 통해 현재 실행 파일을 관리자 권한으로 다시 실행
pub fn relaunch_as_admin() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("실행 파일 경로 확인 실패: {e}"))?;
    let exe_w = HSTRING::from(exe.as_os_str());
    let arg_w = HSTRING::from(ELEVATED_ARG);
    let ret = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(exe_w.as_ptr()),
            PCWSTR(arg_w.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (ret.0 as isize) <= 32 {
        return Err("관리자 권한 실행이 취소되었거나 실패했습니다.".into());
    }
    Ok(())
}

/// 현재 Windows 빌드 번호 조회 (22000 이상이면 Windows 11)
pub fn windows_build_number() -> u32 {
    RegKey::open_read(RegRoot::Hklm, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .and_then(|k| k.read_string("CurrentBuildNumber"))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0)
}

/// 지정한 레지스트리 경로를 regedit.exe 에서 바로 열기
pub fn open_regedit_at(full_key_path: &str) -> Result<(), String> {
    let regedit_key = "Software\\Microsoft\\Windows\\CurrentVersion\\Applets\\Regedit";
    if let Ok(key) = RegKey::create_write(RegRoot::Hkcu, regedit_key) {
        let last_key = format!("컴퓨터\\{full_key_path}");
        let _ = key.set_string("LastKey", &last_key);
    }
    let ret = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("regedit.exe"),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (ret.0 as isize) <= 32 {
        return Err("레지스트리 편집기(regedit.exe)를 열 수 없습니다.".into());
    }
    Ok(())
}

/// 파일 또는 폴더를 탐색기에서 선택된 상태로 열기
pub fn open_in_explorer(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let arg = if p.is_file() {
        format!("/select,\"{path}\"")
    } else if p.is_dir() {
        format!("\"{path}\"")
    } else if let Some(parent) = p.parent().filter(|pp| pp.exists()) {
        format!("\"{}\"", parent.display())
    } else {
        return Err("파일이나 상위 폴더가 존재하지 않습니다.".into());
    };

    let arg_w = HSTRING::from(arg);
    let ret = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("explorer.exe"),
            PCWSTR(arg_w.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (ret.0 as isize) <= 32 {
        return Err("탐색기를 열 수 없습니다.".into());
    }
    Ok(())
}

/// Windows 탐색기 프로세스를 종료 후 다시 시작
pub fn restart_explorer() -> Result<(), String> {
    let status = Command::new("taskkill.exe")
        .args(["/F", "/IM", "explorer.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("탐색기 종료 실패: {e}"))?;

    if !status.success() {
        // explorer 가 이미 꺼져 있어도 재실행 시도
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    Command::new("explorer.exe")
        .spawn()
        .map_err(|e| format!("탐색기 재실행 실패: {e}"))?;
    Ok(())
}

/// 패키지 전체 이름 유효성 검사 (영문/숫자/./_/- 만 허용)
pub fn is_valid_package_full_name(pkg: &str) -> bool {
    !pkg.is_empty()
        && pkg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// 잔여 Windows 11 AppX / Sparse Package 제거
pub fn remove_appx_package(package_full_name: &str) -> Result<(), String> {
    if !is_valid_package_full_name(package_full_name) {
        return Err(format!("올바르지 않은 패키지 이름: {package_full_name}"));
    }
    let script = format!("Remove-AppxPackage -Package '{package_full_name}' -ErrorAction Stop");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("PowerShell 실행 실패: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("exit code {:?}", output.status.code())
        } else {
            stderr.lines().next().unwrap_or(&stderr).to_string()
        };
        Err(format!("패키지 제거 실패 ({package_full_name}): {detail}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_full_name_validation_rejects_shell_metachars() {
        assert!(is_valid_package_full_name("shellmenua_5.0.0.0_neutral__5r2yrzxs9d21c"));
        assert!(is_valid_package_full_name("Emurasoft.EmEditor.EmEditor_1.0.0.0_x64__bpd82vs5ee13j"));
        assert!(!is_valid_package_full_name(""));
        assert!(!is_valid_package_full_name("pkg'; calc.exe #"));
        assert!(!is_valid_package_full_name("pkg name with space"));
    }
}
