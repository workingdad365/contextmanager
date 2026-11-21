//! 경로 추출, 환경변수 확장, MUI 간접 문자열 해석, 바로가기(.lnk) 해석 및 시스템 항목 판별

use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::UI::Shell::{IShellLinkW, SHLoadIndirectString, SLGP_RAWPATH, ShellLink};
use windows::core::{HSTRING, Interface, PCWSTR};

/// %VAR% 형태의 Windows 환경변수를 확장
pub fn expand_env(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    let wide = HSTRING::from(input);
    let required = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), None) };
    if required == 0 {
        return input.to_string();
    }
    let mut buf = vec![0u16; required as usize];
    let written = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buf)) };
    if written == 0 {
        return input.to_string();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// @dll,-id 형태의 MUI 간접 문자열을 실제 표시 텍스트로 해석
pub fn resolve_mui_string(input: &str) -> String {
    let trimmed = input.trim();
    if !trimmed.starts_with('@') {
        return clean_mnemonic(trimmed);
    }
    let wide = HSTRING::from(trimmed);
    let mut out = vec![0u16; 512];
    let ok = unsafe { SHLoadIndirectString(PCWSTR(wide.as_ptr()), &mut out, None) }.is_ok();
    if ok {
        let end = out.iter().position(|&c| c == 0).unwrap_or(out.len());
        let resolved = String::from_utf16_lossy(&out[..end]);
        if !resolved.trim().is_empty() {
            return clean_mnemonic(&resolved);
        }
    }
    clean_mnemonic(trimmed)
}

/// 메뉴 단축키 표기(&O 등)를 깔끔하게 정리 (&& 는 & 로 유지)
pub fn clean_mnemonic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
            // 단일 '&' 는 메뉴 가속키 표시이므로 제거
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// COM InprocServer32 / LocalServer32 값에서 DLL/EXE 경로를 추출
/// InprocServer32 는 보통 따옴표 없이 공백 포함 전체 경로(C:\Program Files\...\foo.dll)로 저장됨
pub fn extract_server_path(raw: &str) -> Option<String> {
    let expanded = expand_env(raw.trim());
    let trimmed = expanded.trim();
    if trimmed.is_empty() {
        return None;
    }
    let unquoted = trimmed.trim_matches('"').trim();
    if Path::new(unquoted).exists() {
        return Some(unquoted.to_string());
    }
    let lower = unquoted.to_ascii_lowercase();
    if (lower.ends_with(".dll") || lower.ends_with(".ocx") || lower.ends_with(".cpl") || lower.ends_with(".exe"))
        && !unquoted.contains(" /")
        && !unquoted.contains(" -")
    {
        return Some(unquoted.to_string());
    }
    extract_executable_from_command(trimmed)
}

/// %1, %L, %V, %* 같은 셸 플레이스홀더인지 판별
fn is_shell_placeholder(token: &str) -> bool {
    let t = token.trim().trim_matches('"');
    t.starts_with('%') && !t.contains('\\') && !t.contains('/') && t.len() <= 4
}

/// 레지스트리 command 문자열에서 실제 실행 파일 경로를 추출
/// 예: `"C:\Users\me\AppData\Local\Programs\cursor\Cursor.exe" "%V"` -> `C:\Users\me\AppData\Local\Programs\cursor\Cursor.exe`
pub fn extract_executable_from_command(cmd: &str) -> Option<String> {
    let expanded = expand_env(cmd.trim());
    let s = expanded.trim();
    if s.is_empty() {
        return None;
    }

    // 1. 큰따옴표로 시작하는 경우 첫 번째 따옴표 쌍 내부 추출
    if let Some(rest) = s.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let candidate = rest[..end].trim();
            if candidate.is_empty() || is_shell_placeholder(candidate) {
                return None;
            }
            return Some(unwrap_wrapper_target(candidate, rest[end + 1..].trim()));
        }
    }

    // 2. cmd /c, cmd /k 등 래퍼 명령이 따옴표 없이 시작하는 경우 우선 처리
    if let Some(first_ws) = s.find(char::is_whitespace) {
        let first_tok = &s[..first_ws];
        let rest_args = s[first_ws..].trim();
        let first_lower = Path::new(first_tok)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(first_tok)
            .to_ascii_lowercase();
        if matches!(first_lower.as_str(), "cmd" | "cmd.exe" | "rundll32" | "rundll32.exe") {
            return Some(unwrap_wrapper_target(first_tok, rest_args));
        }
    }

    // 3. 따옴표가 없는 경우: 확장자 경계를 탐색 (앞부분에 인자 스위치가 섞이지 않은 경우만)
    let lower = s.to_ascii_lowercase();
    for ext in [".exe", ".dll", ".ocx", ".cpl", ".com", ".bat", ".cmd", ".msc", ".vbs", ".ps1"] {
        let mut search_from = 0;
        while let Some(pos) = lower[search_from..].find(ext) {
            let end_idx = search_from + pos + ext.len();
            let candidate = s[..end_idx].trim();
            let after = &s[end_idx..];
            let has_switch_inside = candidate.contains(" /") || candidate.contains(" -");
            if !has_switch_inside
                && (after.is_empty() || after.starts_with(char::is_whitespace) || after.starts_with(','))
            {
                if is_shell_placeholder(candidate) {
                    return None;
                }
                return Some(unwrap_wrapper_target(candidate, after.trim()));
            }
            search_from = end_idx;
        }
    }

    // 4. 공백 기준 첫 토큰 반환
    let first = s.split_whitespace().next()?;
    if first.is_empty() || is_shell_placeholder(first) {
        None
    } else {
        Some(first.to_string())
    }
}

/// rundll32.exe / cmd.exe 의 경우 실제 대상 DLL 또는 스크립트 파일이 인자에 있으면 추출
fn unwrap_wrapper_target(exe: &str, args: &str) -> String {
    let file_name = Path::new(exe)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if file_name == "rundll32.exe" || file_name == "rundll32" {
        if let Some(dll) = extract_rundll_target(args) {
            return dll;
        }
    } else if file_name == "cmd.exe" || file_name == "cmd" {
        if let Some(script) = extract_cmd_script_target(args) {
            return script;
        }
    }
    exe.to_string()
}

fn extract_cmd_script_target(args: &str) -> Option<String> {
    let mut rest = args.trim();
    // /c, /k, /q, /d 등의 스위치 건너뛰기
    loop {
        let lower = rest.to_ascii_lowercase();
        if lower.starts_with("/c ")
            || lower.starts_with("/k ")
            || lower.starts_with("/q ")
            || lower.starts_with("/d ")
        {
            rest = rest[2..].trim_start();
        } else {
            break;
        }
    }
    if rest.is_empty() {
        return None;
    }
    // 따옴표로 감싸진 스크립트 경로
    if let Some(after_q) = rest.strip_prefix('"') {
        if let Some(end) = after_q.find('"') {
            let cand = after_q[..end].trim();
            if looks_like_file_path(cand) {
                return Some(cand.to_string());
            }
        }
    }
    // 확장자 경계 탐색
    let lower = rest.to_ascii_lowercase();
    for ext in [".bat", ".cmd", ".exe", ".ps1", ".vbs"] {
        if let Some(pos) = lower.find(ext) {
            let end_idx = pos + ext.len();
            let cand = rest[..end_idx].trim();
            if looks_like_file_path(cand) {
                return Some(cand.to_string());
            }
        }
    }
    None
}

fn looks_like_file_path(s: &str) -> bool {
    let bytes = s.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
        || s.starts_with("\\\\")
}

fn extract_rundll_target(args: &str) -> Option<String> {
    let a = args.trim();
    if a.is_empty() {
        return None;
    }
    if let Some(rest) = a.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let p = rest[..end].trim();
            if !p.is_empty() {
                return Some(p.to_string());
            }
        }
    }
    // 쉼표 앞부분이 DLL 경로 (예: C:\Windows\System32\shell32.dll,Control_RunDLL)
    let before_comma = a.split(',').next()?.trim();
    let lower = before_comma.to_ascii_lowercase();
    if lower.ends_with(".dll") || lower.ends_with(".cpl") || lower.ends_with(".ocx") {
        return Some(before_comma.to_string());
    }
    None
}

/// Icon 문자열 (`"C:\...\app.exe",0` 또는 `C:\...\app.exe,-101`)에서 파일 경로만 추출
pub fn extract_path_from_icon(icon: &str) -> Option<String> {
    let expanded = expand_env(icon.trim());
    let s = expanded.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let p = rest[..end].trim();
            return if p.is_empty() { None } else { Some(p.to_string()) };
        }
    }
    // 끝부분의 ,숫자 인덱스 제거
    let path_part = match s.rfind(',') {
        Some(idx) if s[idx + 1..].trim().parse::<i32>().is_ok() => s[..idx].trim(),
        _ => s,
    };
    let clean = path_part.trim_matches('"').trim();
    if clean.is_empty() {
        None
    } else {
        Some(clean.to_string())
    }
}

/// 파일 존재 여부 확인 (WindowsApps 등 접근 권한만 제한된 기존 파일은 존재하는 것으로 간주)
pub fn path_exists_safe(p: &Path) -> bool {
    match std::fs::metadata(p) {
        Ok(_) => true,
        Err(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
    }
}

/// 추출된 실행 파일/DLL 경로가 실제 디스크에 존재하는지 검사
/// 경로 구분자 없이 이름만 있는 경우(예: notepad.exe) System32, Windows, PATH 에서 탐색
pub fn resolve_and_check_exists(raw_path: &str) -> (String, bool) {
    let expanded = expand_env(raw_path.trim()).trim_matches('"').to_string();
    if expanded.is_empty() {
        return (expanded, false);
    }

    let p = Path::new(&expanded);
    if path_exists_safe(p) {
        return (expanded, true);
    }

    // 절대 경로인데 확장자가 생략된 경우(예: C:\Windows\system32\perfmon) .exe 보완 검사
    if p.is_absolute() {
        if p.extension().is_none() {
            let with_exe = format!("{expanded}.exe");
            if path_exists_safe(Path::new(&with_exe)) {
                return (with_exe, true);
            }
        }
        return (expanded, false);
    }

    // 파일명만 지정된 경우 PATH 및 시스템 디렉토리에서 검색
    if let Some(found) = find_in_path(&expanded) {
        return (found.to_string_lossy().into_owned(), true);
    }
    (expanded, false)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let candidates: Vec<String> = if Path::new(name).extension().is_some() {
        vec![name.to_string()]
    } else {
        vec![
            name.to_string(),
            format!("{name}.exe"),
            format!("{name}.com"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
        ]
    };

    let mut search_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(windir) = std::env::var("WINDIR") {
        search_dirs.push(PathBuf::from(&windir).join("System32"));
        search_dirs.push(PathBuf::from(&windir));
    }
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_env) {
            search_dirs.push(dir);
        }
    }

    for dir in search_dirs {
        for cand in &candidates {
            let full = dir.join(cand);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// .lnk 바로가기 파일의 대상 경로를 COM IShellLinkW 로 조회
pub fn resolve_lnk_target(lnk_path: &Path) -> Option<String> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninit = hr.is_ok();

        let result = (|| -> Option<String> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let persist: IPersistFile = link.cast().ok()?;
            let wide = HSTRING::from(lnk_path.as_os_str());
            persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;

            let mut buf = vec![0u16; 1024];
            link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32).ok()?;
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let target = String::from_utf16_lossy(&buf[..end]);
            if target.trim().is_empty() {
                None
            } else {
                Some(expand_env(&target))
            }
        })();

        if should_uninit {
            CoUninitialize();
        }
        result
    }
}

/// Microsoft / Windows 기본 내장 항목인지 휴리스틱 판별
pub fn is_likely_system_entry(
    raw_name: &str,
    resolved_target: Option<&str>,
    package_name: Option<&str>,
) -> bool {
    if let Some(pkg) = package_name {
        let lower = pkg.to_ascii_lowercase();
        if lower.starts_with("microsoft.windows")
            || lower.starts_with("microsoft.paint")
            || lower.starts_with("microsoft.windowsterminal")
            || lower.starts_with("microsoft.windowsnotepad")
            || lower.starts_with("microsoft.screensketch")
            || lower.starts_with("windows.")
        {
            return true;
        }
    }

    if let Some(target) = resolved_target {
        let lower = target.to_ascii_lowercase();
        if lower.starts_with("c:\\windows\\")
            || lower.starts_with("%systemroot%\\")
            || lower.starts_with("%windir%\\")
            || lower.contains("\\windows\\system32\\")
            || lower.contains("\\windows\\syswow64\\")
            || lower.contains("\\windows\\explorer.exe")
            || lower.contains("\\windows defender\\")
        {
            return true;
        }
    }

    // 명령어가 따로 없이 시스템이 내장 처리하는 기본 동사
    if resolved_target.is_none() {
        let lower = raw_name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "open"
                | "opennewprocess"
                | "opennewwindow"
                | "explore"
                | "find"
                | "edit"
                | "print"
                | "printto"
                | "runas"
                | "runasuser"
                | "properties"
                | "copyaspath"
                | "pintohome"
                | "pintostartscreen"
                | "windows.modernshare"
                | "cmd"
                | "powershell"
        ) {
            return true;
        }
    }

    false
}

/// 파일 경로에서 그룹화용 정규화 키 생성 (대소문자/슬래시 통일)
pub fn normalize_path_key(path: &str) -> String {
    path.trim().trim_matches('"').replace('/', "\\").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_mnemonic_removes_single_ampersand_and_keeps_double() {
        assert_eq!(clean_mnemonic("Open with &Cursor"), "Open with Cursor");
        assert_eq!(clean_mnemonic("Copy && Paste"), "Copy & Paste");
        assert_eq!(clean_mnemonic("&열기(&O)"), "열기(O)");
    }

    #[test]
    fn extract_executable_handles_quoted_unquoted_and_rundll() {
        assert_eq!(
            extract_executable_from_command(
                "\"C:\\Users\\test\\AppData\\Local\\Programs\\cursor\\Cursor.exe\" \"%V\""
            )
            .as_deref(),
            Some("C:\\Users\\test\\AppData\\Local\\Programs\\cursor\\Cursor.exe")
        );
        assert_eq!(
            extract_executable_from_command("C:\\Program Files\\My App\\tool.exe %1").as_deref(),
            Some("C:\\Program Files\\My App\\tool.exe")
        );
        assert_eq!(
            extract_executable_from_command("rundll32.exe C:\\Windows\\System32\\shimgvw.dll,ImageView_Fullscreen %1")
                .as_deref(),
            Some("C:\\Windows\\System32\\shimgvw.dll")
        );
        assert_eq!(
            extract_executable_from_command("cmd /k c:\\[bat]\\mkv2mp4.bat \"%1\"").as_deref(),
            Some("c:\\[bat]\\mkv2mp4.bat")
        );
        assert_eq!(extract_executable_from_command("\"%1\" %*"), None);
        assert_eq!(
            extract_server_path("C:\\Program Files\\Beyond Compare 5\\BCShellEx64.dll").as_deref(),
            Some("C:\\Program Files\\Beyond Compare 5\\BCShellEx64.dll")
        );
        assert_eq!(extract_executable_from_command("   "), None);
    }

    #[test]
    fn extract_icon_path_strips_index_and_quotes() {
        assert_eq!(
            extract_path_from_icon("\"C:\\Apps\\Cursor.exe\",0").as_deref(),
            Some("C:\\Apps\\Cursor.exe")
        );
        assert_eq!(
            extract_path_from_icon("C:\\Windows\\System32\\shell32.dll,-210").as_deref(),
            Some("C:\\Windows\\System32\\shell32.dll")
        );
    }

    #[test]
    fn system_entry_detection_distinguishes_windows_and_third_party() {
        assert!(is_likely_system_entry(
            "cmd",
            Some("C:\\Windows\\System32\\cmd.exe"),
            None
        ));
        assert!(!is_likely_system_entry(
            "Cursor",
            Some("C:\\Users\\me\\AppData\\Local\\Programs\\cursor\\Cursor.exe"),
            None
        ));
        assert!(is_likely_system_entry("open", None, None));
        assert!(is_likely_system_entry(
            "Terminal",
            None,
            Some("Microsoft.WindowsTerminal_1.20.0_x64__8wekyb3d8bbwe")
        ));
    }
}
