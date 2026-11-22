//! 삭제 전 자동 백업(.json + .reg) 및 원클릭 복원 관리

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::model::{ContextEntry, EntryKind};
use crate::reg::{self, RegKey, RegKeySnapshot, RegRoot};
use crate::scanner::BLOCKED_SUBKEY;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackupItem {
    Registry {
        root: RegRoot,
        subkey: String,
        display_name: String,
        snapshot: RegKeySnapshot,
    },
    File {
        original_path: String,
        backup_file: String,
        display_name: String,
    },
    Win11Block {
        clsid: String,
        display_name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,
    pub created_at: String,
    pub summary: String,
    pub reg_file: Option<String>,
    pub items: Vec<BackupItem>,
}

/// 기본 백업 저장 폴더 (%LOCALAPPDATA%\ContextManager\backups)
pub fn default_backup_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("ContextManager").join("backups")
}

/// 로컬 시스템 시각을 기반으로 (id, 사람이 읽는 시각 문자열) 생성
fn current_timestamp() -> (String, String) {
    let st: SYSTEMTIME = unsafe { GetLocalTime() };
    let id = format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}_{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    );
    let human = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    );
    (id, human)
}

/// 삭제 대상 항목들의 백업 레코드를 생성하고 디스크에 저장
pub fn create_backup(entries: &[&ContextEntry], backup_dir: &Path) -> Result<BackupRecord, String> {
    std::fs::create_dir_all(backup_dir).map_err(|e| format!("백업 폴더 생성 실패: {e}"))?;

    let (id, created_at) = current_timestamp();
    let files_dir = backup_dir.join(format!("{id}_files"));
    let mut items = Vec::new();
    let mut reg_export_list = Vec::new();

    for (idx, entry) in entries.iter().enumerate() {
        match entry.kind {
            EntryKind::ShellVerb | EntryKind::ShellEx | EntryKind::ShellNew => {
                if let (Some(root), Some(subkey)) = (entry.registry_root, &entry.registry_subkey) {
                    if let Ok(snap) = reg::snapshot_key(root, subkey) {
                        reg_export_list.push((root, subkey.clone(), snap.clone()));
                        items.push(BackupItem::Registry {
                            root,
                            subkey: subkey.clone(),
                            display_name: entry.display_name.clone(),
                            snapshot: snap,
                        });
                    }
                }
            }
            EntryKind::SendTo => {
                let current_path = entry
                    .disabled_file_path
                    .as_deref()
                    .or(entry.file_path.as_deref());
                let original_path = entry.file_path.as_deref();
                if let (Some(cur), Some(orig)) = (current_path, original_path) {
                    let src = Path::new(cur);
                    if src.is_file() {
                        std::fs::create_dir_all(&files_dir)
                            .map_err(|e| format!("백업 파일 폴더 생성 실패: {e}"))?;
                        let fname = src
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("sendto_item");
                        let stored_name = format!("{idx}_{fname}");
                        let dst = files_dir.join(&stored_name);
                        std::fs::copy(src, &dst)
                            .map_err(|e| format!("SendTo 파일 백업 실패 ({cur}): {e}"))?;
                        items.push(BackupItem::File {
                            original_path: orig.to_string(),
                            backup_file: format!("{id}_files/{stored_name}"),
                            display_name: entry.display_name.clone(),
                        });
                    }
                }
            }
            EntryKind::Win11Packaged => {
                if let Some(clsid) = &entry.clsid {
                    items.push(BackupItem::Win11Block {
                        clsid: clsid.clone(),
                        display_name: entry.display_name.clone(),
                    });
                }
            }
        }
    }

    let reg_file = if !reg_export_list.is_empty() {
        let reg_name = format!("{id}.reg");
        let reg_text = reg::format_reg_file(&reg_export_list);
        // Windows regedit 호환성을 위해 UTF-16LE BOM 형식으로 저장
        let mut utf16_bytes = vec![0xFFu8, 0xFEu8];
        for u in reg_text.encode_utf16() {
            utf16_bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(backup_dir.join(&reg_name), utf16_bytes)
            .map_err(|e| format!(".reg 백업 파일 저장 실패: {e}"))?;
        Some(reg_name)
    } else {
        None
    };

    let summary = if entries.is_empty() {
        "항목 없음".to_string()
    } else if entries.len() == 1 {
        entries[0].display_name.clone()
    } else {
        format!("{} 외 {}건", entries[0].display_name, entries.len() - 1)
    };

    let record = BackupRecord {
        id: id.clone(),
        created_at,
        summary,
        reg_file,
        items,
    };

    let json = serde_json::to_string_pretty(&record)
        .map_err(|e| format!("백업 JSON 직렬화 실패: {e}"))?;
    std::fs::write(backup_dir.join(format!("{id}.json")), json)
        .map_err(|e| format!("백업 JSON 저장 실패: {e}"))?;

    Ok(record)
}

/// 저장된 백업 목록을 최신순으로 조회
pub fn list_backups(backup_dir: &Path) -> Vec<BackupRecord> {
    let Ok(rd) = std::fs::read_dir(backup_dir) else {
        return Vec::new();
    };
    let mut list = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(rec) = serde_json::from_str::<BackupRecord>(&text) {
                list.push(rec);
            }
        }
    }
    list.sort_by(|a, b| b.id.cmp(&a.id));
    list
}

/// 백업 레코드 복원 결과
#[derive(Debug, Default)]
pub struct RestoreReport {
    pub succeeded: usize,
    pub errors: Vec<String>,
}

/// 백업 레코드의 모든 항목을 원래 위치로 복원
pub fn restore_backup(record: &BackupRecord, backup_dir: &Path) -> RestoreReport {
    let mut report = RestoreReport::default();
    for item in &record.items {
        let res = match item {
            BackupItem::Registry {
                root,
                subkey,
                snapshot,
                ..
            } => reg::restore_key(*root, subkey, snapshot),
            BackupItem::File {
                original_path,
                backup_file,
                ..
            } => {
                let src = backup_dir.join(backup_file);
                let dst = Path::new(original_path);
                if let Some(parent) = dst.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::copy(&src, dst)
                    .map(|_| ())
                    .map_err(|e| format!("파일 복원 실패 ({original_path}): {e}"))
            }
            BackupItem::Win11Block { clsid, .. } => {
                match RegKey::open_write(RegRoot::Hkcu, BLOCKED_SUBKEY) {
                    Ok(k) => k.delete_value(clsid),
                    Err(_) => Ok(()),
                }
            }
        };
        match res {
            Ok(()) => report.succeeded += 1,
            Err(e) => report.errors.push(e),
        }
    }
    report
}

/// 백업 기록 파일(.json, .reg, 첨부 파일 폴더) 삭제
pub fn delete_backup_record(record: &BackupRecord, backup_dir: &Path) -> Result<(), String> {
    let json_path = backup_dir.join(format!("{}.json", record.id));
    if json_path.exists() {
        std::fs::remove_file(&json_path).map_err(|e| format!("백업 파일 삭제 실패: {e}"))?;
    }
    if let Some(ref rf) = record.reg_file {
        let reg_path = backup_dir.join(rf);
        if reg_path.exists() {
            let _ = std::fs::remove_file(reg_path);
        }
    }
    let files_dir = backup_dir.join(format!("{}_files", record.id));
    if files_dir.exists() {
        let _ = std::fs::remove_dir_all(files_dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Category, Scope, TargetLabel};

    #[test]
    fn backup_delete_and_restore_registry_and_file_roundtrip() {
        let pid = std::process::id();
        let temp_dir = std::env::temp_dir().join(format!("contextmanager_backup_test_{pid}"));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let backup_dir = temp_dir.join("backups");
        let sendto_dir = temp_dir.join("sendto");
        std::fs::create_dir_all(&sendto_dir).unwrap();

        // 1) 테스트용 HKCU 레지스트리 키 생성
        let reg_base = format!("Software\\ContextManager_BackupTest_{pid}");
        let reg_subkey = format!("{reg_base}\\CursorTest");
        let _ = reg::delete_tree(RegRoot::Hkcu, &reg_base);
        {
            let k = RegKey::create_write(RegRoot::Hkcu, &reg_subkey).unwrap();
            k.set_string("", "Open with Cursor").unwrap();
            let c = RegKey::create_write(RegRoot::Hkcu, &format!("{reg_subkey}\\command")).unwrap();
            c.set_string("", "\"C:\\Missing\\Cursor.exe\" \"%V\"").unwrap();
        }

        // 2) 테스트용 SendTo 파일 생성
        let sendto_file = sendto_dir.join("Demo.lnk");
        std::fs::write(&sendto_file, b"dummy shortcut bytes").unwrap();

        let e_reg = ContextEntry {
            id: "e1".into(),
            display_name: "Open with Cursor".into(),
            raw_name: "CursorTest".into(),
            kind: EntryKind::ShellVerb,
            scope: Scope::User,
            category: Category::Directory,
            target_label: TargetLabel::Directory,
            registry_root: Some(RegRoot::Hkcu),
            registry_subkey: Some(reg_subkey.clone()),
            file_path: None,
            disabled_file_path: None,
            command: None,
            resolved_target: None,
            target_exists: Some(false),
            clsid: None,
            clsid_description: None,
            menu_labels: Vec::new(),
            package_name: None,
            icon: None,
            disabled: false,
            extended: false,
            os_hidden: false,
            is_system: false,
            group_key: "g1".into(),
        };
        let e_file = ContextEntry {
            id: "e2".into(),
            display_name: "Demo".into(),
            raw_name: "Demo.lnk".into(),
            kind: EntryKind::SendTo,
            scope: Scope::User,
            category: Category::SendTo,
            target_label: TargetLabel::SendTo,
            registry_root: None,
            registry_subkey: None,
            file_path: Some(sendto_file.to_string_lossy().into_owned()),
            disabled_file_path: None,
            command: None,
            resolved_target: None,
            target_exists: Some(true),
            clsid: None,
            clsid_description: None,
            menu_labels: Vec::new(),
            package_name: None,
            icon: None,
            disabled: false,
            extended: false,
            os_hidden: false,
            is_system: false,
            group_key: "g2".into(),
        };

        // 백업 생성
        let rec = create_backup(&[&e_reg, &e_file], &backup_dir).unwrap();
        assert_eq!(rec.items.len(), 2);
        assert!(rec.reg_file.is_some());
        assert!(backup_dir.join(rec.reg_file.as_ref().unwrap()).exists());

        // 실제 삭제 시뮬레이션
        reg::delete_tree(RegRoot::Hkcu, &reg_subkey).unwrap();
        std::fs::remove_file(&sendto_file).unwrap();
        assert!(RegKey::open_read(RegRoot::Hkcu, &reg_subkey).is_none());
        assert!(!sendto_file.exists());

        // 목록 조회 및 복원
        let listed = list_backups(&backup_dir);
        assert_eq!(listed.len(), 1);
        let rep = restore_backup(&listed[0], &backup_dir);
        assert_eq!(rep.succeeded, 2);
        assert!(rep.errors.is_empty());

        // 복원 확인
        let k = RegKey::open_read(RegRoot::Hkcu, &reg_subkey).unwrap();
        assert_eq!(k.read_default_string().as_deref(), Some("Open with Cursor"));
        assert_eq!(std::fs::read(&sendto_file).unwrap(), b"dummy shortcut bytes");

        // 정리
        let _ = reg::delete_tree(RegRoot::Hkcu, &reg_base);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
