//! Windows 레지스트리 읽기/쓰기/스냅샷 백업 및 복원 래퍼

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_CREATE_SUB_KEY,
    KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WRITE, REG_BINARY,
    REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE, REG_QWORD, REG_SZ,
    REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW,
    RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, RegRenameKey, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

/// 지원하는 최상위 레지스트리 하이브
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegRoot {
    Hkcu,
    Hklm,
}

impl RegRoot {
    pub fn as_hkey(self) -> HKEY {
        match self {
            RegRoot::Hkcu => HKEY_CURRENT_USER,
            RegRoot::Hklm => HKEY_LOCAL_MACHINE,
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            RegRoot::Hkcu => "HKCU",
            RegRoot::Hklm => "HKLM",
        }
    }

    pub fn full_name(self) -> &'static str {
        match self {
            RegRoot::Hkcu => "HKEY_CURRENT_USER",
            RegRoot::Hklm => "HKEY_LOCAL_MACHINE",
        }
    }

    pub fn format_full_path(self, subkey: &str) -> String {
        if subkey.is_empty() {
            self.full_name().to_string()
        } else {
            format!("{}\\{subkey}", self.full_name())
        }
    }

    pub fn format_short_path(self, subkey: &str) -> String {
        if subkey.is_empty() {
            self.short_name().to_string()
        } else {
            format!("{}\\{subkey}", self.short_name())
        }
    }
}

/// RAII 레지스트리 키 핸들
pub struct RegKey {
    hkey: HKEY,
}

impl RegKey {
    pub fn open_read(root: RegRoot, subkey: &str) -> Option<Self> {
        Self::open_hkey_read(root.as_hkey(), subkey)
    }

    pub fn open_hkcr_read(subkey: &str) -> Option<Self> {
        Self::open_hkey_read(HKEY_CLASSES_ROOT, subkey)
    }

    pub fn open_hkey_read(base: HKEY, subkey: &str) -> Option<Self> {
        let wide = HSTRING::from(subkey);
        let mut hkey = HKEY::default();
        let err = unsafe { RegOpenKeyExW(base, PCWSTR(wide.as_ptr()), Some(0), KEY_READ, &mut hkey) };
        if err == ERROR_SUCCESS {
            Some(Self { hkey })
        } else {
            None
        }
    }

    pub fn open_subkey_read(&self, subkey: &str) -> Option<Self> {
        Self::open_hkey_read(self.hkey, subkey)
    }

    pub fn open_write(root: RegRoot, subkey: &str) -> Result<Self, String> {
        let wide = HSTRING::from(subkey);
        let mut hkey = HKEY::default();
        let err = unsafe {
            RegOpenKeyExW(
                root.as_hkey(),
                PCWSTR(wide.as_ptr()),
                Some(0),
                KEY_READ | KEY_WRITE,
                &mut hkey,
            )
        };
        if err == ERROR_SUCCESS {
            Ok(Self { hkey })
        } else {
            Err(format_win32_err(err.0, &root.format_short_path(subkey)))
        }
    }

    pub fn create_write(root: RegRoot, subkey: &str) -> Result<Self, String> {
        let wide = HSTRING::from(subkey);
        let mut hkey = HKEY::default();
        let err = unsafe {
            RegCreateKeyExW(
                root.as_hkey(),
                PCWSTR(wide.as_ptr()),
                Some(0),
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE | KEY_CREATE_SUB_KEY | KEY_SET_VALUE | KEY_ENUMERATE_SUB_KEYS | KEY_QUERY_VALUE,
                None,
                &mut hkey,
                None,
            )
        };
        if err == ERROR_SUCCESS {
            Ok(Self { hkey })
        } else {
            Err(format_win32_err(err.0, &root.format_short_path(subkey)))
        }
    }

    pub fn enum_subkeys(&self) -> Vec<String> {
        let mut result = Vec::new();
        let mut buf = vec![0u16; 512];
        let mut idx = 0u32;
        loop {
            let mut len = buf.len() as u32;
            let err = unsafe {
                RegEnumKeyExW(
                    self.hkey,
                    idx,
                    Some(PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if err == ERROR_NO_MORE_ITEMS {
                break;
            }
            if err == ERROR_MORE_DATA {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            if err != ERROR_SUCCESS {
                break;
            }
            result.push(String::from_utf16_lossy(&buf[..len as usize]));
            idx += 1;
        }
        result
    }

    pub fn read_string(&self, name: &str) -> Option<String> {
        let (typ, data) = self.read_raw_value(name)?;
        match REG_VALUE_TYPE(typ) {
            REG_SZ | REG_EXPAND_SZ => Some(decode_utf16_sz(&data)),
            _ => None,
        }
    }

    pub fn read_default_string(&self) -> Option<String> {
        self.read_string("")
    }

    pub fn value_exists(&self, name: &str) -> bool {
        let wide = HSTRING::from(name);
        let name_ptr = if name.is_empty() { PCWSTR::null() } else { PCWSTR(wide.as_ptr()) };
        let mut typ = REG_VALUE_TYPE(0);
        let mut len = 0u32;
        let err = unsafe { RegQueryValueExW(self.hkey, name_ptr, None, Some(&mut typ), None, Some(&mut len)) };
        err == ERROR_SUCCESS || err == ERROR_MORE_DATA
    }

    pub fn read_raw_value(&self, name: &str) -> Option<(u32, Vec<u8>)> {
        let wide = HSTRING::from(name);
        let name_ptr = if name.is_empty() { PCWSTR::null() } else { PCWSTR(wide.as_ptr()) };
        let mut typ = REG_VALUE_TYPE(0);
        let mut len = 0u32;
        let err = unsafe { RegQueryValueExW(self.hkey, name_ptr, None, Some(&mut typ), None, Some(&mut len)) };
        if err != ERROR_SUCCESS && err != ERROR_MORE_DATA {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let err = unsafe {
            RegQueryValueExW(
                self.hkey,
                name_ptr,
                None,
                Some(&mut typ),
                if buf.is_empty() { None } else { Some(buf.as_mut_ptr()) },
                Some(&mut len),
            )
        };
        if err != ERROR_SUCCESS {
            return None;
        }
        buf.truncate(len as usize);
        Some((typ.0, buf))
    }

    pub fn enum_values(&self) -> Vec<(String, RegValueData)> {
        let mut result = Vec::new();
        let mut name_buf = vec![0u16; 512];
        let mut data_buf = vec![0u8; 4096];
        let mut idx = 0u32;
        loop {
            let mut name_len = name_buf.len() as u32;
            let mut data_len = data_buf.len() as u32;
            let mut typ = 0u32;
            let err = unsafe {
                RegEnumValueW(
                    self.hkey,
                    idx,
                    Some(PWSTR(name_buf.as_mut_ptr())),
                    &mut name_len,
                    None,
                    Some(&mut typ),
                    Some(data_buf.as_mut_ptr()),
                    Some(&mut data_len),
                )
            };
            if err == ERROR_NO_MORE_ITEMS {
                break;
            }
            if err == ERROR_MORE_DATA {
                if name_len as usize >= name_buf.len() {
                    name_buf.resize(name_len as usize + 64, 0);
                }
                if data_len as usize > data_buf.len() {
                    data_buf.resize(data_len as usize + 256, 0);
                } else {
                    data_buf.resize(data_buf.len() * 2, 0);
                }
                continue;
            }
            if err != ERROR_SUCCESS {
                break;
            }
            let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            let val = RegValueData::from_raw(typ, &data_buf[..data_len as usize]);
            result.push((name, val));
            idx += 1;
        }
        result
    }

    pub fn set_string(&self, name: &str, value: &str) -> Result<(), String> {
        let wide_name = HSTRING::from(name);
        let name_ptr = if name.is_empty() { PCWSTR::null() } else { PCWSTR(wide_name.as_ptr()) };
        let encoded = encode_utf16_sz(value);
        let err = unsafe { RegSetValueExW(self.hkey, name_ptr, Some(0), REG_SZ, Some(&encoded)) };
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format_win32_err(err.0, name))
        }
    }

    pub fn set_value_data(&self, name: &str, value: &RegValueData) -> Result<(), String> {
        let wide_name = HSTRING::from(name);
        let name_ptr = if name.is_empty() { PCWSTR::null() } else { PCWSTR(wide_name.as_ptr()) };
        let (typ, bytes) = value.to_raw();
        let err = unsafe { RegSetValueExW(self.hkey, name_ptr, Some(0), REG_VALUE_TYPE(typ), Some(&bytes)) };
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format_win32_err(err.0, name))
        }
    }

    pub fn delete_value(&self, name: &str) -> Result<(), String> {
        let wide_name = HSTRING::from(name);
        let name_ptr = if name.is_empty() { PCWSTR::null() } else { PCWSTR(wide_name.as_ptr()) };
        let err = unsafe { RegDeleteValueW(self.hkey, name_ptr) };
        if err == ERROR_SUCCESS || err.0 == 2 {
            // ERROR_FILE_NOT_FOUND(2) 는 이미 삭제된 상태로 간주
            Ok(())
        } else {
            Err(format_win32_err(err.0, name))
        }
    }
}

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.hkey);
        }
    }
}

/// 백업/복원용 레지스트리 값 표현
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegValueData {
    Sz(String),
    ExpandSz(String),
    Dword(u32),
    Qword(u64),
    MultiSz(Vec<String>),
    Binary(Vec<u8>),
    Raw { typ: u32, data: Vec<u8> },
}

impl RegValueData {
    pub fn from_raw(typ: u32, data: &[u8]) -> Self {
        match REG_VALUE_TYPE(typ) {
            REG_SZ => Self::Sz(decode_utf16_sz(data)),
            REG_EXPAND_SZ => Self::ExpandSz(decode_utf16_sz(data)),
            REG_DWORD if data.len() == 4 => {
                Self::Dword(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
            }
            REG_QWORD if data.len() == 8 => Self::Qword(u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ])),
            REG_MULTI_SZ => Self::MultiSz(decode_utf16_multi_sz(data)),
            REG_BINARY => Self::Binary(data.to_vec()),
            _ => Self::Raw { typ, data: data.to_vec() },
        }
    }

    pub fn to_raw(&self) -> (u32, Vec<u8>) {
        match self {
            Self::Sz(s) => (REG_SZ.0, encode_utf16_sz(s)),
            Self::ExpandSz(s) => (REG_EXPAND_SZ.0, encode_utf16_sz(s)),
            Self::Dword(v) => (REG_DWORD.0, v.to_le_bytes().to_vec()),
            Self::Qword(v) => (REG_QWORD.0, v.to_le_bytes().to_vec()),
            Self::MultiSz(list) => (REG_MULTI_SZ.0, encode_utf16_multi_sz(list)),
            Self::Binary(b) => (REG_BINARY.0, b.clone()),
            Self::Raw { typ, data } => (*typ, data.clone()),
        }
    }
}

/// 백업/복원용 레지스트리 키 재귀 스냅샷
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegKeySnapshot {
    pub name: String,
    pub values: Vec<(String, RegValueData)>,
    pub subkeys: Vec<RegKeySnapshot>,
}

/// 지정한 레지스트리 키와 모든 하위 키/값을 재귀적으로 스냅샷
pub fn snapshot_key(root: RegRoot, subkey: &str) -> Result<RegKeySnapshot, String> {
    let key = RegKey::open_read(root, subkey)
        .ok_or_else(|| format!("레지스트리 키를 읽을 수 없습니다: {}", root.format_short_path(subkey)))?;
    let name = subkey.rsplit('\\').next().unwrap_or(subkey).to_string();
    Ok(snapshot_open_key(&key, name))
}

fn snapshot_open_key(key: &RegKey, name: String) -> RegKeySnapshot {
    let values = key.enum_values();
    let mut subkeys = Vec::new();
    for sub in key.enum_subkeys() {
        if let Some(child) = key.open_subkey_read(&sub) {
            subkeys.push(snapshot_open_key(&child, sub));
        }
    }
    RegKeySnapshot { name, values, subkeys }
}

/// 스냅샷을 원래 레지스트리 경로에 재귀적으로 복원
pub fn restore_key(root: RegRoot, subkey: &str, snapshot: &RegKeySnapshot) -> Result<(), String> {
    let key = RegKey::create_write(root, subkey)?;
    for (name, val) in &snapshot.values {
        key.set_value_data(name, val)?;
    }
    for child in &snapshot.subkeys {
        let child_path = if subkey.is_empty() {
            child.name.clone()
        } else {
            format!("{subkey}\\{}", child.name)
        };
        restore_key(root, &child_path, child)?;
    }
    Ok(())
}

/// 지정한 레지스트리 키와 모든 하위 키를 완전히 삭제
pub fn delete_tree(root: RegRoot, subkey: &str) -> Result<(), String> {
    if subkey.trim().is_empty() {
        return Err("루트 레지스트리 키는 삭제할 수 없습니다.".into());
    }
    let wide = HSTRING::from(subkey);
    let err = unsafe { RegDeleteTreeW(root.as_hkey(), PCWSTR(wide.as_ptr())) };
    if err == ERROR_SUCCESS || err.0 == 2 {
        Ok(())
    } else {
        Err(format_win32_err(err.0, &root.format_short_path(subkey)))
    }
}

/// 레지스트리 키 이름 변경 (ShellNew 비활성화/복원 등에 사용)
pub fn rename_subkey(root: RegRoot, parent_subkey: &str, old_name: &str, new_name: &str) -> Result<(), String> {
    let parent = RegKey::open_write(root, parent_subkey)?;
    let old_w = HSTRING::from(old_name);
    let new_w = HSTRING::from(new_name);
    let err = unsafe { RegRenameKey(parent.hkey, PCWSTR(old_w.as_ptr()), PCWSTR(new_w.as_ptr())) };
    if err == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format_win32_err(
            err.0,
            &root.format_short_path(&format!("{parent_subkey}\\{old_name}")),
        ))
    }
}

/// 스냅샷을 사람이 직접 더블클릭으로 가져올 수 있는 .reg 파일 형식으로 변환
pub fn format_reg_file(items: &[(RegRoot, String, RegKeySnapshot)]) -> String {
    let mut out = String::from("Windows Registry Editor Version 5.00\r\n");
    for (root, subkey, snap) in items {
        append_reg_section(&mut out, &root.format_full_path(subkey), snap);
    }
    out
}

fn append_reg_section(out: &mut String, full_path: &str, snap: &RegKeySnapshot) {
    out.push_str("\r\n[");
    out.push_str(full_path);
    out.push_str("]\r\n");
    for (name, val) in &snap.values {
        if name.is_empty() {
            out.push('@');
        } else {
            out.push('"');
            out.push_str(&escape_reg_str(name));
            out.push('"');
        }
        out.push('=');
        match val {
            RegValueData::Sz(s) => {
                out.push('"');
                out.push_str(&escape_reg_str(s));
                out.push('"');
            }
            RegValueData::Dword(v) => {
                out.push_str(&format!("dword:{v:08x}"));
            }
            RegValueData::Binary(b) => {
                out.push_str("hex:");
                out.push_str(&format_hex_bytes(b));
            }
            other => {
                let (typ, raw) = other.to_raw();
                out.push_str(&format!("hex({typ:x}):"));
                out.push_str(&format_hex_bytes(&raw));
            }
        }
        out.push_str("\r\n");
    }
    for child in &snap.subkeys {
        let child_path = format!("{full_path}\\{}", child.name);
        append_reg_section(out, &child_path, child);
    }
}

fn escape_reg_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out
}

fn format_hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(",")
}

fn decode_utf16_sz(data: &[u8]) -> String {
    let u16s: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let end = u16s.iter().position(|&c| c == 0).unwrap_or(u16s.len());
    String::from_utf16_lossy(&u16s[..end])
}

fn encode_utf16_sz(s: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((s.len() + 1) * 2);
    for u in s.encode_utf16().chain(std::iter::once(0)) {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    bytes
}

fn decode_utf16_multi_sz(data: &[u8]) -> Vec<String> {
    let u16s: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut list = Vec::new();
    let mut start = 0;
    for (i, &c) in u16s.iter().enumerate() {
        if c == 0 {
            if i == start {
                break;
            }
            list.push(String::from_utf16_lossy(&u16s[start..i]));
            start = i + 1;
        }
    }
    list
}

fn encode_utf16_multi_sz(list: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for s in list {
        for u in s.encode_utf16().chain(std::iter::once(0)) {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

fn format_win32_err(code: u32, target: &str) -> String {
    match code {
        5 => format!("접근이 거부되었습니다 (관리자 권한 필요): {target}"),
        2 => format!("대상을 찾을 수 없습니다: {target}"),
        _ => format!("레지스트리 작업 실패 (오류 코드 {code}): {target}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_roundtrip_sz_and_multi_sz() {
        let s = "Open with Cursor - 한글 테스트";
        let raw = encode_utf16_sz(s);
        assert_eq!(decode_utf16_sz(&raw), s);

        let multi = vec!["first".to_string(), "둘째".to_string()];
        let raw_multi = encode_utf16_multi_sz(&multi);
        assert_eq!(decode_utf16_multi_sz(&raw_multi), multi);
    }

    #[test]
    fn reg_file_export_escapes_strings() {
        let snap = RegKeySnapshot {
            name: "Cursor".into(),
            values: vec![
                ("".into(), RegValueData::Sz("Open with Cursor".into())),
                ("Icon".into(), RegValueData::Sz("C:\\Apps\\Cursor.exe,0".into())),
                ("Flags".into(), RegValueData::Dword(0x10)),
            ],
            subkeys: vec![RegKeySnapshot {
                name: "command".into(),
                values: vec![("".into(), RegValueData::Sz("\"C:\\Apps\\Cursor.exe\" \"%V\"".into()))],
                subkeys: vec![],
            }],
        };
        let text = format_reg_file(&[(RegRoot::Hkcu, "Software\\Classes\\Directory\\shell\\Cursor".into(), snap)]);
        assert!(text.contains("Windows Registry Editor Version 5.00"));
        assert!(text.contains("[HKEY_CURRENT_USER\\Software\\Classes\\Directory\\shell\\Cursor]"));
        assert!(text.contains("@=\"Open with Cursor\""));
        assert!(text.contains("\"Icon\"=\"C:\\\\Apps\\\\Cursor.exe,0\""));
        assert!(text.contains("\"Flags\"=dword:00000010"));
        assert!(text.contains("[HKEY_CURRENT_USER\\Software\\Classes\\Directory\\shell\\Cursor\\command]"));
        assert!(text.contains("@=\"\\\"C:\\\\Apps\\\\Cursor.exe\\\" \\\"%V\\\"\""));
    }

    #[test]
    fn hkcu_temp_key_snapshot_delete_restore_and_rename() {
        let base = format!("Software\\ContextManager_UnitTest_{}", std::process::id());
        let target = format!("{base}\\TargetKey");
        let _ = delete_tree(RegRoot::Hkcu, &base);

        // 테스트용 키 및 하위 키 생성
        {
            let k = RegKey::create_write(RegRoot::Hkcu, &target).unwrap();
            k.set_string("", "테스트 항목").unwrap();
            k.set_value_data("Count", &RegValueData::Dword(42)).unwrap();
            let sub = RegKey::create_write(RegRoot::Hkcu, &format!("{target}\\command")).unwrap();
            sub.set_string("", "\"C:\\NonExistent\\demo.exe\" \"%1\"").unwrap();
        }

        // 스냅샷
        let snap = snapshot_key(RegRoot::Hkcu, &target).unwrap();
        assert_eq!(snap.subkeys.len(), 1);

        // 이름 변경 테스트 (ShellNew 비활성화용)
        rename_subkey(RegRoot::Hkcu, &base, "TargetKey", "TargetKey_Disabled").unwrap();
        assert!(RegKey::open_read(RegRoot::Hkcu, &target).is_none());
        assert!(RegKey::open_read(RegRoot::Hkcu, &format!("{base}\\TargetKey_Disabled")).is_some());
        rename_subkey(RegRoot::Hkcu, &base, "TargetKey_Disabled", "TargetKey").unwrap();

        // 삭제 후 존재하지 않음 확인
        delete_tree(RegRoot::Hkcu, &target).unwrap();
        assert!(RegKey::open_read(RegRoot::Hkcu, &target).is_none());

        // 복원 후 원본 값 일치 확인
        restore_key(RegRoot::Hkcu, &target, &snap).unwrap();
        let restored = snapshot_key(RegRoot::Hkcu, &target).unwrap();
        assert_eq!(snap, restored);

        // 정리
        delete_tree(RegRoot::Hkcu, &base).unwrap();
        assert!(RegKey::open_read(RegRoot::Hkcu, &base).is_none());
    }
}
