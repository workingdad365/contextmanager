//! Windows 탐색기 컨텍스트 메뉴 통합 스캐너
//! - Shell 명령 (HKCU/HKLM Classes, SystemFileAssociations, 확장자/ProgID)
//! - COM 셸 확장 (shellex\ContextMenuHandlers)
//! - Windows 11 신형 메뉴 (MSIX/Sparse Package IExplorerCommand)
//! - 보내기 (SendTo)
//! - 새로 만들기 (ShellNew)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::model::{Category, ContextEntry, EntryKind, Scope, TargetLabel};
use crate::path_util::{
    clean_mnemonic, expand_env, extract_executable_from_command, extract_path_from_icon,
    extract_server_path, is_likely_system_entry, resolve_and_check_exists, resolve_lnk_target,
    resolve_mui_string,
};
use crate::reg::{RegKey, RegRoot};

pub const BLOCKED_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Blocked";

const FIXED_SHELL_TARGETS: &[(&str, Category, fn() -> TargetLabel)] = &[
    ("*", Category::AllFiles, || TargetLabel::AllFiles),
    ("AllFilesystemObjects", Category::AllFiles, || TargetLabel::AllFilesystemObjects),
    ("Directory", Category::Directory, || TargetLabel::Directory),
    ("Directory\\Background", Category::Background, || TargetLabel::DirectoryBackground),
    ("Folder", Category::Directory, || TargetLabel::FolderVirtual),
    ("Drive", Category::Drive, || TargetLabel::Drive),
    ("DesktopBackground", Category::Background, || TargetLabel::DesktopBackground),
    ("LibraryFolder", Category::Directory, || TargetLabel::LibraryFolder),
    ("LibraryFolder\\Background", Category::Background, || TargetLabel::LibraryBackground),
];

const SKIP_CLASS_KEYS: &[&str] = &[
    "*",
    "allfilesystemobjects",
    "directory",
    "folder",
    "drive",
    "desktopbackground",
    "libraryfolder",
    "systemfileassociations",
    "clsid",
    "interface",
    "typelib",
    "appid",
    "packagedcom",
    "activatableclasses",
    "local settings",
    "wow6432node",
    "directshow",
    "media type",
    "installer",
    "component categories",
    "mime",
    "protocols",
    "record",
    "licenses",
    "virtualstore",
];

#[derive(Debug, Clone)]
struct ClsidInfo {
    name: Option<String>,
    server_path: Option<String>,
}

/// 스캔 결과 요약
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub entries: Vec<ContextEntry>,
    pub elapsed_ms: u128,
}

/// 전체 컨텍스트 메뉴 스캔 실행
pub fn scan_all() -> ScanResult {
    let start = std::time::Instant::now();
    let mut ctx = ScanContext::new();

    // 1. Shell 명령 및 COM 셸 확장 스캔 (HKCU, HKLM)
    ctx.scan_classes_root(RegRoot::Hkcu, Scope::User);
    ctx.scan_classes_root(RegRoot::Hklm, Scope::System);

    // 2. Windows 11 신형 컨텍스트 메뉴 (MSIX / Sparse Package)
    ctx.scan_win11_packaged();

    // 3. 보내기 (SendTo)
    ctx.scan_sendto();

    // 4. 새로 만들기 (ShellNew)
    ctx.scan_shell_new(RegRoot::Hkcu, Scope::User);
    ctx.scan_shell_new(RegRoot::Hklm, Scope::System);

    ScanResult {
        entries: ctx.entries,
        elapsed_ms: start.elapsed().as_millis(),
    }
}

struct ScanContext {
    entries: Vec<ContextEntry>,
    seen_ids: HashSet<String>,
    blocked_clsids: HashSet<String>,
    clsid_cache: HashMap<String, Option<ClsidInfo>>,
}

impl ScanContext {
    fn new() -> Self {
        let mut blocked_clsids = HashSet::new();
        for root in [RegRoot::Hkcu, RegRoot::Hklm] {
            if let Some(key) = RegKey::open_read(root, BLOCKED_SUBKEY) {
                for (name, _) in key.enum_values() {
                    let norm = normalize_clsid(&name);
                    if !norm.is_empty() {
                        blocked_clsids.insert(norm);
                    }
                }
            }
        }
        Self {
            entries: Vec::new(),
            seen_ids: HashSet::new(),
            blocked_clsids,
            clsid_cache: HashMap::new(),
        }
    }

    fn push_entry(&mut self, entry: ContextEntry) {
        let id_key = entry.id.to_ascii_lowercase();
        if self.seen_ids.insert(id_key) {
            self.entries.push(entry);
        }
    }

    fn lookup_clsid(&mut self, raw_clsid: &str) -> Option<ClsidInfo> {
        let norm = normalize_clsid(raw_clsid);
        if norm.is_empty() {
            return None;
        }
        if let Some(cached) = self.clsid_cache.get(&norm) {
            return cached.clone();
        }

        let subkey_rel = format!("CLSID\\{norm}");
        let key = RegKey::open_read(RegRoot::Hkcu, &format!("Software\\Classes\\{subkey_rel}"))
            .or_else(|| RegKey::open_read(RegRoot::Hklm, &format!("SOFTWARE\\Classes\\{subkey_rel}")))
            .or_else(|| RegKey::open_hkcr_read(&subkey_rel));

        let info = key.map(|k| {
            let name = k.read_default_string().filter(|s| !s.trim().is_empty());
            let server_path = k
                .open_subkey_read("InprocServer32")
                .and_then(|s| s.read_default_string())
                .or_else(|| k.open_subkey_read("LocalServer32").and_then(|s| s.read_default_string()))
                .filter(|s| !s.trim().is_empty());
            ClsidInfo { name, server_path }
        });

        self.clsid_cache.insert(norm, info.clone());
        info
    }

    fn scan_classes_root(&mut self, root: RegRoot, scope: Scope) {
        let classes_base = match root {
            RegRoot::Hkcu => "Software\\Classes",
            RegRoot::Hklm => "SOFTWARE\\Classes",
        };
        let Some(classes_key) = RegKey::open_read(root, classes_base) else {
            return;
        };

        // A. 고정 탐색기 대상 (폴더, 배경, 모든 파일, 드라이브 등)
        for &(rel, category, label_fn) in FIXED_SHELL_TARGETS {
            let label = label_fn();
            self.scan_shell_under(root, scope, classes_base, rel, category, &label);
            self.scan_shellex_under(root, scope, classes_base, rel, category, &label);
        }

        // B. SystemFileAssociations\<ext 또는 종류>
        if let Some(sfa_key) = classes_key.open_subkey_read("SystemFileAssociations") {
            for name in sfa_key.enum_subkeys() {
                let rel = format!("SystemFileAssociations\\{name}");
                let label = TargetLabel::FileAssociation(name);
                self.scan_shell_under(root, scope, classes_base, &rel, Category::FileExtension, &label);
                self.scan_shellex_under(root, scope, classes_base, &rel, Category::FileExtension, &label);
            }
        }

        // C. 확장자(.ext) 및 ProgID 전체 순회
        for name in classes_key.enum_subkeys() {
            let lower = name.to_ascii_lowercase();
            if SKIP_CLASS_KEYS.contains(&lower.as_str()) {
                continue;
            }
            let Some(item_key) = classes_key.open_subkey_read(&name) else {
                continue;
            };
            // 브라우저/앱 커스텀 URL 프로토콜(예: http, mailto, cursor://)은 탐색기 우클릭 메뉴가 아니므로 제외
            if item_key.value_exists("URL Protocol") {
                continue;
            }
            let has_shell = item_key.open_subkey_read("shell").is_some();
            let has_shellex = item_key.open_subkey_read("shellex\\ContextMenuHandlers").is_some();
            if !has_shell && !has_shellex {
                continue;
            }

            let label_str = if name.starts_with('.') {
                name.clone()
            } else if let Some(desc) = item_key.read_default_string().filter(|s| !s.trim().is_empty()) {
                let clean = resolve_mui_string(&desc);
                if clean.eq_ignore_ascii_case(&name) || clean.len() > 40 {
                    name.clone()
                } else {
                    format!("{name} ({clean})")
                }
            } else {
                name.clone()
            };
            let label = TargetLabel::Custom(label_str);

            if has_shell {
                self.scan_shell_under(root, scope, classes_base, &name, Category::FileExtension, &label);
            }
            if has_shellex {
                self.scan_shellex_under(root, scope, classes_base, &name, Category::FileExtension, &label);
            }
        }
    }

    fn scan_shell_under(
        &mut self,
        root: RegRoot,
        scope: Scope,
        classes_base: &str,
        rel_target: &str,
        category: Category,
        target_label: &TargetLabel,
    ) {
        let shell_subkey = format!("{classes_base}\\{rel_target}\\shell");
        let Some(shell_key) = RegKey::open_read(root, &shell_subkey) else {
            return;
        };

        for verb in shell_key.enum_subkeys() {
            let verb_subkey = format!("{shell_subkey}\\{verb}");
            let Some(verb_key) = shell_key.open_subkey_read(&verb) else {
                continue;
            };

            // 내부 호출 전용 동사는 컨텍스트 메뉴에 나타나지 않으므로 제외
            if verb_key.value_exists("ProgrammaticAccessOnly") && !verb_key.value_exists("LegacyDisable") {
                continue;
            }

            let disabled = verb_key.value_exists("LegacyDisable");
            let raw_display = verb_key
                .read_string("MUIVerb")
                .filter(|s| !s.trim().is_empty())
                .or_else(|| verb_key.read_default_string().filter(|s| !s.trim().is_empty()))
                .unwrap_or_else(|| verb.clone());
            let display_name = resolve_mui_string(&raw_display);

            let icon = verb_key.read_string("Icon").filter(|s| !s.trim().is_empty());

            // 명령줄 / 핸들러 / 하위 메뉴 해석
            let cmd_key = verb_key.open_subkey_read("command");
            let command = cmd_key.as_ref().and_then(|c| c.read_default_string()).filter(|s| !s.trim().is_empty());
            let delegate_clsid = cmd_key
                .as_ref()
                .and_then(|c| c.read_string("DelegateExecute"))
                .filter(|s| !s.trim().is_empty());
            let handler_clsid = verb_key
                .read_string("ExplorerCommandHandler")
                .filter(|s| !s.trim().is_empty());
            let clsid = handler_clsid.or(delegate_clsid).map(|c| normalize_clsid(&c)).filter(|c| !c.is_empty());

            // 실제 실행 파일 / DLL 대상 추출
            let mut extracted_path = command.as_deref().and_then(extract_executable_from_command);

            if extracted_path.is_none() {
                if let Some(ref c) = clsid {
                    if let Some(info) = self.lookup_clsid(c) {
                        if let Some(srv) = info.server_path {
                            extracted_path = extract_server_path(&srv).or(Some(srv));
                        }
                    }
                }
            }

            // 캐스케이드(하위 메뉴) 항목이면 첫 번째 하위 명령 또는 아이콘에서 대상 파일 추출
            if extracted_path.is_none() {
                if let Some(nested) = verb_key.open_subkey_read("shell") {
                    for sub_verb in nested.enum_subkeys() {
                        if let Some(sub_cmd) = nested
                            .open_subkey_read(&format!("{sub_verb}\\command"))
                            .and_then(|c| c.read_default_string())
                        {
                            if let Some(p) = extract_executable_from_command(&sub_cmd) {
                                extracted_path = Some(p);
                                break;
                            }
                        }
                    }
                }
            }

            if extracted_path.is_none() {
                if let Some(ref ic) = icon {
                    extracted_path = extract_path_from_icon(ic);
                }
            }

            // 명령, CLSID, 하위 메뉴, 아이콘이 모두 없는 빈 키는 스킵
            let has_subcommands = verb_key.value_exists("SubCommands") || verb_key.open_subkey_read("shell").is_some();
            if command.is_none() && clsid.is_none() && !has_subcommands && icon.is_none() {
                continue;
            }

            let (resolved_target, target_exists) = match extracted_path {
                Some(raw) => {
                    let (p, exists) = resolve_and_check_exists(&raw);
                    (Some(p), Some(exists))
                }
                None => (None, None),
            };

            let id = root.format_short_path(&verb_subkey);
            let is_system = is_likely_system_entry(&verb, resolved_target.as_deref(), None);
            let group_key = ContextEntry::compute_group_key(
                EntryKind::ShellVerb,
                &display_name,
                resolved_target.as_deref(),
                clsid.as_deref(),
                None,
                &id,
            );

            self.push_entry(ContextEntry {
                id,
                display_name,
                raw_name: verb,
                kind: EntryKind::ShellVerb,
                scope,
                category,
                target_label: target_label.clone(),
                registry_root: Some(root),
                registry_subkey: Some(verb_subkey),
                file_path: None,
                disabled_file_path: None,
                command,
                resolved_target,
                target_exists,
                clsid,
                package_name: None,
                icon,
                disabled,
                is_system,
                group_key,
            });
        }
    }

    fn scan_shellex_under(
        &mut self,
        root: RegRoot,
        scope: Scope,
        classes_base: &str,
        rel_target: &str,
        category: Category,
        target_label: &TargetLabel,
    ) {
        let handlers_subkey = format!("{classes_base}\\{rel_target}\\shellex\\ContextMenuHandlers");
        let Some(handlers_key) = RegKey::open_read(root, &handlers_subkey) else {
            return;
        };

        for handler_name in handlers_key.enum_subkeys() {
            let entry_subkey = format!("{handlers_subkey}\\{handler_name}");
            let Some(hkey) = handlers_key.open_subkey_read(&handler_name) else {
                continue;
            };
            let def = hkey.read_default_string().unwrap_or_default();
            let raw_clsid = if def.trim().starts_with('{') {
                def.trim().to_string()
            } else if handler_name.trim().starts_with('{') {
                handler_name.trim().to_string()
            } else {
                continue;
            };
            let clsid = normalize_clsid(&raw_clsid);
            if clsid.is_empty() {
                continue;
            }

            let info = self.lookup_clsid(&clsid);
            let display_name = if !handler_name.starts_with('{') {
                clean_mnemonic(&handler_name)
            } else if let Some(n) = info.as_ref().and_then(|i| i.name.as_deref()) {
                resolve_mui_string(n)
            } else {
                clsid.clone()
            };

            let raw_server = info.as_ref().and_then(|i| i.server_path.clone());
            let (resolved_target, target_exists) = match raw_server.as_deref() {
                Some(srv) => {
                    let extracted = extract_server_path(srv).unwrap_or_else(|| srv.to_string());
                    let (p, exists) = resolve_and_check_exists(&extracted);
                    (Some(p), Some(exists))
                }
                None => (None, Some(false)), // CLSID 등록이 사라진 깨진 핸들러
            };

            let disabled = self.blocked_clsids.contains(&clsid);
            let id = root.format_short_path(&entry_subkey);
            let is_system = is_likely_system_entry(&handler_name, resolved_target.as_deref(), None);
            let group_key = ContextEntry::compute_group_key(
                EntryKind::ShellEx,
                &display_name,
                resolved_target.as_deref(),
                Some(&clsid),
                None,
                &id,
            );

            self.push_entry(ContextEntry {
                id,
                display_name,
                raw_name: handler_name,
                kind: EntryKind::ShellEx,
                scope,
                category,
                target_label: target_label.clone(),
                registry_root: Some(root),
                registry_subkey: Some(entry_subkey),
                file_path: None,
                disabled_file_path: None,
                command: raw_server,
                resolved_target,
                target_exists,
                clsid: Some(clsid),
                package_name: None,
                icon: None,
                disabled,
                is_system,
                group_key,
            });
        }
    }

    /// Windows 11 신형 컨텍스트 메뉴 (MSIX / Sparse Package IExplorerCommand) 스캔
    fn scan_win11_packaged(&mut self) {
        let repo_key_path = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Packages";
        let Some(repo_key) = RegKey::open_read(RegRoot::Hkcu, repo_key_path) else {
            return;
        };

        for pkg_full_name in repo_key.enum_subkeys() {
            let Some(pkg_key) = repo_key.open_subkey_read(&pkg_full_name) else {
                continue;
            };
            let Some(root_folder) = pkg_key.read_string("PackageRootFolder") else {
                continue;
            };
            let manifest_path = Path::new(&root_folder).join("AppxManifest.xml");
            let Ok(xml) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            if !xml.contains("FileExplorerContextMenus") {
                continue;
            }

            let pkg_display = pkg_key
                .read_string("DisplayName")
                .map(|s| resolve_mui_string(&s))
                .filter(|s| !s.trim().is_empty() && !s.starts_with("ms-resource:"))
                .or_else(|| extract_xml_tag_text(&xml, "DisplayName").filter(|s| !s.starts_with("ms-resource:")))
                .unwrap_or_else(|| short_package_name(&pkg_full_name));

            let verbs = parse_manifest_context_menus(&xml);
            for verb in verbs {
                let clsid = normalize_clsid(&verb.clsid);
                if clsid.is_empty() {
                    continue;
                }

                // PackagedCom 에서 실제 DLL 경로 조회
                let dll_rel = find_packaged_com_dll(&pkg_full_name, &clsid);
                let (resolved_target, target_exists) = match dll_rel {
                    Some(rel) => {
                        let full = if Path::new(&rel).is_absolute() {
                            PathBuf::from(rel)
                        } else {
                            Path::new(&root_folder).join(rel)
                        };
                        let s = full.to_string_lossy().into_owned();
                        let exists = full.exists();
                        (Some(s), Some(exists))
                    }
                    None => {
                        let exists = Path::new(&root_folder).exists();
                        (Some(root_folder.clone()), Some(exists))
                    }
                };

                let target_label = match verb.item_type.as_str() {
                    "*" => TargetLabel::AllFiles,
                    "Directory" => TargetLabel::Directory,
                    "Directory\\Background" => TargetLabel::DirectoryBackground,
                    other => TargetLabel::Custom(other.to_string()),
                };

                let display_name = if verb.id.is_empty() || verb.id.eq_ignore_ascii_case(&pkg_display) {
                    pkg_display.clone()
                } else {
                    format!("{pkg_display} ({})", verb.id)
                };

                let disabled = self.blocked_clsids.contains(&clsid);
                let id = format!("WIN11:{pkg_full_name}:{clsid}:{}", verb.item_type);
                let is_system = is_likely_system_entry(
                    &verb.id,
                    resolved_target.as_deref(),
                    Some(&pkg_full_name),
                );
                let group_key = ContextEntry::compute_group_key(
                    EntryKind::Win11Packaged,
                    &pkg_display,
                    resolved_target.as_deref(),
                    Some(&clsid),
                    Some(&pkg_full_name),
                    &id,
                );

                self.push_entry(ContextEntry {
                    id,
                    display_name,
                    raw_name: if verb.id.is_empty() { clsid.clone() } else { verb.id },
                    kind: EntryKind::Win11Packaged,
                    scope: Scope::User,
                    category: Category::Win11Modern,
                    target_label,
                    registry_root: None,
                    registry_subkey: None,
                    file_path: Some(manifest_path.to_string_lossy().into_owned()),
                    disabled_file_path: None,
                    command: None,
                    resolved_target,
                    target_exists,
                    clsid: Some(clsid),
                    package_name: Some(pkg_full_name.clone()),
                    icon: None,
                    disabled,
                    is_system,
                    group_key,
                });
            }
        }
    }

    /// 보내기 (SendTo) 스캔 (활성 + 비활성화 보관 폴더)
    fn scan_sendto(&mut self) {
        let active_dir = sendto_active_dir();
        let disabled_dir = sendto_disabled_dir();

        if let Some(dir) = &active_dir {
            self.scan_sendto_folder(dir, false, active_dir.as_deref());
        }
        if let Some(dir) = &disabled_dir {
            self.scan_sendto_folder(dir, true, active_dir.as_deref());
        }
    }

    fn scan_sendto_folder(&mut self, dir: &Path, disabled: bool, active_dir: Option<&Path>) {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if file_name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }

            let ext_lower = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(file_name)
                .to_string();

            let is_builtin_sendto = matches!(
                ext_lower.as_str(),
                "desklink" | "mapimail" | "zfsendtotarget" | "mydocs"
            );

            let (resolved_target, target_exists) = if ext_lower == "lnk" {
                match resolve_lnk_target(&path) {
                    Some(target) => {
                        let exists = Path::new(&target).exists();
                        (Some(target), Some(exists))
                    }
                    None => (Some(path.to_string_lossy().into_owned()), Some(true)),
                }
            } else {
                (Some(path.to_string_lossy().into_owned()), Some(true))
            };

            let original_path = if disabled {
                active_dir
                    .map(|ad| ad.join(file_name).to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned())
            } else {
                path.to_string_lossy().into_owned()
            };

            let id = format!("SENDTO:{}", file_name.to_ascii_lowercase());
            let is_system = is_builtin_sendto
                || is_likely_system_entry(&stem, resolved_target.as_deref(), None);
            let group_key = ContextEntry::compute_group_key(
                EntryKind::SendTo,
                &stem,
                resolved_target.as_deref(),
                None,
                None,
                &id,
            );

            self.push_entry(ContextEntry {
                id,
                display_name: stem,
                raw_name: file_name.to_string(),
                kind: EntryKind::SendTo,
                scope: Scope::User,
                category: Category::SendTo,
                target_label: TargetLabel::SendTo,
                registry_root: None,
                registry_subkey: None,
                file_path: Some(original_path),
                disabled_file_path: if disabled {
                    Some(path.to_string_lossy().into_owned())
                } else {
                    None
                },
                command: None,
                resolved_target,
                target_exists,
                clsid: None,
                package_name: None,
                icon: None,
                disabled,
                is_system,
                group_key,
            });
        }
    }

    /// 새로 만들기 (ShellNew / ShellNew_Disabled) 스캔
    fn scan_shell_new(&mut self, root: RegRoot, scope: Scope) {
        let classes_base = match root {
            RegRoot::Hkcu => "Software\\Classes",
            RegRoot::Hklm => "SOFTWARE\\Classes",
        };
        let Some(classes_key) = RegKey::open_read(root, classes_base) else {
            return;
        };

        for ext in classes_key.enum_subkeys() {
            if !ext.starts_with('.') {
                continue;
            }
            let Some(ext_key) = classes_key.open_subkey_read(&ext) else {
                continue;
            };
            let prog_id = ext_key.read_default_string().filter(|s| !s.trim().is_empty());

            // 1) .ext\ShellNew 또는 .ext\ShellNew_Disabled
            for (sub_name, disabled) in [("ShellNew", false), ("ShellNew_Disabled", true)] {
                if let Some(sn_key) = ext_key.open_subkey_read(sub_name) {
                    let subkey_full = format!("{classes_base}\\{ext}\\{sub_name}");
                    self.record_shell_new(
                        root,
                        scope,
                        &ext,
                        prog_id.as_deref(),
                        &subkey_full,
                        &sn_key,
                        disabled,
                    );
                }
            }

            // 2) .ext\<ProgID>\ShellNew 또는 ShellNew_Disabled
            if let Some(ref pid) = prog_id {
                if let Some(pid_sub) = ext_key.open_subkey_read(pid) {
                    for (sub_name, disabled) in [("ShellNew", false), ("ShellNew_Disabled", true)] {
                        if let Some(sn_key) = pid_sub.open_subkey_read(sub_name) {
                            let subkey_full = format!("{classes_base}\\{ext}\\{pid}\\{sub_name}");
                            self.record_shell_new(
                                root,
                                scope,
                                &ext,
                                Some(pid.as_str()),
                                &subkey_full,
                                &sn_key,
                                disabled,
                            );
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_shell_new(
        &mut self,
        root: RegRoot,
        scope: Scope,
        ext: &str,
        prog_id: Option<&str>,
        subkey_full: &str,
        sn_key: &RegKey,
        disabled: bool,
    ) {
        let has_trigger = sn_key.value_exists("NullFile")
            || sn_key.value_exists("FileName")
            || sn_key.value_exists("Data")
            || sn_key.value_exists("Command")
            || sn_key.value_exists("Handler");
        if !has_trigger {
            return;
        }

        let item_name = sn_key
            .read_string("ItemName")
            .filter(|s| !s.trim().is_empty())
            .map(|s| resolve_mui_string(&s));

        let prog_desc = prog_id.and_then(|pid| {
            RegKey::open_hkcr_read(pid).and_then(|k| {
                k.read_string("FriendlyTypeName")
                    .or_else(|| k.read_default_string())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| resolve_mui_string(&s))
            })
        });

        let label_base = item_name
            .or(prog_desc)
            .unwrap_or_else(|| format!("{ext} 파일"));
        let display_name = if label_base.contains(ext) {
            label_base
        } else {
            format!("{label_base} ({ext})")
        };

        let command = sn_key.read_string("Command").filter(|s| !s.trim().is_empty());
        let file_name = sn_key.read_string("FileName").filter(|s| !s.trim().is_empty());

        let (resolved_target, target_exists) = if let Some(ref cmd) = command {
            match extract_executable_from_command(cmd) {
                Some(p) => {
                    let (rp, exists) = resolve_and_check_exists(&p);
                    (Some(rp), Some(exists))
                }
                None => (None, None),
            }
        } else if let Some(ref fname) = file_name {
            let expanded = expand_env(fname);
            let p = Path::new(&expanded);
            if p.is_absolute() {
                let exists = p.exists();
                (Some(expanded), Some(exists))
            } else {
                let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
                let shellnew_path = Path::new(&windir).join("ShellNew").join(&expanded);
                let exists = shellnew_path.exists();
                (Some(shellnew_path.to_string_lossy().into_owned()), Some(exists))
            }
        } else {
            (None, None)
        };

        let is_builtin_ext = matches!(
            ext.to_ascii_lowercase().as_str(),
            ".txt" | ".bmp" | ".rtf" | ".zip" | ".lnk" | ".contact" | ".library-ms" | ".folder"
        );
        let is_system = is_builtin_ext
            || is_likely_system_entry(ext, resolved_target.as_deref(), None);

        let id = root.format_short_path(subkey_full);
        let group_key = ContextEntry::compute_group_key(
            EntryKind::ShellNew,
            &display_name,
            resolved_target.as_deref(),
            None,
            None,
            &id,
        );

        self.push_entry(ContextEntry {
            id,
            display_name,
            raw_name: ext.to_string(),
            kind: EntryKind::ShellNew,
            scope,
            category: Category::ShellNew,
            target_label: TargetLabel::ShellNew(ext.to_string()),
            registry_root: Some(root),
            registry_subkey: Some(subkey_full.to_string()),
            file_path: None,
            disabled_file_path: None,
            command: command.or(file_name),
            resolved_target,
            target_exists,
            clsid: None,
            package_name: None,
            icon: None,
            disabled,
            is_system,
            group_key,
        });
    }
}

pub fn sendto_active_dir() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(PathBuf::from(appdata).join("Microsoft").join("Windows").join("SendTo"))
}

pub fn sendto_disabled_dir() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    Some(PathBuf::from(local).join("ContextManager").join("disabled_sendto"))
}

/// CLSID 를 `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` 형태로 정규화
pub fn normalize_clsid(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let inner = trimmed.trim_start_matches('{').trim_end_matches('}');
    // 기본 GUID 길이(36) 및 하이픈 형식 확인
    if inner.len() == 36 && inner.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        format!("{{{}}}", inner.to_ascii_uppercase())
    } else {
        String::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestVerb {
    item_type: String,
    id: String,
    clsid: String,
}

/// AppxManifest.xml 에서 FileExplorerContextMenus 의 ItemType 및 Verb 항목을 추출
fn parse_manifest_context_menus(xml: &str) -> Vec<ManifestVerb> {
    let mut results = Vec::new();
    let mut search = xml;

    while let Some(start) = search.find("FileExplorerContextMenus>") {
        let after = &search[start + "FileExplorerContextMenus>".len()..];
        let Some(end) = after.find("FileExplorerContextMenus>") else {
            break;
        };
        let block = &after[..end];
        parse_item_types_block(block, &mut results);
        search = &after[end + "FileExplorerContextMenus>".len()..];
    }

    results
}

fn parse_item_types_block(block: &str, out: &mut Vec<ManifestVerb>) {
    let mut current_type = "*".to_string();
    let mut rest = block;

    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        let Some(gt) = rest.find('>') else {
            break;
        };
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];

        let tag_trim = tag.trim();
        if tag_trim.starts_with('/') || tag_trim.starts_with('!') || tag_trim.starts_with('?') {
            continue;
        }

        let tag_name = tag_trim.split_whitespace().next().unwrap_or("");
        let local_name = tag_name.rsplit(':').next().unwrap_or(tag_name);

        if local_name.eq_ignore_ascii_case("ItemType") {
            if let Some(t) = extract_xml_attr(tag_trim, "Type") {
                current_type = t;
            }
        } else if local_name.eq_ignore_ascii_case("Verb") {
            if let Some(clsid) = extract_xml_attr(tag_trim, "Clsid") {
                let id = extract_xml_attr(tag_trim, "Id").unwrap_or_default();
                out.push(ManifestVerb {
                    item_type: current_type.clone(),
                    id,
                    clsid,
                });
            }
        }
    }
}

fn extract_xml_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let pos = tag.find(&needle)?;
    let after = tag[pos + needle.len()..].trim_start();
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn extract_xml_tag_text(xml: &str, tag_name: &str) -> Option<String> {
    let open = format!("<{tag_name}>");
    let close = format!("</{tag_name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    let text = xml[start..start + end].trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn short_package_name(pkg_full_name: &str) -> String {
    pkg_full_name.split('_').next().unwrap_or(pkg_full_name).to_string()
}

fn find_packaged_com_dll(pkg_full_name: &str, clsid: &str) -> Option<String> {
    let rel = format!("PackagedCom\\Package\\{pkg_full_name}\\Class\\{clsid}");
    for (root, base) in [
        (RegRoot::Hkcu, "Software\\Classes"),
        (RegRoot::Hklm, "SOFTWARE\\Classes"),
    ] {
        if let Some(k) = RegKey::open_read(root, &format!("{base}\\{rel}")) {
            if let Some(dll) = k.read_string("DllPath").filter(|s| !s.trim().is_empty()) {
                return Some(dll);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_clsid_handles_braces_and_case() {
        assert_eq!(
            normalize_clsid("9f156764-7844-4dc4-b2b1-901f640f5155"),
            "{9F156764-7844-4DC4-B2B1-901F640F5155}"
        );
        assert_eq!(
            normalize_clsid("{9F156764-7844-4DC4-B2B1-901F640F5155}"),
            "{9F156764-7844-4DC4-B2B1-901F640F5155}"
        );
        assert_eq!(normalize_clsid("not-a-guid"), "");
    }

    #[test]
    fn parse_manifest_extracts_win11_context_menu_verbs() {
        let xml = r#"
        <Package>
          <Properties><DisplayName>Windows Terminal</DisplayName></Properties>
          <Extensions>
            <desktop4:Extension Category="windows.fileExplorerContextMenus">
              <desktop4:FileExplorerContextMenus>
                <desktop5:ItemType Type="Directory">
                  <desktop5:Verb Id="OpenInTerminal" Clsid="9F156764-7844-4DC4-B2B1-901F640F5155" />
                </desktop5:ItemType>
                <desktop5:ItemType Type="Directory\Background">
                  <desktop5:Verb Id="OpenInTerminalBg" Clsid="9F156764-7844-4DC4-B2B1-901F640F5155" />
                </desktop5:ItemType>
              </desktop4:FileExplorerContextMenus>
            </desktop4:Extension>
          </Extensions>
        </Package>
        "#;
        let verbs = parse_manifest_context_menus(xml);
        assert_eq!(verbs.len(), 2);
        assert_eq!(verbs[0].item_type, "Directory");
        assert_eq!(verbs[0].id, "OpenInTerminal");
        assert_eq!(verbs[0].clsid, "9F156764-7844-4DC4-B2B1-901F640F5155");
        assert_eq!(verbs[1].item_type, "Directory\\Background");
    }

    #[test]
    #[ignore]
    fn live_scan_diagnostics() {
        let res = scan_all();
        println!("=== LIVE SCAN: {} entries in {} ms ===", res.entries.len(), res.elapsed_ms);
        let lang = crate::i18n::Language::En;
        for &cat in Category::SIDEBAR {
            if cat == Category::All {
                continue;
            }
            let total = res.entries.iter().filter(|e| e.category == cat).count();
            let orphan = res.entries.iter().filter(|e| e.category == cat && e.is_orphaned()).count();
            println!("  {:24}: total={:4}, orphan={:3}", cat.label(lang), total, orphan);
        }

        let orphans: Vec<_> = res.entries.iter().filter(|e| e.is_orphaned()).collect();
        println!("--- ORPHANED ENTRIES ({}) ---", orphans.len());
        let orphan_idx: Vec<usize> = res
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_orphaned())
            .map(|(i, _)| i)
            .collect();
        let groups = crate::model::build_groups(&res.entries, &orphan_idx, lang);
        for g in groups.iter().take(20) {
            println!(
                "  [GROUP] {} | kind={} | scope={} | count={} | targets={} | exe={:?}",
                g.display_name,
                g.kind.label(lang),
                g.scope_label(),
                g.entry_indices.len(),
                g.summary_targets,
                g.resolved_target
            );
        }

        let cursor_hits: Vec<_> = res
            .entries
            .iter()
            .filter(|e| {
                e.display_name.to_ascii_lowercase().contains("cursor")
                    || e.raw_name.to_ascii_lowercase().contains("cursor")
                    || e.resolved_target
                        .as_deref()
                        .is_some_and(|s| s.to_ascii_lowercase().contains("cursor"))
            })
            .collect();
        println!("--- CURSOR MATCHES ({}) ---", cursor_hits.len());
        let non_ext: Vec<_> = cursor_hits.iter().filter(|c| c.category != Category::FileExtension).collect();
        println!("  (non-extension cursor hits: {})", non_ext.len());
        for c in non_ext {
            println!(
                "  [NON-EXT] {} | kind={} | cat={:?} | target={} | exists={:?} | loc={} | exe={:?}",
                c.display_name,
                c.kind.label(lang),
                c.category,
                c.target_text(lang),
                c.target_exists,
                c.location_display(),
                c.resolved_target
            );
        }
        for c in cursor_hits.iter().take(10) {
            println!(
                "  {} | kind={} | cat={:?} | target={} | exists={:?} | loc={} | exe={:?}",
                c.display_name,
                c.kind.label(lang),
                c.category,
                c.target_text(lang),
                c.target_exists,
                c.location_display(),
                c.resolved_target
            );
        }
    }
}
