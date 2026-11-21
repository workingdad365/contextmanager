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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Category, Scope};

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
            target_label: "폴더".into(),
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
}
