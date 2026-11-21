//! 컨텍스트 메뉴 항목 비활성화(숨기기), 활성화 및 백업 후 삭제 실행기

use std::path::Path;

use crate::backup::{self, BackupRecord};
use crate::model::{ContextEntry, EntryKind};
use crate::reg::{self, RegKey, RegRoot};
use crate::scanner::{BLOCKED_SUBKEY, sendto_active_dir, sendto_disabled_dir};

/// 일괄 작업 실행 결과
#[derive(Debug, Default)]
pub struct ActionReport {
    pub succeeded: usize,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
}

/// 선택 항목들을 일괄 비활성화(disable=true) 또는 다시 활성화(disable=false)
pub fn set_entries_disabled(entries: &[&ContextEntry], disable: bool) -> ActionReport {
    let mut report = ActionReport::default();
    for entry in entries {
        match set_single_disabled(entry, disable) {
            Ok(()) => report.succeeded += 1,
            Err(e) => report.errors.push(format!("[{}] {e}", entry.display_name)),
        }
    }
    report
}

fn set_single_disabled(entry: &ContextEntry, disable: bool) -> Result<(), String> {
    match entry.kind {
        EntryKind::ShellVerb => {
            let root = entry.registry_root.ok_or("레지스트리 루트 정보 없음")?;
            let subkey = entry.registry_subkey.as_deref().ok_or("레지스트리 경로 없음")?;
            let key = RegKey::open_write(root, subkey)?;
            if disable {
                key.set_string("LegacyDisable", "")
            } else {
                key.delete_value("LegacyDisable")
            }
        }
        EntryKind::ShellEx | EntryKind::Win11Packaged => {
            let clsid = entry.clsid.as_deref().ok_or("CLSID 정보가 없습니다.")?;
            if disable {
                let key = RegKey::create_write(RegRoot::Hkcu, BLOCKED_SUBKEY)?;
                key.set_string(clsid, &entry.display_name)
            } else {
                // HKCU Blocked 에서 제거
                if let Ok(key) = RegKey::open_write(RegRoot::Hkcu, BLOCKED_SUBKEY) {
                    key.delete_value(clsid)?;
                }
                // HKLM Blocked 에도 등록되어 있으면 제거 시도
                if let Some(ro) = RegKey::open_read(RegRoot::Hklm, BLOCKED_SUBKEY) {
                    if ro.value_exists(clsid) {
                        let key = RegKey::open_write(RegRoot::Hklm, BLOCKED_SUBKEY)?;
                        key.delete_value(clsid)?;
                    }
                }
                Ok(())
            }
        }
        EntryKind::SendTo => {
            let active_dir = sendto_active_dir().ok_or("SendTo 폴더 경로를 찾을 수 없습니다.")?;
            let disabled_dir = sendto_disabled_dir().ok_or("비활성화 보관 폴더 경로를 찾을 수 없습니다.")?;
            let file_name = entry.raw_name.as_str();
            let active_path = active_dir.join(file_name);
            let disabled_path = disabled_dir.join(file_name);

            if disable {
                if !active_path.exists() {
                    return Ok(());
                }
                std::fs::create_dir_all(&disabled_dir)
                    .map_err(|e| format!("보관 폴더 생성 실패: {e}"))?;
                move_file_safe(&active_path, &disabled_path)
            } else {
                if !disabled_path.exists() {
                    return Ok(());
                }
                std::fs::create_dir_all(&active_dir)
                    .map_err(|e| format!("SendTo 폴더 생성 실패: {e}"))?;
                move_file_safe(&disabled_path, &active_path)
            }
        }
        EntryKind::ShellNew => {
            let root = entry.registry_root.ok_or("레지스트리 루트 정보 없음")?;
            let subkey = entry.registry_subkey.as_deref().ok_or("레지스트리 경로 없음")?;
            let (parent, leaf) = subkey
                .rsplit_once('\\')
                .ok_or_else(|| format!("잘못된 ShellNew 경로: {subkey}"))?;
            let target_leaf = if disable { "ShellNew_Disabled" } else { "ShellNew" };
            if leaf.eq_ignore_ascii_case(target_leaf) {
                return Ok(());
            }
            reg::rename_subkey(root, parent, leaf, target_leaf)
        }
    }
}

/// 선택 항목들을 먼저 자동 백업한 뒤 삭제 수행
/// 백업에 실패하면 삭제를 중단하고 오류를 반환
pub fn delete_entries_with_backup(
    entries: &[&ContextEntry],
    backup_dir: &Path,
) -> Result<(BackupRecord, ActionReport), String> {
    if entries.is_empty() {
        return Err("삭제할 항목이 선택되지 않았습니다.".into());
    }

    let record = backup::create_backup(entries, backup_dir)?;
    let mut report = ActionReport::default();

    for entry in entries {
        let res = match entry.kind {
            EntryKind::ShellVerb | EntryKind::ShellEx | EntryKind::ShellNew => {
                match (entry.registry_root, entry.registry_subkey.as_deref()) {
                    (Some(root), Some(subkey)) => reg::delete_tree(root, subkey),
                    _ => Err("레지스트리 경로 정보가 없습니다.".into()),
                }
            }
            EntryKind::SendTo => {
                let target = entry
                    .disabled_file_path
                    .as_deref()
                    .or(entry.file_path.as_deref());
                match target {
                    Some(p) if Path::new(p).exists() => {
                        std::fs::remove_file(p).map_err(|e| format!("파일 삭제 실패 ({p}): {e}"))
                    }
                    Some(_) => Ok(()),
                    None => Err("SendTo 파일 경로가 없습니다.".into()),
                }
            }
            EntryKind::Win11Packaged => {
                // MSIX 패키지 내부 파일은 OS 보호 영역이므로 Blocked 키에 등록하여 영구 숨김 처리
                let r = set_single_disabled(entry, true);
                if r.is_ok() {
                    report.notes.push(format!(
                        "[{}] Windows 11 패키지 메뉴는 OS 보호 영역이므로 차단(숨김) 목록에 등록했습니다.",
                        entry.display_name
                    ));
                }
                r
            }
        };

        match res {
            Ok(()) => report.succeeded += 1,
            Err(e) => report.errors.push(format!("[{}] {e}", entry.display_name)),
        }
    }

    Ok((record, report))
}

fn move_file_safe(src: &Path, dst: &Path) -> Result<(), String> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst).map_err(|e| format!("파일 이동(복사) 실패: {e}"))?;
    std::fs::remove_file(src).map_err(|e| format!("원본 파일 제거 실패: {e}"))?;
    Ok(())
}

/// 새 Shell 명령을 등록할 대상 위치
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellTargetKind {
    AllFiles,
    Directory,
    DirectoryBackground,
    AllFilesystemObjects,
    Drive,
    DesktopBackground,
    CustomExtension,
}

impl ShellTargetKind {
    pub const ALL: &'static [ShellTargetKind] = &[
        ShellTargetKind::AllFiles,
        ShellTargetKind::Directory,
        ShellTargetKind::DirectoryBackground,
        ShellTargetKind::AllFilesystemObjects,
        ShellTargetKind::Drive,
        ShellTargetKind::DesktopBackground,
        ShellTargetKind::CustomExtension,
    ];

    /// 기본 명령 인자 자리 표시자 추천 (배경 우클릭은 %V, 파일/폴더는 %1)
    pub fn default_placeholder(self) -> &'static str {
        match self {
            ShellTargetKind::DirectoryBackground | ShellTargetKind::DesktopBackground => "%V",
            _ => "%1",
        }
    }
}

/// 메뉴 표시 위치 (Position 레지스트리 값)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMenuPosition {
    Default,
    Top,
    Bottom,
}

impl ShellMenuPosition {
    pub const ALL: &'static [ShellMenuPosition] = &[
        ShellMenuPosition::Default,
        ShellMenuPosition::Top,
        ShellMenuPosition::Bottom,
    ];
}

/// Shell 명령 등록/편집 입력 명세
#[derive(Debug, Clone)]
pub struct ShellCommandSpec {
    pub root: RegRoot,
    pub target_kind: ShellTargetKind,
    pub custom_extension: String,
    /// 편집 모드에서는 기존 전체 서브키 경로를 그대로 유지
    pub existing_subkey: Option<String>,
    pub key_name: String,
    pub display_name: String,
    pub command: String,
    pub icon: String,
    pub extended: bool,
    pub position: ShellMenuPosition,
    pub overwrite: bool,
}

impl Default for ShellCommandSpec {
    fn default() -> Self {
        Self {
            root: RegRoot::Hkcu,
            target_kind: ShellTargetKind::AllFiles,
            custom_extension: String::new(),
            existing_subkey: None,
            key_name: String::new(),
            display_name: String::new(),
            command: String::new(),
            icon: String::new(),
            extended: false,
            position: ShellMenuPosition::Default,
            overwrite: false,
        }
    }
}

impl ShellCommandSpec {
    /// 실제 레지스트리 키 이름 계산 (직접 입력이 비어 있으면 표시 이름에서 안전한 이름 생성)
    pub fn effective_key_name(&self) -> String {
        let raw = if !self.key_name.trim().is_empty() {
            self.key_name.trim()
        } else {
            self.display_name.trim()
        };
        raw.chars()
            .filter(|&c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
            .collect::<String>()
            .trim()
            .to_string()
    }

    /// 대상 레지스트리 서브키 경로 계산 (예: `Software\Classes\*\shell\mkv2mp4`)
    pub fn target_subkey(&self) -> Result<String, String> {
        if let Some(ref existing) = self.existing_subkey {
            return Ok(existing.clone());
        }
        let classes_base = match self.root {
            RegRoot::Hkcu => "Software\\Classes",
            RegRoot::Hklm => "SOFTWARE\\Classes",
        };
        let target_seg = match self.target_kind {
            ShellTargetKind::AllFiles => "*".to_string(),
            ShellTargetKind::Directory => "Directory".to_string(),
            ShellTargetKind::DirectoryBackground => "Directory\\Background".to_string(),
            ShellTargetKind::AllFilesystemObjects => "AllFilesystemObjects".to_string(),
            ShellTargetKind::Drive => "Drive".to_string(),
            ShellTargetKind::DesktopBackground => "DesktopBackground".to_string(),
            ShellTargetKind::CustomExtension => {
                let ext = self.custom_extension.trim();
                if ext.is_empty() {
                    return Err("extension_empty".into());
                }
                if ext.contains(['\\', '/', ' ']) {
                    return Err("extension_invalid".into());
                }
                if ext.starts_with('.') {
                    ext.to_string()
                } else {
                    format!(".{ext}")
                }
            }
        };
        let key = self.effective_key_name();
        if key.is_empty() {
            return Err("key_empty".into());
        }
        Ok(format!("{classes_base}\\{target_seg}\\shell\\{key}"))
    }

    pub fn preview_full_path(&self) -> Option<String> {
        self.target_subkey()
            .ok()
            .map(|sub| self.root.format_short_path(&sub))
    }

    pub fn validate(&self) -> Result<String, String> {
        if self.display_name.trim().is_empty() {
            return Err("display_name_empty".into());
        }
        if self.command.trim().is_empty() {
            return Err("command_empty".into());
        }
        if self.key_name.contains(['\\', '/']) {
            return Err("key_invalid".into());
        }
        self.target_subkey()
    }
}

/// 기존 Shell 명령 키를 편집용 ShellCommandSpec 으로 로드
pub fn load_shell_command(root: RegRoot, subkey: &str) -> Result<ShellCommandSpec, String> {
    let key = RegKey::open_read(root, subkey)
        .ok_or_else(|| format!("레지스트리 키를 열 수 없습니다: {}", root.format_short_path(subkey)))?;
    let leaf = subkey.rsplit('\\').next().unwrap_or(subkey).to_string();
    let display_name = key
        .read_string("MUIVerb")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| key.read_default_string().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| leaf.clone());
    let command = key
        .open_subkey_read("command")
        .and_then(|c| c.read_default_string())
        .unwrap_or_default();
    let icon = key.read_string("Icon").unwrap_or_default();
    let extended = key.value_exists("Extended");
    let position = match key
        .read_string("Position")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "top" => ShellMenuPosition::Top,
        "bottom" => ShellMenuPosition::Bottom,
        _ => ShellMenuPosition::Default,
    };

    Ok(ShellCommandSpec {
        root,
        target_kind: ShellTargetKind::AllFiles,
        custom_extension: String::new(),
        existing_subkey: Some(subkey.to_string()),
        key_name: leaf,
        display_name,
        command,
        icon,
        extended,
        position,
        overwrite: true,
    })
}

/// Shell 명령을 레지스트리에 새로 등록하거나 덮어쓰기 저장
pub fn save_shell_command(spec: &ShellCommandSpec) -> Result<String, String> {
    let subkey = spec.validate()?;
    save_shell_command_at(spec.root, &subkey, spec)
}

fn save_shell_command_at(root: RegRoot, subkey: &str, spec: &ShellCommandSpec) -> Result<String, String> {
    if !spec.overwrite && RegKey::open_read(root, subkey).is_some() {
        return Err("already_exists".into());
    }

    let key = RegKey::create_write(root, subkey)?;
    key.set_string("", spec.display_name.trim())?;
    // 기존에 MUIVerb 가 있었다면 편집한 표시 이름과 충돌하지 않도록 함께 갱신
    if key.value_exists("MUIVerb") {
        key.set_string("MUIVerb", spec.display_name.trim())?;
    }

    let icon_trimmed = spec.icon.trim();
    if icon_trimmed.is_empty() {
        let _ = key.delete_value("Icon");
    } else {
        key.set_string("Icon", icon_trimmed)?;
    }

    if spec.extended {
        key.set_string("Extended", "")?;
    } else {
        let _ = key.delete_value("Extended");
    }

    match spec.position {
        ShellMenuPosition::Default => {
            let _ = key.delete_value("Position");
        }
        ShellMenuPosition::Top => key.set_string("Position", "Top")?,
        ShellMenuPosition::Bottom => key.set_string("Position", "Bottom")?,
    }

    let cmd_subkey = format!("{subkey}\\command");
    let cmd_key = RegKey::create_write(root, &cmd_subkey)?;
    cmd_key.set_string("", spec.command.trim())?;

    Ok(root.format_short_path(subkey))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Category, Scope, TargetLabel};

    #[test]
    fn shell_verb_toggle_legacy_disable_works() {
        let base = format!("Software\\ContextManager_ActionTest_{}", std::process::id());
        let subkey = format!("{base}\\DemoVerb");
        let _ = reg::delete_tree(RegRoot::Hkcu, &base);
        {
            let k = RegKey::create_write(RegRoot::Hkcu, &subkey).unwrap();
            k.set_string("", "Demo").unwrap();
        }

        let entry = ContextEntry {
            id: "t1".into(),
            display_name: "Demo".into(),
            raw_name: "DemoVerb".into(),
            kind: EntryKind::ShellVerb,
            scope: Scope::User,
            category: Category::Directory,
            target_label: TargetLabel::Directory,
            registry_root: Some(RegRoot::Hkcu),
            registry_subkey: Some(subkey.clone()),
            file_path: None,
            disabled_file_path: None,
            command: None,
            resolved_target: None,
            target_exists: None,
            clsid: None,
            package_name: None,
            icon: None,
            disabled: false,
            is_system: false,
            group_key: "g1".into(),
        };

        let rep = set_entries_disabled(&[&entry], true);
        assert_eq!(rep.succeeded, 1);
        assert!(RegKey::open_read(RegRoot::Hkcu, &subkey).unwrap().value_exists("LegacyDisable"));

        let rep = set_entries_disabled(&[&entry], false);
        assert_eq!(rep.succeeded, 1);
        assert!(!RegKey::open_read(RegRoot::Hkcu, &subkey).unwrap().value_exists("LegacyDisable"));

        let _ = reg::delete_tree(RegRoot::Hkcu, &base);
    }

    #[test]
    fn shell_command_spec_builds_paths_and_validates() {
        let mut spec = ShellCommandSpec {
            root: RegRoot::Hklm,
            target_kind: ShellTargetKind::AllFiles,
            display_name: "mkv2mp4".into(),
            command: "cmd /k c:\\[bat]\\mkv2mp4.bat \"%1\"".into(),
            ..Default::default()
        };
        assert_eq!(
            spec.validate().unwrap(),
            "SOFTWARE\\Classes\\*\\shell\\mkv2mp4"
        );

        spec.root = RegRoot::Hkcu;
        spec.target_kind = ShellTargetKind::CustomExtension;
        spec.custom_extension = "mkv".into();
        spec.key_name = "ConvertMkv".into();
        assert_eq!(
            spec.validate().unwrap(),
            "Software\\Classes\\.mkv\\shell\\ConvertMkv"
        );

        spec.command.clear();
        assert_eq!(spec.validate().unwrap_err(), "command_empty");
    }

    #[test]
    fn shell_command_save_load_overwrite_roundtrip() {
        let base = format!("Software\\ContextManager_AddTest_{}", std::process::id());
        let subkey = format!("{base}\\mkv2mp4");
        let _ = reg::delete_tree(RegRoot::Hkcu, &base);

        let spec = ShellCommandSpec {
            root: RegRoot::Hkcu,
            existing_subkey: Some(subkey.clone()),
            display_name: "mkv2mp4".into(),
            command: "cmd /k c:\\[bat]\\mkv2mp4.bat \"%1\"".into(),
            icon: "C:\\Python313\\python.exe".into(),
            extended: true,
            position: ShellMenuPosition::Top,
            overwrite: false,
            ..Default::default()
        };

        let saved_path = save_shell_command(&spec).unwrap();
        assert!(saved_path.ends_with("\\mkv2mp4"));

        // overwrite=false 일 때는 중복 저장 거부
        assert_eq!(save_shell_command(&spec).unwrap_err(), "already_exists");

        // 로드 후 값 일치 검증
        let mut loaded = load_shell_command(RegRoot::Hkcu, &subkey).unwrap();
        assert_eq!(loaded.display_name, "mkv2mp4");
        assert_eq!(loaded.command, "cmd /k c:\\[bat]\\mkv2mp4.bat \"%1\"");
        assert_eq!(loaded.icon, "C:\\Python313\\python.exe");
        assert!(loaded.extended);
        assert_eq!(loaded.position, ShellMenuPosition::Top);

        // 수정 후 덮어쓰기 검증
        loaded.display_name = "mkv2mp4 (Updated)".into();
        loaded.icon.clear();
        loaded.extended = false;
        loaded.position = ShellMenuPosition::Default;
        save_shell_command(&loaded).unwrap();

        let reloaded = load_shell_command(RegRoot::Hkcu, &subkey).unwrap();
        assert_eq!(reloaded.display_name, "mkv2mp4 (Updated)");
        assert!(reloaded.icon.is_empty());
        assert!(!reloaded.extended);
        assert_eq!(reloaded.position, ShellMenuPosition::Default);

        let _ = reg::delete_tree(RegRoot::Hkcu, &base);
    }
}
