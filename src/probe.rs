//! COM 셸 확장(IContextMenu)의 실제 탐색기 메뉴 표시 문구를 격리 자식 프로세스로 안전하게 조회

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CLSIDFromString, CoCreateInstance, IDataObject,
};
use windows::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX, SetErrorMode,
};
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::System::Registry::{HKEY, HKEY_CLASSES_ROOT, KEY_READ, RegCloseKey, RegOpenKeyExW};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_NORMAL, IContextMenu, IContextMenu2, IShellExtInit, IShellFolder, ILFree, SHBindToParent,
    SHParseDisplayName,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemInfoW, HMENU, MENUITEMINFOW,
    MFT_SEPARATOR, MIIM_FTYPE, MIIM_ID, MIIM_STRING, MIIM_SUBMENU, WM_INITMENUPOPUP,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};

use crate::model::{ContextEntry, EntryKind, TargetLabel};
use crate::path_util::clean_mnemonic;

pub const PROBE_WORKER_ARG: &str = "--probe-menus";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PER_ITEM_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_SUBMENU_DEPTH: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProbeTarget {
    Folder,
    FolderBackground,
    Drive,
    AnyFile,
    FileExt(String),
}

impl ProbeTarget {
    pub fn from_target_label(label: &TargetLabel) -> Self {
        match label {
            TargetLabel::Directory | TargetLabel::FolderVirtual | TargetLabel::LibraryFolder => {
                ProbeTarget::Folder
            }
            TargetLabel::DirectoryBackground
            | TargetLabel::DesktopBackground
            | TargetLabel::LibraryBackground => ProbeTarget::FolderBackground,
            TargetLabel::Drive => ProbeTarget::Drive,
            TargetLabel::AllFiles | TargetLabel::AllFilesystemObjects => ProbeTarget::AnyFile,
            TargetLabel::FileAssociation(name) => {
                if name.starts_with('.') {
                    ProbeTarget::FileExt(sanitize_ext(name))
                } else {
                    ProbeTarget::AnyFile
                }
            }
            TargetLabel::Custom(s) => {
                if s.starts_with('.') {
                    ProbeTarget::FileExt(sanitize_ext(s))
                } else {
                    ProbeTarget::AnyFile
                }
            }
            TargetLabel::SendTo | TargetLabel::ShellNew(_) => ProbeTarget::AnyFile,
        }
    }

    pub fn key_suffix(&self) -> String {
        match self {
            ProbeTarget::Folder => "folder".to_string(),
            ProbeTarget::FolderBackground => "folder_bg".to_string(),
            ProbeTarget::Drive => "drive".to_string(),
            ProbeTarget::AnyFile => "any_file".to_string(),
            ProbeTarget::FileExt(ext) => format!("ext:{ext}"),
        }
    }
}

fn sanitize_ext(raw: &str) -> String {
    let trimmed = raw.trim().to_ascii_lowercase();
    let cleaned: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect();
    if cleaned.len() > 1 && cleaned.starts_with('.') {
        cleaned
    } else {
        ".txt".to_string()
    }
}

/// (CLSID, 적용 대상) 단위 프로브 캐시 키 생성
pub fn probe_key_for_entry(entry: &ContextEntry) -> Option<String> {
    if entry.kind != EntryKind::ShellEx {
        return None;
    }
    if entry.target_exists == Some(false) {
        return None;
    }
    let clsid = entry.clsid.as_deref()?.trim();
    if clsid.is_empty() {
        return None;
    }
    let target = ProbeTarget::from_target_label(&entry.target_label);
    Some(format!("{}|{}", clsid.to_ascii_uppercase(), target.key_suffix()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRequest {
    pub key: String,
    pub clsid: String,
    pub target: ProbeTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResponse {
    pub key: String,
    pub labels: Vec<String>,
}

/// 스캔 결과에서 중복을 제거한 프로브 요청 목록 생성
pub fn build_probe_requests(entries: &[ContextEntry]) -> Vec<ProbeRequest> {
    let mut seen = HashSet::new();
    let mut ext_counts_per_clsid: HashMap<String, usize> = HashMap::new();
    let mut requests = Vec::new();

    for entry in entries {
        if entry.is_system {
            continue;
        }
        let Some(key) = probe_key_for_entry(entry) else {
            continue;
        };
        if !seen.insert(key.clone()) {
            continue;
        }
        let Some(clsid) = entry.clsid.clone() else {
            continue;
        };
        let target = ProbeTarget::from_target_label(&entry.target_label);
        if matches!(target, ProbeTarget::FileExt(_)) {
            let cnt = ext_counts_per_clsid.entry(clsid.to_ascii_uppercase()).or_insert(0);
            if *cnt >= 5 {
                continue;
            }
            *cnt += 1;
        }
        requests.push(ProbeRequest { key, clsid, target });
    }
    requests
}

/// 조회된 동적 메뉴 문구 결과를 각 항목에 반영
pub fn apply_probe_results(entries: &mut [ContextEntry], results: &HashMap<String, Vec<String>>) {
    if results.is_empty() {
        return;
    }
    for entry in entries {
        let Some(key) = probe_key_for_entry(entry) else {
            continue;
        };
        if let Some(labels) = results.get(&key) {
            entry.menu_labels = labels.clone();
        }
    }
}

/// 격리된 자식 프로세스를 통해 프로브 요청 목록을 안전하게 실행
/// 특정 DLL 이 크래시되거나 멈추면 해당 항목만 건너뛰고 남은 항목을 새 워커로 이어서 처리
pub fn run_probe_isolated(requests: &[ProbeRequest]) -> HashMap<String, Vec<String>> {
    let mut results = HashMap::new();
    if requests.is_empty() {
        return results;
    }
    let Ok(exe) = std::env::current_exe() else {
        return results;
    };

    let mut offset = 0usize;
    while offset < requests.len() {
        let slice = &requests[offset..];
        let Ok(payload) = serde_json::to_string(slice) else {
            break;
        };

        let mut child = match Command::new(&exe)
            .arg(PROBE_WORKER_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        {
            Ok(c) => c,
            Err(_) => break,
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(payload.as_bytes());
        }

        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            break;
        };

        let (tx, rx) = mpsc::channel::<String>();
        let reader_handle = std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        let mut processed = 0usize;
        let mut crashed_or_hung = false;

        while processed < slice.len() {
            match rx.recv_timeout(PER_ITEM_TIMEOUT) {
                Ok(line) => {
                    if let Ok(resp) = serde_json::from_str::<ProbeResponse>(&line) {
                        if !resp.labels.is_empty() {
                            results.insert(resp.key, resp.labels);
                        }
                    }
                    processed += 1;
                }
                Err(_) => {
                    crashed_or_hung = true;
                    break;
                }
            }
        }

        if crashed_or_hung {
            let _ = child.kill();
        }
        let _ = child.wait();
        let _ = reader_handle.join();

        if crashed_or_hung {
            // 문제를 일으킨 현재 항목 1개만 건너뛰고 남은 항목 계속 진행
            offset += processed + 1;
        } else {
            offset += processed;
        }
    }

    results
}

/// 워커 프로세스 진입점 (--probe-menus)
pub fn run_probe_worker_main() {
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
    }

    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(requests) = serde_json::from_str::<Vec<ProbeRequest>>(&input) else {
        return;
    };

    let mut env = ProbeSandboxDir::new();
    let ole_ok = unsafe { OleInitialize(None) }.is_ok();
    let mut stdout = std::io::stdout().lock();

    for req in &requests {
        let labels = probe_single_clsid(&req.clsid, &req.target, &mut env);
        let resp = ProbeResponse {
            key: req.key.clone(),
            labels,
        };
        if let Ok(line) = serde_json::to_string(&resp) {
            let _ = writeln!(stdout, "{line}");
            let _ = stdout.flush();
        }
    }

    if ole_ok {
        unsafe { OleUninitialize() };
    }
}

/// 프로브용 임시 폴더 및 더미 파일 관리 (종료 시 자동 정리)
struct ProbeSandboxDir {
    root: PathBuf,
    sample_folder: PathBuf,
    created_files: HashMap<String, PathBuf>,
}

impl ProbeSandboxDir {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cm_menu_probe_{}", std::process::id()));
        let sample_folder = root.join("SampleFolder");
        let _ = std::fs::create_dir_all(&sample_folder);
        let _ = std::fs::write(sample_folder.join("sample.txt"), b"contextmanager probe\r\n");
        let _ = std::fs::write(sample_folder.join("sample2.txt"), b"contextmanager probe 2\r\n");

        let mut created_files = HashMap::new();
        let default_txt = root.join("sample.txt");
        let _ = std::fs::write(&default_txt, b"contextmanager probe\r\n");
        created_files.insert(".txt".to_string(), default_txt);

        Self {
            root,
            sample_folder,
            created_files,
        }
    }

    fn path_for_target(&mut self, target: &ProbeTarget) -> PathBuf {
        match target {
            ProbeTarget::Folder | ProbeTarget::FolderBackground => self.sample_folder.clone(),
            ProbeTarget::Drive => {
                let sys_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
                PathBuf::from(format!("{sys_drive}\\"))
            }
            ProbeTarget::AnyFile => self.file_for_ext(".txt"),
            ProbeTarget::FileExt(ext) => self.file_for_ext(ext),
        }
    }

    fn file_for_ext(&mut self, ext: &str) -> PathBuf {
        let norm = sanitize_ext(ext);
        if let Some(p) = self.created_files.get(&norm) {
            return p.clone();
        }
        let path = self.root.join(format!("sample{norm}"));
        let _ = std::fs::write(&path, b"contextmanager probe\r\n");
        self.created_files.insert(norm, path.clone());
        path
    }
}

impl Drop for ProbeSandboxDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn open_progid_hkey(target: &ProbeTarget) -> Option<HKEY> {
    let candidates: Vec<String> = match target {
        ProbeTarget::Folder => vec!["Folder".into(), "Directory".into()],
        ProbeTarget::FolderBackground => vec!["Directory\\Background".into()],
        ProbeTarget::Drive => vec!["Drive".into()],
        ProbeTarget::AnyFile => vec!["*".into(), "AllFilesystemObjects".into()],
        ProbeTarget::FileExt(ext) => vec![ext.clone(), "*".into()],
    };
    for name in candidates {
        let wide = HSTRING::from(name);
        let mut hkey = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CLASSES_ROOT,
                PCWSTR(wide.as_ptr()),
                Some(0),
                KEY_READ,
                &mut hkey,
            )
        };
        if status.is_ok() {
            return Some(hkey);
        }
    }
    None
}

unsafe fn create_pidl(path: &Path) -> Option<*mut ITEMIDLIST> {
    let wide = HSTRING::from(path.as_os_str());
    let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None) }.ok()?;
    if pidl.is_null() { None } else { Some(pidl) }
}

unsafe fn create_data_object_for_pidl(pidl: *const ITEMIDLIST) -> Option<IDataObject> {
    let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
    let parent: IShellFolder = unsafe { SHBindToParent(pidl, Some(&mut child)) }.ok()?;
    if child.is_null() {
        return None;
    }
    let child_const = child as *const ITEMIDLIST;
    unsafe { parent.GetUIObjectOf::<IDataObject>(HWND::default(), &[child_const], None) }.ok()
}

/// 단일 CLSID 의 IContextMenu 를 조회하여 실제 표시 메뉴 문자열 목록 추출
fn probe_single_clsid(clsid: &str, target: &ProbeTarget, env: &mut ProbeSandboxDir) -> Vec<String> {
    let wide_clsid = HSTRING::from(clsid);
    let Ok(guid) = (unsafe { CLSIDFromString(PCWSTR(wide_clsid.as_ptr())) }) else {
        return Vec::new();
    };

    let Ok(cm) = (unsafe { CoCreateInstance::<_, IContextMenu>(&guid, None, CLSCTX_INPROC_SERVER) })
    else {
        return Vec::new();
    };

    let sample_path = env.path_for_target(target);
    let pidl = unsafe { create_pidl(&sample_path) };
    let progid_hkey = open_progid_hkey(target);

    if let Ok(ext_init) = cm.cast::<IShellExtInit>() {
        let init_res = match target {
            ProbeTarget::FolderBackground => {
                let p_opt = pidl.map(|p| p as *const ITEMIDLIST);
                unsafe { ext_init.Initialize(p_opt, None, progid_hkey) }
            }
            _ => {
                let data_obj = pidl.and_then(|p| unsafe { create_data_object_for_pidl(p) });
                unsafe { ext_init.Initialize(None, data_obj.as_ref(), progid_hkey) }
            }
        };
        if init_res.is_err() {
            cleanup_handles(pidl, progid_hkey);
            return Vec::new();
        }
    }

    let mut labels = Vec::new();
    if let Ok(hmenu) = unsafe { CreatePopupMenu() } {
        let hr = unsafe { cm.QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL) };
        if hr.is_ok() {
            let mut seen = HashSet::new();
            unsafe {
                collect_menu_labels(hmenu, &cm, 0, &mut seen, &mut labels);
            }
        }
        let _ = unsafe { DestroyMenu(hmenu) };
    }

    cleanup_handles(pidl, progid_hkey);
    labels
}

fn cleanup_handles(pidl: Option<*mut ITEMIDLIST>, hkey: Option<HKEY>) {
    if let Some(p) = pidl {
        unsafe { ILFree(Some(p as *const ITEMIDLIST)) };
    }
    if let Some(k) = hkey {
        let _ = unsafe { RegCloseKey(k) };
    }
}

unsafe fn collect_menu_labels(
    hmenu: HMENU,
    cm: &IContextMenu,
    depth: usize,
    seen: &mut HashSet<String>,
    out: &mut Vec<String>,
) {
    let count = unsafe { GetMenuItemCount(Some(hmenu)) };
    if count <= 0 {
        return;
    }

    for i in 0..(count as u32) {
        let mut mii = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STRING | MIIM_SUBMENU | MIIM_ID,
            ..Default::default()
        };
        if unsafe { GetMenuItemInfoW(hmenu, i, true, &mut mii) }.is_err() {
            continue;
        }
        if (mii.fType & MFT_SEPARATOR) == MFT_SEPARATOR {
            continue;
        }

        let mut text = String::new();
        if mii.cch > 0 {
            let mut buf = vec![0u16; (mii.cch as usize) + 1];
            mii.dwTypeData = PWSTR(buf.as_mut_ptr());
            mii.cch = buf.len() as u32;
            mii.fMask = MIIM_STRING;
            if unsafe { GetMenuItemInfoW(hmenu, i, true, &mut mii) }.is_ok() {
                let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                text = String::from_utf16_lossy(&buf[..end]);
            }
        }

        let cleaned = normalize_menu_label(&text);
        if !cleaned.is_empty() {
            let dedup = cleaned.to_ascii_lowercase();
            if seen.insert(dedup) {
                out.push(cleaned);
            }
        }

        if !mii.hSubMenu.is_invalid() && depth < MAX_SUBMENU_DEPTH {
            // 일부 셸 확장은 WM_INITMENUPOPUP 에서 하위 메뉴를 동적으로 채움
            if unsafe { GetMenuItemCount(Some(mii.hSubMenu)) } <= 0 {
                if let Ok(cm2) = cm.cast::<IContextMenu2>() {
                    let _ = unsafe {
                        cm2.HandleMenuMsg(
                            WM_INITMENUPOPUP,
                            WPARAM(mii.hSubMenu.0 as usize),
                            LPARAM(i as isize),
                        )
                    };
                }
            }
            unsafe {
                collect_menu_labels(mii.hSubMenu, cm, depth + 1, seen, out);
            }
        }
    }
}

/// 메뉴 문자열에서 가속키(&) 및 탭 뒤 단축키 표기를 정리
pub fn normalize_menu_label(raw: &str) -> String {
    let before_tab = raw.split('\t').next().unwrap_or(raw);
    clean_mnemonic(before_tab)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_menu_label_strips_mnemonic_and_tab_shortcut() {
        assert_eq!(
            normalize_menu_label("Acrobat에서 파일 결합(&C)...\tCtrl+M"),
            "Acrobat에서 파일 결합(C)..."
        );
        assert_eq!(normalize_menu_label("&Combine files in Acrobat..."), "Combine files in Acrobat...");
        assert_eq!(normalize_menu_label("   "), "");
    }

    #[test]
    fn sanitize_ext_keeps_valid_extensions_and_falls_back() {
        assert_eq!(sanitize_ext(".PDF"), ".pdf");
        assert_eq!(sanitize_ext("invalid"), ".txt");
        assert_eq!(sanitize_ext("."), ".txt");
    }

    #[test]
    fn build_and_apply_probe_results_dedupes_and_maps_per_target() {
        use crate::model::{Category, Scope};
        use crate::reg::RegRoot;

        let make_entry = |id: &str, target: TargetLabel| ContextEntry {
            id: id.into(),
            display_name: "Adobe.Acrobat.ContextMenu".into(),
            raw_name: "Adobe.Acrobat.ContextMenu".into(),
            kind: EntryKind::ShellEx,
            scope: Scope::System,
            category: Category::Directory,
            target_label: target,
            registry_root: Some(RegRoot::Hklm),
            registry_subkey: Some(id.into()),
            file_path: None,
            disabled_file_path: None,
            command: None,
            resolved_target: Some("C:\\Adobe\\ContextMenuShim64.dll".into()),
            target_exists: Some(true),
            clsid: Some("{A6595CD1-BF77-430A-A452-18696685F7C7}".into()),
            clsid_description: Some("Acrobat Elements Context Menu".into()),
            menu_labels: Vec::new(),
            package_name: None,
            icon: None,
            disabled: false,
            is_system: false,
            group_key: "g1".into(),
        };

        let mut entries = vec![
            make_entry("e1", TargetLabel::FolderVirtual),
            make_entry("e2", TargetLabel::Directory),
            make_entry("e3", TargetLabel::AllFiles),
        ];

        let reqs = build_probe_requests(&entries);
        // FolderVirtual 과 Directory 는 모두 ProbeTarget::Folder 로 중복 제거됨
        assert_eq!(reqs.len(), 2);

        let mut map = HashMap::new();
        map.insert(
            "{A6595CD1-BF77-430A-A452-18696685F7C7}|folder".to_string(),
            vec!["Acrobat에서 파일 결합...".to_string()],
        );
        map.insert(
            "{A6595CD1-BF77-430A-A452-18696685F7C7}|any_file".to_string(),
            vec!["Adobe PDF로 변환(B)".to_string()],
        );
        apply_probe_results(&mut entries, &map);

        assert_eq!(entries[0].menu_labels, vec!["Acrobat에서 파일 결합..."]);
        assert_eq!(entries[1].menu_labels, vec!["Acrobat에서 파일 결합..."]);
        assert_eq!(entries[2].menu_labels, vec!["Adobe PDF로 변환(B)"]);
    }

    #[test]
    #[ignore]
    fn live_probe_acrobat_and_shellex() {
        let mut env = ProbeSandboxDir::new();
        let ole_ok = unsafe { OleInitialize(None) }.is_ok();
        let acrobat = probe_single_clsid(
            "{A6595CD1-BF77-430A-A452-18696685F7C7}",
            &ProbeTarget::Folder,
            &mut env,
        );
        println!("Acrobat (Folder): {acrobat:?}");
        let acrobat_file = probe_single_clsid(
            "{A6595CD1-BF77-430A-A452-18696685F7C7}",
            &ProbeTarget::AnyFile,
            &mut env,
        );
        println!("Acrobat (AnyFile): {acrobat_file:?}");
        let foxit = probe_single_clsid(
            "{C5269811-4A29-4818-A4BB-111F9FC63A5F}",
            &ProbeTarget::Folder,
            &mut env,
        );
        println!("Foxit (Folder): {foxit:?}");
        if ole_ok {
            unsafe { OleUninitialize() };
        }
    }
}
