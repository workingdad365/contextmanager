//! Win32 공용 파일 선택 대화상자 (Shell 명령 / 아이콘 경로 선택용)

use std::path::PathBuf;

use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

const FILE_BUF_LEN: usize = 4096;

fn wide_nul(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn build_filter(pairs: &[(&str, &str)]) -> Vec<u16> {
    let mut v = Vec::new();
    for &(label, pattern) in pairs {
        v.extend(wide_nul(label));
        v.extend(wide_nul(pattern));
    }
    v.push(0);
    v
}

fn open_file(title: &str, filter_pairs: &[(&str, &str)]) -> Option<PathBuf> {
    let filter = build_filter(filter_pairs);
    let title_w = HSTRING::from(title);
    let mut buf = vec![0u16; FILE_BUF_LEN];

    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        nFilterIndex: 1,
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title_w.as_ptr()),
        Flags: OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST,
        ..Default::default()
    };

    let ok = unsafe { GetOpenFileNameW(&mut ofn) };
    if !ok.as_bool() {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// 실행 파일 또는 스크립트 선택 (.exe, .bat, .cmd, .ps1 등)
pub fn pick_command_file(title: &str) -> Option<PathBuf> {
    open_file(
        title,
        &[
            ("Executable / Script (*.exe;*.bat;*.cmd;*.ps1;*.vbs)", "*.exe;*.bat;*.cmd;*.ps1;*.vbs;*.com"),
            ("All Files (*.*)", "*.*"),
        ],
    )
}

/// 아이콘 파일 또는 아이콘이 포함된 실행 파일/DLL 선택 (.ico, .exe, .dll)
pub fn pick_icon_file(title: &str) -> Option<PathBuf> {
    open_file(
        title,
        &[
            ("Icon / Executable / DLL (*.ico;*.exe;*.dll)", "*.ico;*.exe;*.dll"),
            ("All Files (*.*)", "*.*"),
        ],
    )
}
