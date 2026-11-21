//! 컨텍스트 메뉴 항목 아이콘 추출 (HICON/EXE/DLL/ICO/PNG -> egui 텍스처) 및 비동기 캐시

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::model::{ContextEntry, EntryKind};
use crate::path_util::expand_env;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IconSource {
    /// 레지스트리 Icon 값 (예: "C:\...\app.exe",0 또는 C:\...\res.dll,-128 또는 .ico)
    IconLocation(String),
    /// MSIX / Sparse Package 의 PNG 로고 파일
    PngFile(String),
    /// 실행 파일 / DLL / .lnk 파일 경로
    FilePath(String),
    /// 확장자 기반 파일 유형 아이콘 (예: ".txt")
    FileExtension(String),
}

impl IconSource {
    /// ContextEntry 에서 가장 적합한 아이콘 추출 소스 결정
    pub fn from_entry(entry: &ContextEntry) -> Option<Self> {
        if let Some(ref ic) = entry.icon {
            let trimmed = ic.trim();
            if !trimmed.is_empty() {
                if trimmed.to_ascii_lowercase().ends_with(".png") && Path::new(trimmed).is_file() {
                    return Some(IconSource::PngFile(trimmed.to_string()));
                }
                return Some(IconSource::IconLocation(trimmed.to_string()));
            }
        }
        if entry.kind == EntryKind::SendTo {
            if let Some(p) = entry.disabled_file_path.as_deref().or(entry.file_path.as_deref()) {
                if Path::new(p).exists() {
                    return Some(IconSource::FilePath(p.to_string()));
                }
            }
        }
        if let Some(ref target) = entry.resolved_target {
            if entry.target_exists != Some(false) && !target.trim().is_empty() {
                return Some(IconSource::FilePath(target.clone()));
            }
        }
        if entry.kind == EntryKind::ShellNew && entry.raw_name.starts_with('.') {
            return Some(IconSource::FileExtension(entry.raw_name.clone()));
        }
        None
    }

    pub fn cache_key(&self) -> String {
        match self {
            IconSource::IconLocation(s) => format!("loc:{}", s.to_ascii_lowercase()),
            IconSource::PngFile(s) => format!("png:{}", s.to_ascii_lowercase()),
            IconSource::FilePath(s) => format!("file:{}", s.to_ascii_lowercase()),
            IconSource::FileExtension(s) => format!("ext:{}", s.to_ascii_lowercase()),
        }
    }
}

struct IconRequest {
    key: String,
    source: IconSource,
    fallback_file: Option<String>,
}

struct IconResult {
    key: String,
    rgba: Option<(u32, u32, Vec<u8>)>,
}

enum CacheEntry {
    Pending,
    Ready(Option<egui::TextureHandle>),
}

/// 백그라운드 스레드 1개로 아이콘을 지연 추출하여 egui 텍스처로 캐싱
pub struct IconCache {
    tx: Sender<IconRequest>,
    rx: Receiver<IconResult>,
    map: HashMap<String, CacheEntry>,
}

impl IconCache {
    pub fn new(ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<IconRequest>();
        let (res_tx, res_rx) = mpsc::channel::<IconResult>();

        std::thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                let mut rgba = extract_icon_rgba(&req.source);
                if rgba.is_none() {
                    if let Some(ref fb) = req.fallback_file {
                        rgba = extract_icon_rgba(&IconSource::FilePath(fb.clone()));
                    }
                }
                if res_tx.send(IconResult { key: req.key, rgba }).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });

        Self {
            tx: req_tx,
            rx: res_rx,
            map: HashMap::new(),
        }
    }

    /// 완료된 아이콘 결과를 텍스처로 업로드
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(res) = self.rx.try_recv() {
            let tex = res.rgba.map(|(w, h, pixels)| {
                let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
                ctx.load_texture(
                    format!("icon_{}", res.key),
                    img,
                    egui::TextureOptions::LINEAR,
                )
            });
            self.map.insert(res.key, CacheEntry::Ready(tex));
        }
    }

    /// 현재 항목의 아이콘 텍스처 조회 (캐시에 없으면 비동기 추출 요청)
    pub fn get_for_entry(&mut self, entry: &ContextEntry) -> Option<&egui::TextureHandle> {
        let source = IconSource::from_entry(entry)?;
        let key = source.cache_key();
        if !self.map.contains_key(&key) {
            self.map.insert(key.clone(), CacheEntry::Pending);
            let fallback_file = match &source {
                IconSource::IconLocation(_) | IconSource::PngFile(_) => {
                    if entry.target_exists != Some(false) {
                        entry.resolved_target.clone()
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let _ = self.tx.send(IconRequest {
                key: key.clone(),
                source,
                fallback_file,
            });
        }
        match self.map.get(&key) {
            Some(CacheEntry::Ready(Some(tex))) => Some(tex),
            _ => None,
        }
    }

    pub fn has_pending(&self) -> bool {
        self.map.values().any(|v| matches!(v, CacheEntry::Pending))
    }
}

/// 레지스트리 Icon 문자열 (`"C:\path\app.exe",0` 또는 `C:\...\res.dll,-128`)을 (경로, 인덱스)로 분리
pub fn parse_icon_location(raw: &str) -> Option<(String, i32)> {
    let expanded = expand_env(raw.trim());
    let s = expanded.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let path = rest[..end].trim().to_string();
            let tail = rest[end + 1..].trim();
            let idx = tail
                .strip_prefix(',')
                .and_then(|n| n.trim().parse::<i32>().ok())
                .unwrap_or(0);
            return if path.is_empty() { None } else { Some((path, idx)) };
        }
    }
    if let Some(comma) = s.rfind(',') {
        if let Ok(idx) = s[comma + 1..].trim().parse::<i32>() {
            let path = s[..comma].trim().trim_matches('"').to_string();
            return if path.is_empty() { None } else { Some((path, idx)) };
        }
    }
    let path = s.trim_matches('"').trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some((path, 0))
    }
}

/// IconSource 로부터 RGBA 픽셀 데이터 추출
pub fn extract_icon_rgba(source: &IconSource) -> Option<(u32, u32, Vec<u8>)> {
    match source {
        IconSource::PngFile(path) => {
            let bytes = std::fs::read(path).ok()?;
            let icon = eframe::icon_data::from_png_bytes(&bytes).ok()?;
            Some((icon.width, icon.height, icon.rgba))
        }
        IconSource::IconLocation(raw) => {
            let (path, idx) = parse_icon_location(raw)?;
            let resolved = resolve_icon_file(&path)?;
            if let Some(hicon) = extract_hicon_private(&resolved, idx, 24) {
                return hicon_to_rgba(hicon);
            }
            if let Some(hicon) = sh_get_file_icon(&resolved, false) {
                return hicon_to_rgba(hicon);
            }
            None
        }
        IconSource::FilePath(path) => {
            let resolved = resolve_icon_file(path)?;
            let hicon = sh_get_file_icon(&resolved, false)?;
            hicon_to_rgba(hicon)
        }
        IconSource::FileExtension(ext) => {
            let hicon = sh_get_file_icon(ext, true)?;
            hicon_to_rgba(hicon)
        }
    }
}

fn resolve_icon_file(raw: &str) -> Option<String> {
    let p = Path::new(raw);
    if p.exists() {
        return Some(raw.to_string());
    }
    if !p.is_absolute() {
        if let Ok(windir) = std::env::var("WINDIR") {
            let sys32 = Path::new(&windir).join("System32").join(raw);
            if sys32.exists() {
                return Some(sys32.to_string_lossy().into_owned());
            }
            let win = Path::new(&windir).join(raw);
            if win.exists() {
                return Some(win.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn extract_hicon_private(path: &str, index: i32, size: i32) -> Option<HICON> {
    let utf16: Vec<u16> = path.encode_utf16().collect();
    if utf16.len() >= 260 {
        return None;
    }
    let mut name_buf = [0u16; 260];
    name_buf[..utf16.len()].copy_from_slice(&utf16);
    let mut hicons = [HICON::default(); 1];
    let mut icon_id = 0u32;
    let extracted = unsafe {
        PrivateExtractIconsW(
            &name_buf,
            index,
            size,
            size,
            Some(&mut hicons),
            Some(&mut icon_id),
            0,
        )
    };
    if extracted > 0 && !hicons[0].is_invalid() {
        Some(hicons[0])
    } else {
        None
    }
}

fn sh_get_file_icon(path_or_ext: &str, use_file_attrs: bool) -> Option<HICON> {
    let wide = HSTRING::from(path_or_ext);
    let mut shfi = SHFILEINFOW::default();
    let mut flags = SHGFI_ICON | SHGFI_SMALLICON;
    let attrs = if use_file_attrs {
        flags |= SHGFI_USEFILEATTRIBUTES;
        FILE_ATTRIBUTE_NORMAL
    } else {
        windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0)
    };
    let ret = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            attrs,
            Some(&mut shfi),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if ret != 0 && !shfi.hIcon.is_invalid() {
        Some(shfi.hIcon)
    } else {
        None
    }
}

/// Win32 HICON 을 32bpp RGBA 픽셀 버퍼로 변환하고 HICON 핸들을 해제
fn hicon_to_rgba(hicon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut info = ICONINFO::default();
        if GetIconInfo(hicon, &mut info).is_err() {
            let _ = DestroyIcon(hicon);
            return None;
        }

        let result = (|| -> Option<(u32, u32, Vec<u8>)> {
            let color_bmp = info.hbmColor;
            let mask_bmp = info.hbmMask;
            let target_bmp = if !color_bmp.is_invalid() { color_bmp } else { mask_bmp };
            if target_bmp.is_invalid() {
                return None;
            }

            let mut bm = BITMAP::default();
            if GetObjectW(
                HGDIOBJ(target_bmp.0),
                size_of::<BITMAP>() as i32,
                Some(&mut bm as *mut _ as *mut _),
            ) == 0
            {
                return None;
            }

            let w = bm.bmWidth.max(0) as u32;
            let h = if color_bmp.is_invalid() {
                (bm.bmHeight.max(0) / 2) as u32
            } else {
                bm.bmHeight.max(0) as u32
            };
            if w == 0 || h == 0 || w > 256 || h > 256 {
                return None;
            }

            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }

            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32); // top-down
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;

            let mut pixels = vec![0u8; (w * h * 4) as usize];
            let got = if !color_bmp.is_invalid() {
                GetDIBits(
                    dc,
                    color_bmp,
                    0,
                    h,
                    Some(pixels.as_mut_ptr().cast()),
                    &mut bmi,
                    DIB_RGB_COLORS,
                )
            } else {
                0
            };

            if got == 0 {
                let _ = DeleteDC(dc);
                return None;
            }

            // 32bpp 알파 채널이 모두 0 인 구형 아이콘은 mask 비트맵으로 투명도 복원
            let has_alpha = pixels.chunks_exact(4).any(|px| px[3] != 0);
            if !has_alpha && !mask_bmp.is_invalid() {
                let mut mask_pixels = vec![0u8; (w * h * 4) as usize];
                let got_mask = GetDIBits(
                    dc,
                    mask_bmp,
                    0,
                    h,
                    Some(mask_pixels.as_mut_ptr().cast()),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                if got_mask != 0 {
                    for (px, mpx) in pixels.chunks_exact_mut(4).zip(mask_pixels.chunks_exact(4)) {
                        // 마스크가 검은색(0)이면 불투명, 흰색(255)이면 투명
                        px[3] = if mpx[0] == 0 { 255 } else { 0 };
                    }
                } else {
                    for px in pixels.chunks_exact_mut(4) {
                        px[3] = 255;
                    }
                }
            }

            let _ = DeleteDC(dc);

            // BGRA -> RGBA
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            Some((w, h, pixels))
        })();

        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        }
        let _ = DestroyIcon(hicon);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_icon_locations_with_positive_and_negative_indices() {
        assert_eq!(
            parse_icon_location("\"C:\\Apps\\Tool.exe\",0"),
            Some(("C:\\Apps\\Tool.exe".into(), 0))
        );
        assert_eq!(
            parse_icon_location("C:\\Apps\\emedres.dll,-128"),
            Some(("C:\\Apps\\emedres.dll".into(), -128))
        );
        assert_eq!(
            parse_icon_location("C:\\Apps\\icon.ico"),
            Some(("C:\\Apps\\icon.ico".into(), 0))
        );
        assert_eq!(parse_icon_location("   "), None);
    }

    #[test]
    fn extracts_system_shell32_and_extension_icons() {
        let (w, h, rgba) =
            extract_icon_rgba(&IconSource::IconLocation("%SystemRoot%\\System32\\shell32.dll,0".into()))
                .expect("shell32.dll icon should extract");
        assert!(w >= 16 && h >= 16);
        assert_eq!(rgba.len(), (w * h * 4) as usize);

        let (ew, eh, ergba) =
            extract_icon_rgba(&IconSource::FileExtension(".txt".into())).expect(".txt icon should extract");
        assert!(ew >= 16 && eh >= 16);
        assert_eq!(ergba.len(), (ew * eh * 4) as usize);
    }
}
