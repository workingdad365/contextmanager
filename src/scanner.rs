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
    extract_server_path, is_likely_system_entry, path_exists_safe, resolve_and_check_exists,
    resolve_lnk_target, resolve_mui_string,
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
        self.scan_shell_at(root, scope, &shell_subkey, category, target_label, None, false, 0);
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_shell_at(
        &mut self,
        root: RegRoot,
        scope: Scope,
        shell_subkey: &str,
        category: Category,
        target_label: &TargetLabel,
        parent_display: Option<&str>,
        parent_is_system: bool,
        depth: u8,
    ) -> (Vec<String>, Option<String>) {
        if depth > 2 {
            return (Vec::new(), None);
        }
        let Some(shell_key) = RegKey::open_read(root, shell_subkey) else {
            return (Vec::new(), None);
        };

        let mut collected_labels: Vec<String> = Vec::new();
        let mut first_exe: Option<String> = None;

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
            let extended = verb_key.value_exists("Extended");
            let os_hidden = verb_key.value_exists("HideBasedOnVelocityId");
            let raw_display = verb_key
                .read_string("MUIVerb")
                .filter(|s| !s.trim().is_empty())
                .or_else(|| verb_key.read_default_string().filter(|s| !s.trim().is_empty()))
                .unwrap_or_else(|| verb.clone());
            let own_display = resolve_mui_string(&raw_display);
            let display_name = match parent_display {
                Some(p) => format!("{p} > {own_display}"),
                None => own_display.clone(),
            };

            let icon = verb_key.read_string("Icon").filter(|s| !s.trim().is_empty());

            // 명령줄 / 핸들러 / 하위 메뉴 해석
            let cmd_key = verb_key.open_subkey_read("command");
            let command = cmd_key
                .as_ref()
                .and_then(|c| c.read_default_string())
                .filter(|s| !s.trim().is_empty());
            let delegate_clsid = cmd_key
                .as_ref()
                .and_then(|c| c.read_string("DelegateExecute"))
                .filter(|s| !s.trim().is_empty());
            let handler_clsid = verb_key
                .read_string("ExplorerCommandHandler")
                .filter(|s| !s.trim().is_empty());
            let clsid = handler_clsid
                .or(delegate_clsid)
                .map(|c| normalize_clsid(&c))
                .filter(|c| !c.is_empty());

            let clsid_info = clsid.as_deref().and_then(|c| self.lookup_clsid(c));
            let clsid_description = clsid_info
                .as_ref()
                .and_then(|i| i.name.as_deref())
                .map(resolve_mui_string)
                .filter(|s| !s.is_empty());

            let ext_subcommands_key = verb_key
                .read_string("ExtendedSubCommandsKey")
                .map(|s| s.trim().trim_start_matches('\\').to_string())
                .filter(|s| !s.is_empty());
            let subcommands_val = verb_key.read_string("SubCommands");
            let has_inline_shell = verb_key.open_subkey_read("shell").is_some();
            let has_subcommands =
                ext_subcommands_key.is_some() || subcommands_val.is_some() || has_inline_shell;

            // 실제 실행 파일 / DLL 대상 추출
            let mut extracted_path = command.as_deref().and_then(extract_executable_from_command);

            if extracted_path.is_none() {
                if let Some(srv) = clsid_info.as_ref().and_then(|i| i.server_path.as_deref()) {
                    extracted_path = extract_server_path(srv).or_else(|| Some(srv.to_string()));
                }
            }

            if extracted_path.is_none() {
                if let Some(ref ic) = icon {
                    extracted_path = extract_path_from_icon(ic);
                }
            }

            // 명령, CLSID, 하위 메뉴, 아이콘이 모두 없는 빈 키는 스킵
            if command.is_none() && clsid.is_none() && !has_subcommands && icon.is_none() {
                continue;
            }

            // 캐스케이드 하위 메뉴 스캔 (개별 제어 가능하도록 하위 항목도 각각 등록)
            let preliminary_system =
                parent_is_system || is_likely_system_entry(&verb, extracted_path.as_deref(), None);
            let mut sub_labels: Vec<String> = Vec::new();
            let mut sub_first_exe: Option<String> = None;

            if depth < 2 {
                // 1) 인라인 하위 shell 키 (...\shell\<verb>\shell\*)
                if has_inline_shell {
                    let inline_sub = format!("{verb_subkey}\\shell");
                    let (lbls, exe) = self.scan_shell_at(
                        root,
                        scope,
                        &inline_sub,
                        category,
                        target_label,
                        Some(&display_name),
                        preliminary_system,
                        depth + 1,
                    );
                    sub_labels.extend(lbls);
                    if sub_first_exe.is_none() {
                        sub_first_exe = exe;
                    }
                }

                // 2) ExtendedSubCommandsKey (예: Directory\ContextMenus\PowerShell7x64)
                if let Some(ref ext_rel) = ext_subcommands_key {
                    let candidates: &[(RegRoot, Scope, &str)] = match root {
                        RegRoot::Hkcu => &[
                            (RegRoot::Hkcu, Scope::User, "Software\\Classes"),
                            (RegRoot::Hklm, Scope::System, "SOFTWARE\\Classes"),
                        ],
                        RegRoot::Hklm => &[(RegRoot::Hklm, Scope::System, "SOFTWARE\\Classes")],
                    };
                    for &(sub_root, sub_scope, base) in candidates {
                        let ext_shell = format!("{base}\\{ext_rel}\\shell");
                        if RegKey::open_read(sub_root, &ext_shell).is_some() {
                            let (lbls, exe) = self.scan_shell_at(
                                sub_root,
                                sub_scope,
                                &ext_shell,
                                category,
                                target_label,
                                Some(&display_name),
                                preliminary_system,
                                depth + 1,
                            );
                            sub_labels.extend(lbls);
                            if sub_first_exe.is_none() {
                                sub_first_exe = exe;
                            }
                            break;
                        }
                    }
                }

                // 3) SubCommands (세미콜론으로 구분된 Explorer\CommandStore\shell\<name>)
                if let Some(ref sc_list) = subcommands_val {
                    for part in sc_list.split(';').map(str::trim).filter(|s| !s.is_empty() && *s != "|") {
                        let (lbl, exe) = self.scan_command_store_item(
                            part,
                            category,
                            target_label,
                            &display_name,
                            preliminary_system,
                        );
                        if let Some(l) = lbl {
                            sub_labels.push(l);
                        }
                        if sub_first_exe.is_none() {
                            sub_first_exe = exe;
                        }
                    }
                }
            }

            if extracted_path.is_none() {
                extracted_path = sub_first_exe;
            }

            let (resolved_target, target_exists) = match extracted_path {
                Some(raw) => {
                    let (p, exists) = resolve_and_check_exists(&raw);
                    (Some(p), Some(exists))
                }
                None => (None, None),
            };

            if first_exe.is_none() {
                first_exe = resolved_target.clone();
            }
            collected_labels.push(own_display);

            // 중복 제거된 하위 메뉴 라벨 정리
            let mut menu_labels = Vec::new();
            let mut seen_lbl = HashSet::new();
            for lbl in sub_labels {
                if seen_lbl.insert(lbl.to_ascii_lowercase()) {
                    menu_labels.push(lbl);
                }
            }

            let id = root.format_short_path(&verb_subkey);
            let is_system =
                parent_is_system || is_likely_system_entry(&verb, resolved_target.as_deref(), None);
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
                clsid_description,
                menu_labels,
                package_name: None,
                icon,
                disabled,
                extended,
                os_hidden,
                is_system,
                group_key,
            });
        }

        (collected_labels, first_exe)
    }

    /// SubCommands 에서 참조하는 Explorer\CommandStore\shell\<name> 단일 항목 스캔
    fn scan_command_store_item(
        &mut self,
        cmd_name: &str,
        category: Category,
        target_label: &TargetLabel,
        parent_display: &str,
        parent_is_system: bool,
    ) -> (Option<String>, Option<String>) {
        let store_paths = [
            (
                RegRoot::Hkcu,
                Scope::User,
                format!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\CommandStore\\shell\\{cmd_name}"),
            ),
            (
                RegRoot::Hklm,
                Scope::System,
                format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\CommandStore\\shell\\{cmd_name}"),
            ),
        ];

        for (root, scope, verb_subkey) in store_paths {
            let Some(verb_key) = RegKey::open_read(root, &verb_subkey) else {
                continue;
            };
            let disabled = verb_key.value_exists("LegacyDisable");
            let extended = verb_key.value_exists("Extended");
            let os_hidden = verb_key.value_exists("HideBasedOnVelocityId");
            let raw_display = verb_key
                .read_string("MUIVerb")
                .filter(|s| !s.trim().is_empty())
                .or_else(|| verb_key.read_default_string().filter(|s| !s.trim().is_empty()))
                .unwrap_or_else(|| cmd_name.to_string());
            let own_display = resolve_mui_string(&raw_display);
            let display_name = format!("{parent_display} > {own_display}");
            let icon = verb_key.read_string("Icon").filter(|s| !s.trim().is_empty());
            let cmd_key = verb_key.open_subkey_read("command");
            let command = cmd_key
                .as_ref()
                .and_then(|c| c.read_default_string())
                .filter(|s| !s.trim().is_empty());
            let mut extracted = command.as_deref().and_then(extract_executable_from_command);
            if extracted.is_none() {
                if let Some(ref ic) = icon {
                    extracted = extract_path_from_icon(ic);
                }
            }
            let (resolved_target, target_exists) = match extracted {
                Some(raw) => {
                    let (p, exists) = resolve_and_check_exists(&raw);
                    (Some(p), Some(exists))
                }
                None => (None, None),
            };
            let id = root.format_short_path(&verb_subkey);
            let is_system = parent_is_system
                || is_likely_system_entry(cmd_name, resolved_target.as_deref(), None);
            let group_key = ContextEntry::compute_group_key(
                EntryKind::ShellVerb,
                &display_name,
                resolved_target.as_deref(),
                None,
                None,
                &id,
            );
            let ret_exe = resolved_target.clone();
            self.push_entry(ContextEntry {
                id,
                display_name,
                raw_name: cmd_name.to_string(),
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
                clsid: None,
                clsid_description: None,
                menu_labels: Vec::new(),
                package_name: None,
                icon,
                disabled,
                extended,
                os_hidden,
                is_system,
                group_key,
            });
            return (Some(own_display), ret_exe);
        }
        (None, None)
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
            let clsid_description = info
                .as_ref()
                .and_then(|i| i.name.as_deref())
                .map(resolve_mui_string)
                .filter(|s| !s.is_empty());
            let display_name = if !handler_name.starts_with('{') {
                clean_mnemonic(&handler_name)
            } else if let Some(ref desc) = clsid_description {
                desc.clone()
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
                clsid_description,
                menu_labels: Vec::new(),
                package_name: None,
                icon: None,
                disabled,
                extended: false,
                os_hidden: false,
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
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());
        let windows_apps_dir = Path::new(&program_files).join("WindowsApps");

        for pkg_full_name in repo_key.enum_subkeys() {
            let Some(pkg_key) = repo_key.open_subkey_read(&pkg_full_name) else {
                continue;
            };
            let Some(root_folder) = pkg_key.read_string("PackageRootFolder") else {
                continue;
            };
            let Some(manifest_path) =
                find_package_manifest(&root_folder, &windows_apps_dir, &pkg_full_name)
            else {
                continue;
            };
            let Ok(xml) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            if !xml.contains("FileExplorerContextMenus") {
                continue;
            }

            let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new(&root_folder));
            let pkg_display = pkg_key
                .read_string("DisplayName")
                .map(|s| resolve_mui_string(&s))
                .filter(|s| !s.trim().is_empty() && !s.starts_with("ms-resource:"))
                .or_else(|| {
                    extract_xml_tag_text(&xml, "DisplayName")
                        .filter(|s| !s.starts_with("ms-resource:"))
                })
                .unwrap_or_else(|| short_package_name(&pkg_full_name));

            let pkg_logo = resolve_package_logo(&xml, Path::new(&root_folder), manifest_dir);
            let verbs = parse_manifest_context_menus(&xml);
            for verb in verbs {
                let clsid = normalize_clsid(&verb.clsid);
                if clsid.is_empty() {
                    continue;
                }

                // PackagedCom 에서 실제 DLL 경로 조회 (Sparse Package 는 PackageRootFolder 또는 WindowsApps 하위)
                let dll_rel = find_packaged_com_dll(&pkg_full_name, &clsid);
                let (resolved_target, target_exists) = match dll_rel {
                    Some(rel) => {
                        let p = Path::new(&rel);
                        let full = if p.is_absolute() {
                            PathBuf::from(rel)
                        } else {
                            let under_root = Path::new(&root_folder).join(&rel);
                            if path_exists_safe(&under_root) {
                                under_root
                            } else {
                                manifest_dir.join(&rel)
                            }
                        };
                        let s = full.to_string_lossy().into_owned();
                        let exists = path_exists_safe(&full);
                        (Some(s), Some(exists))
                    }
                    None => {
                        let exists = path_exists_safe(Path::new(&root_folder));
                        (Some(root_folder.clone()), Some(exists))
                    }
                };

                let target_label = match verb.item_type.as_str() {
                    "*" => TargetLabel::AllFiles,
                    "Directory" => TargetLabel::Directory,
                    "Directory\\Background" => TargetLabel::DirectoryBackground,
                    other => TargetLabel::Custom(other.to_string()),
                };

                let display_name =
                    if verb.id.is_empty() || verb.id.eq_ignore_ascii_case(&pkg_display) {
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
                    &display_name,
                    resolved_target.as_deref(),
                    Some(&clsid),
                    Some(&pkg_full_name),
                    &id,
                );

                let clsid_description = self
                    .lookup_clsid(&clsid)
                    .and_then(|i| i.name)
                    .map(|n| resolve_mui_string(&n))
                    .filter(|s| !s.is_empty());

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
                    clsid_description,
                    menu_labels: Vec::new(),
                    package_name: Some(pkg_full_name.clone()),
                    icon: pkg_logo.clone(),
                    disabled,
                    extended: false,
                    os_hidden: false,
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
                clsid_description: None,
                menu_labels: Vec::new(),
                package_name: None,
                icon: None,
                disabled,
                extended: false,
                os_hidden: false,
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
            clsid_description: None,
            menu_labels: Vec::new(),
            package_name: None,
            icon: None,
            disabled,
            extended: false,
            os_hidden: false,
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

/// AppxManifest.xml 에서 Square44x44Logo 또는 <Logo> 경로를 추출해 실제 PNG 파일 경로 탐색
fn resolve_package_logo(xml: &str, root_folder: &Path, manifest_dir: &Path) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(rel) = extract_xml_attr(xml, "Square44x44Logo") {
        candidates.push(rel);
    }
    if let Some(rel) = extract_xml_tag_text(xml, "Logo") {
        candidates.push(rel);
    }
    for rel in candidates {
        for base in [manifest_dir, root_folder] {
            if let Some(found) = find_scaled_asset(base, &rel) {
                return Some(found.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Windows MSIX 리소스 스케일 변형(예: logo.png -> logo.targetsize-24.png / logo.scale-100.png) 탐색
fn find_scaled_asset(base: &Path, rel: &str) -> Option<PathBuf> {
    let clean = rel.trim().replace('/', "\\");
    if clean.is_empty() {
        return None;
    }
    let direct = base.join(&clean);
    if direct.is_file() {
        return Some(direct);
    }
    let rel_path = Path::new(&clean);
    let parent = rel_path.parent().map(|p| base.join(p)).unwrap_or_else(|| base.to_path_buf());
    let stem = rel_path.file_stem()?.to_str()?;
    let ext = rel_path.extension().and_then(|e| e.to_str()).unwrap_or("png");

    for suffix in [
        "targetsize-24",
        "targetsize-32",
        "targetsize-48",
        "scale-100",
        "scale-125",
        "scale-150",
        "scale-200",
    ] {
        let cand = parent.join(format!("{stem}.{suffix}.{ext}"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    // 디렉토리 내 동일 stem 접두사로 시작하는 첫 png (예: targetsize-24_altform-unplated)
    if let Ok(rd) = std::fs::read_dir(&parent) {
        let prefix = format!("{}.", stem.to_ascii_lowercase());
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                let lower = name.to_ascii_lowercase();
                if lower.starts_with(&prefix) && lower.ends_with(".png") {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 패키지 매니페스트 경로 탐색
/// 일반 MSIX 는 PackageRootFolder\AppxManifest.xml, Sparse Package 는 WindowsApps\<pkg>\AppxManifest.xml 에 위치
fn find_package_manifest(
    root_folder: &str,
    windows_apps_dir: &Path,
    pkg_full_name: &str,
) -> Option<PathBuf> {
    let direct = Path::new(root_folder).join("AppxManifest.xml");
    if direct.is_file() {
        return Some(direct);
    }
    let sparse = windows_apps_dir.join(pkg_full_name).join("AppxManifest.xml");
    if sparse.is_file() {
        return Some(sparse);
    }
    None
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
    fn find_package_manifest_falls_back_to_windows_apps_for_sparse_package() {
        let base = std::env::temp_dir().join(format!("cm_sparse_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app_root = base.join("ExternalApp");
        let win_apps = base.join("WindowsApps");
        let pkg = "Vendor.Demo_1.0.0.0_x64__abc";
        std::fs::create_dir_all(&app_root).unwrap();
        std::fs::create_dir_all(win_apps.join(pkg)).unwrap();
        let expected = win_apps.join(pkg).join("AppxManifest.xml");
        std::fs::write(&expected, "<Package/>").unwrap();

        let found = find_package_manifest(&app_root.to_string_lossy(), &win_apps, pkg);
        assert_eq!(found.as_deref(), Some(expected.as_path()));

        let _ = std::fs::remove_dir_all(&base);
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

        let pwsh_hits: Vec<_> = res
            .entries
            .iter()
            .filter(|e| {
                e.display_name.to_ascii_lowercase().contains("powershell 7")
                    || e.raw_name.to_ascii_lowercase().contains("powershell7")
                    || e.raw_name.to_ascii_lowercase().contains("openpwsh")
            })
            .collect();
        println!("--- POWERSHELL 7 MATCHES ({}) ---", pwsh_hits.len());
        for p in pwsh_hits {
            println!(
                "  {} | cat={:?} | sys={} | labels={:?} | loc={}",
                p.display_name,
                p.category,
                p.is_system,
                p.menu_labels,
                p.location_display()
            );
        }
    }

    #[test]
    fn scans_extended_subcommands_key_and_inline_submenus_individually() {
        let pid = std::process::id();
        let classes_rel = format!("ContextManager_SubTest_{pid}");
        let base_under_classes = format!("Software\\Classes\\{classes_rel}");
        let _ = crate::reg::delete_tree(RegRoot::Hkcu, &base_under_classes);

        // 상위 메뉴: ExtendedSubCommandsKey 로 외부 ContextMenus 참조 (Icon 없이도 탐지되어야 함)
        let parent_key = format!("{base_under_classes}\\Dir\\shell\\MyCascading");
        let ext_rel = format!("{classes_rel}\\ContextMenus\\MyCascading");
        let sub1 = format!("Software\\Classes\\{ext_rel}\\shell\\openhere");
        let sub2 = format!("Software\\Classes\\{ext_rel}\\shell\\runadmin");
        {
            let pk = RegKey::create_write(RegRoot::Hkcu, &parent_key).unwrap();
            pk.set_string("MUIVerb", "&PowerShell 7 Demo").unwrap();
            pk.set_string("ExtendedSubCommandsKey", &ext_rel).unwrap();

            let s1 = RegKey::create_write(RegRoot::Hkcu, &sub1).unwrap();
            s1.set_string("MUIVerb", "Open &here").unwrap();
            let s1c = RegKey::create_write(RegRoot::Hkcu, &format!("{sub1}\\command")).unwrap();
            s1c.set_string("", "\"C:\\Windows\\System32\\cmd.exe\" /k").unwrap();

            let s2 = RegKey::create_write(RegRoot::Hkcu, &sub2).unwrap();
            s2.set_string("MUIVerb", "Open here as &Administrator").unwrap();
            let s2c = RegKey::create_write(RegRoot::Hkcu, &format!("{sub2}\\command")).unwrap();
            s2c.set_string("", "\"C:\\NonExistent\\pwsh.exe\" -NoExit").unwrap();
        }

        let mut ctx = ScanContext::new();
        ctx.scan_shell_under(
            RegRoot::Hkcu,
            Scope::User,
            "Software\\Classes",
            &format!("{classes_rel}\\Dir"),
            Category::Directory,
            &TargetLabel::Directory,
        );

        assert_eq!(ctx.entries.len(), 3);
        let parent = ctx
            .entries
            .iter()
            .find(|e| e.display_name == "PowerShell 7 Demo")
            .expect("parent entry");
        assert!(parent.menu_labels.iter().any(|l| l == "Open here"));
        assert!(parent.menu_labels.iter().any(|l| l == "Open here as Administrator"));

        let child1 = ctx
            .entries
            .iter()
            .find(|e| e.display_name == "PowerShell 7 Demo > Open here")
            .expect("child1 entry");
        assert_eq!(child1.registry_subkey.as_deref(), Some(sub1.as_str()));

        let child2 = ctx
            .entries
            .iter()
            .find(|e| e.display_name == "PowerShell 7 Demo > Open here as Administrator")
            .expect("child2 entry");
        assert_eq!(child2.registry_subkey.as_deref(), Some(sub2.as_str()));
        assert_eq!(child2.target_exists, Some(false));

        let _ = crate::reg::delete_tree(RegRoot::Hkcu, &base_under_classes);
    }
}
