//! 컨텍스트 메뉴 데이터 모델, 필터링 및 앱별 그룹화 로직

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::i18n::Language;
use crate::path_util::normalize_path_key;
use crate::reg::RegRoot;

/// 항목 저장 범위
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    User,
    System,
}

impl Scope {
    pub fn label(self, lang: Language) -> &'static str {
        match self {
            Scope::User => lang.scope_user(),
            Scope::System => lang.scope_system(),
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Scope::User => "HKCU",
            Scope::System => "HKLM",
        }
    }
}

/// 컨텍스트 메뉴 등록 방식
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// ...\shell\<verb>
    ShellVerb,
    /// ...\shellex\ContextMenuHandlers\<name>
    ShellEx,
    /// Windows 11 MSIX / Sparse Package IExplorerCommand
    Win11Packaged,
    /// %APPDATA%\Microsoft\Windows\SendTo
    SendTo,
    /// .ext\ShellNew
    ShellNew,
}

impl EntryKind {
    pub fn label(self, lang: Language) -> &'static str {
        match self {
            EntryKind::ShellVerb => lang.kind_shell_verb(),
            EntryKind::ShellEx => lang.kind_shellex(),
            EntryKind::Win11Packaged => lang.kind_win11(),
            EntryKind::SendTo => lang.kind_sendto(),
            EntryKind::ShellNew => lang.kind_shellnew(),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            EntryKind::ShellVerb => "shell",
            EntryKind::ShellEx => "shellex",
            EntryKind::Win11Packaged => "win11",
            EntryKind::SendTo => "sendto",
            EntryKind::ShellNew => "shellnew",
        }
    }

    /// 변경 사항(숨기기/복원/삭제)을 즉시 반영하기 위해 탐색기 재시작이 필요한 유형인지 여부
    pub fn needs_explorer_restart(self) -> bool {
        matches!(
            self,
            EntryKind::ShellEx | EntryKind::Win11Packaged | EntryKind::ShellNew
        )
    }
}

/// 사용자 기준 우클릭 위치 카테고리
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    All,
    Directory,
    Background,
    AllFiles,
    Drive,
    FileExtension,
    Win11Modern,
    SendTo,
    ShellNew,
}

impl Category {
    pub const SIDEBAR: &'static [Category] = &[
        Category::All,
        Category::Directory,
        Category::Background,
        Category::AllFiles,
        Category::Drive,
        Category::FileExtension,
        Category::Win11Modern,
        Category::SendTo,
        Category::ShellNew,
    ];

    pub fn label(self, lang: Language) -> &'static str {
        match self {
            Category::All => lang.cat_all(),
            Category::Directory => lang.cat_directory(),
            Category::Background => lang.cat_background(),
            Category::AllFiles => lang.cat_all_files(),
            Category::Drive => lang.cat_drive(),
            Category::FileExtension => lang.cat_file_ext(),
            Category::Win11Modern => lang.cat_win11(),
            Category::SendTo => lang.cat_sendto(),
            Category::ShellNew => lang.cat_shellnew(),
        }
    }
}

/// 다국어 동적 표시를 위한 적용 대상 라벨
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetLabel {
    AllFiles,
    AllFilesystemObjects,
    Directory,
    DirectoryBackground,
    FolderVirtual,
    Drive,
    DesktopBackground,
    LibraryFolder,
    LibraryBackground,
    FileAssociation(String),
    SendTo,
    ShellNew(String),
    Custom(String),
}

impl TargetLabel {
    pub fn display(&self, lang: Language) -> String {
        match self {
            TargetLabel::AllFiles => lang.tgt_all_files().to_string(),
            TargetLabel::AllFilesystemObjects => lang.tgt_all_fs().to_string(),
            TargetLabel::Directory => lang.tgt_directory().to_string(),
            TargetLabel::DirectoryBackground => lang.tgt_dir_bg().to_string(),
            TargetLabel::FolderVirtual => lang.tgt_folder_virtual().to_string(),
            TargetLabel::Drive => lang.tgt_drive().to_string(),
            TargetLabel::DesktopBackground => lang.tgt_desktop_bg().to_string(),
            TargetLabel::LibraryFolder => lang.tgt_lib_folder().to_string(),
            TargetLabel::LibraryBackground => lang.tgt_lib_bg().to_string(),
            TargetLabel::FileAssociation(name) => lang.fmt_tgt_file_assoc(name),
            TargetLabel::SendTo => lang.tgt_sendto().to_string(),
            TargetLabel::ShellNew(ext) => lang.fmt_tgt_shellnew(ext),
            TargetLabel::Custom(s) => s.clone(),
        }
    }
}

/// 개별 컨텍스트 메뉴 항목
#[derive(Debug, Clone)]
pub struct ContextEntry {
    pub id: String,
    pub display_name: String,
    pub raw_name: String,
    pub kind: EntryKind,
    pub scope: Scope,
    pub category: Category,
    pub target_label: TargetLabel,
    pub registry_root: Option<RegRoot>,
    pub registry_subkey: Option<String>,
    pub file_path: Option<String>,
    pub disabled_file_path: Option<String>,
    pub command: Option<String>,
    pub resolved_target: Option<String>,
    pub target_exists: Option<bool>,
    pub clsid: Option<String>,
    pub package_name: Option<String>,
    pub icon: Option<String>,
    pub disabled: bool,
    pub is_system: bool,
    pub group_key: String,
}

impl ContextEntry {
    pub fn is_orphaned(&self) -> bool {
        self.target_exists == Some(false)
    }

    pub fn target_text(&self, lang: Language) -> String {
        self.target_label.display(lang)
    }

    pub fn location_display(&self) -> String {
        if let (Some(root), Some(subkey)) = (self.registry_root, &self.registry_subkey) {
            return root.format_short_path(subkey);
        }
        if let Some(p) = &self.file_path {
            return p.clone();
        }
        if let Some(clsid) = &self.clsid {
            return format!("CLSID {clsid}");
        }
        self.id.clone()
    }

    pub fn requires_admin_to_delete(&self) -> bool {
        match self.kind {
            EntryKind::Win11Packaged | EntryKind::SendTo => false,
            EntryKind::ShellVerb | EntryKind::ShellEx | EntryKind::ShellNew => {
                self.registry_root == Some(RegRoot::Hklm)
            }
        }
    }

    pub fn compute_group_key(
        kind: EntryKind,
        display_name: &str,
        resolved_target: Option<&str>,
        clsid: Option<&str>,
        package_name: Option<&str>,
        id: &str,
    ) -> String {
        if kind == EntryKind::SendTo {
            return format!("sendto:{}", id.to_ascii_lowercase());
        }
        if kind == EntryKind::ShellNew {
            return format!("shellnew:{}", id.to_ascii_lowercase());
        }
        if let Some(pkg) = package_name {
            if !pkg.is_empty() {
                return format!("pkg:{}:{}", pkg.to_ascii_lowercase(), display_name.to_ascii_lowercase());
            }
        }
        if let Some(target) = resolved_target {
            let norm = normalize_path_key(target);
            if !norm.is_empty() {
                return format!("exe:{}:{}", norm, display_name.to_ascii_lowercase());
            }
        }
        if let Some(c) = clsid {
            if !c.is_empty() {
                return format!("clsid:{}", c.to_ascii_lowercase());
            }
        }
        format!("name:{}:{}", kind.code(), display_name.to_ascii_lowercase())
    }
}

/// 앱/명령 단위로 묶인 그룹
#[derive(Debug, Clone)]
pub struct EntryGroup {
    pub key: String,
    pub display_name: String,
    pub kind: EntryKind,
    pub resolved_target: Option<String>,
    pub entry_indices: Vec<usize>,
    pub any_orphaned: bool,
    pub all_orphaned: bool,
    pub any_disabled: bool,
    pub all_disabled: bool,
    pub has_user_scope: bool,
    pub has_system_scope: bool,
    pub is_system: bool,
    pub summary_targets: String,
}

impl EntryGroup {
    pub fn scope_label(&self) -> &'static str {
        match (self.has_user_scope, self.has_system_scope) {
            (true, true) => "HKCU+HKLM",
            (true, false) => "HKCU",
            (false, true) => "HKLM",
            (false, false) => "-",
        }
    }
}

/// 목록 필터 조건
#[derive(Debug, Clone)]
pub struct FilterOptions {
    pub category: Category,
    pub search: String,
    pub only_orphaned: bool,
    pub only_disabled: bool,
    pub hide_system: bool,
}

impl Default for FilterOptions {
    fn default() -> Self {
        Self {
            category: Category::All,
            search: String::new(),
            only_orphaned: false,
            only_disabled: false,
            hide_system: true,
        }
    }
}

/// 개별 항목이 필터 조건에 맞는지 검사
pub fn matches_filter(entry: &ContextEntry, filter: &FilterOptions, lang: Language) -> bool {
    if filter.category != Category::All && entry.category != filter.category {
        return false;
    }
    if filter.hide_system && entry.is_system && !entry.is_orphaned() {
        return false;
    }
    if filter.only_orphaned && !entry.is_orphaned() {
        return false;
    }
    if filter.only_disabled && !entry.disabled {
        return false;
    }
    let q = filter.search.trim().to_ascii_lowercase();
    if !q.is_empty() {
        let tgt = entry.target_text(lang).to_ascii_lowercase();
        let hit = entry.display_name.to_ascii_lowercase().contains(&q)
            || entry.raw_name.to_ascii_lowercase().contains(&q)
            || tgt.contains(&q)
            || entry
                .resolved_target
                .as_deref()
                .is_some_and(|s| s.to_ascii_lowercase().contains(&q))
            || entry
                .command
                .as_deref()
                .is_some_and(|s| s.to_ascii_lowercase().contains(&q))
            || entry
                .registry_subkey
                .as_deref()
                .is_some_and(|s| s.to_ascii_lowercase().contains(&q))
            || entry
                .package_name
                .as_deref()
                .is_some_and(|s| s.to_ascii_lowercase().contains(&q));
        if !hit {
            return false;
        }
    }
    true
}

/// 필터를 통과한 항목 인덱스들을 group_key 기준으로 묶고 정렬
pub fn build_groups(entries: &[ContextEntry], filtered_indices: &[usize], lang: Language) -> Vec<EntryGroup> {
    let mut map: HashMap<String, Vec<usize>> = HashMap::new();
    for &idx in filtered_indices {
        map.entry(entries[idx].group_key.clone()).or_default().push(idx);
    }

    let mut groups: Vec<EntryGroup> = map
        .into_iter()
        .map(|(key, indices)| {
            let first = &entries[indices[0]];
            let any_orphaned = indices.iter().any(|&i| entries[i].is_orphaned());
            let all_orphaned = indices.iter().all(|&i| entries[i].is_orphaned());
            let any_disabled = indices.iter().any(|&i| entries[i].disabled);
            let all_disabled = indices.iter().all(|&i| entries[i].disabled);
            let has_user_scope = indices.iter().any(|&i| entries[i].scope == Scope::User);
            let has_system_scope = indices.iter().any(|&i| entries[i].scope == Scope::System);
            let is_system = indices.iter().all(|&i| entries[i].is_system);

            let mut target_set: BTreeSet<String> = BTreeSet::new();
            for &i in &indices {
                target_set.insert(entries[i].target_text(lang));
            }
            let targets: Vec<String> = target_set.into_iter().collect();
            let summary_targets = if targets.len() <= 2 {
                targets.join(", ")
            } else {
                lang.fmt_summary_more(&targets[0], &targets[1], targets.len() - 2)
            };

            EntryGroup {
                key,
                display_name: first.display_name.clone(),
                kind: first.kind,
                resolved_target: first.resolved_target.clone(),
                entry_indices: indices,
                any_orphaned,
                all_orphaned,
                any_disabled,
                all_disabled,
                has_user_scope,
                has_system_scope,
                is_system,
                summary_targets,
            }
        })
        .collect();

    groups.sort_by(|a, b| {
        b.any_orphaned
            .cmp(&a.any_orphaned)
            .then_with(|| a.display_name.to_ascii_lowercase().cmp(&b.display_name.to_ascii_lowercase()))
            .then_with(|| a.key.cmp(&b.key))
    });

    groups
}

/// 현재 체크된 항목들과 같은 프로그램(group_key)에 속하지만 아직 체크되지 않은 다른 위치 항목 인덱스 조회
pub fn find_related_unchecked_indices(entries: &[ContextEntry], checked_ids: &HashSet<String>) -> Vec<usize> {
    if checked_ids.is_empty() {
        return Vec::new();
    }
    let selected_groups: HashSet<&str> = entries
        .iter()
        .filter(|e| checked_ids.contains(&e.id))
        .map(|e| e.group_key.as_str())
        .collect();

    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| selected_groups.contains(e.group_key.as_str()) && !checked_ids.contains(&e.id))
        .map(|(i, _)| i)
        .collect()
}

/// 전체 항목 기준 group_key 별 총 등록 위치 개수 계산
pub fn count_group_totals(entries: &[ContextEntry]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for e in entries {
        *counts.entry(e.group_key.clone()).or_insert(0) += 1;
    }
    counts
}

/// 동일한 CLSID 를 공유하는 다른 그룹의 표시 이름 목록 조회 (예: 제조사 간 샘플 CLSID 중복)
pub fn find_shared_clsid_peers(
    entries: &[ContextEntry],
    clsid: &str,
    exclude_group_key: &str,
) -> Vec<String> {
    let target = clsid.trim();
    if target.is_empty() {
        return Vec::new();
    }
    let mut names: BTreeSet<String> = BTreeSet::new();
    for e in entries {
        if e.group_key == exclude_group_key {
            continue;
        }
        if e.clsid.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(target)) {
            names.insert(e.display_name.clone());
        }
    }
    names.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(
        id: &str,
        name: &str,
        target: TargetLabel,
        exe: &str,
        exists: bool,
        category: Category,
    ) -> ContextEntry {
        let group_key = ContextEntry::compute_group_key(
            EntryKind::ShellVerb,
            name,
            Some(exe),
            None,
            None,
            id,
        );
        ContextEntry {
            id: id.into(),
            display_name: name.into(),
            raw_name: "Cursor".into(),
            kind: EntryKind::ShellVerb,
            scope: Scope::User,
            category,
            target_label: target,
            registry_root: Some(RegRoot::Hkcu),
            registry_subkey: Some(id.into()),
            file_path: None,
            disabled_file_path: None,
            command: Some(format!("\"{exe}\" \"%V\"")),
            resolved_target: Some(exe.into()),
            target_exists: Some(exists),
            clsid: None,
            package_name: None,
            icon: None,
            disabled: false,
            is_system: false,
            group_key,
        }
    }

    #[test]
    fn groups_same_app_across_multiple_locations_and_sorts_orphans_first() {
        let entries = vec![
            sample_entry("k1", "Open with VS Code", TargetLabel::Directory, "C:\\Apps\\Code.exe", true, Category::Directory),
            sample_entry("k2", "Open with Cursor", TargetLabel::Directory, "C:\\Apps\\Cursor.exe", false, Category::Directory),
            sample_entry("k3", "Open with Cursor", TargetLabel::DirectoryBackground, "C:\\Apps\\Cursor.exe", false, Category::Background),
            sample_entry("k4", "Open with Cursor", TargetLabel::Custom(".py".into()), "C:\\Apps\\Cursor.exe", false, Category::FileExtension),
        ];

        let all_idx: Vec<usize> = (0..entries.len()).collect();
        let groups = build_groups(&entries, &all_idx, Language::En);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].display_name, "Open with Cursor");
        assert_eq!(groups[0].entry_indices.len(), 3);
        assert!(groups[0].all_orphaned);
        assert!(groups[0].summary_targets.contains("+1 more"));
        assert_eq!(groups[1].display_name, "Open with VS Code");
    }

    #[test]
    fn filter_supports_orphan_and_search() {
        let entries = vec![
            sample_entry("k1", "Open with Cursor", TargetLabel::Custom(".rs".into()), "C:\\Apps\\Cursor.exe", false, Category::FileExtension),
            sample_entry("k2", "7-Zip", TargetLabel::AllFiles, "C:\\Apps\\7z.exe", true, Category::AllFiles),
        ];
        let f = FilterOptions {
            only_orphaned: true,
            search: "cursor".into(),
            ..Default::default()
        };
        assert!(matches_filter(&entries[0], &f, Language::En));
        assert!(!matches_filter(&entries[1], &f, Language::En));
    }

    #[test]
    fn finds_related_unchecked_entries_across_other_categories() {
        let entries = vec![
            sample_entry("k1", "Open with Cursor", TargetLabel::Directory, "C:\\Apps\\Cursor.exe", false, Category::Directory),
            sample_entry("k2", "Open with Cursor", TargetLabel::DirectoryBackground, "C:\\Apps\\Cursor.exe", false, Category::Background),
            sample_entry("k3", "Open with Cursor", TargetLabel::Custom(".py".into()), "C:\\Apps\\Cursor.exe", false, Category::FileExtension),
            sample_entry("k4", "7-Zip", TargetLabel::AllFiles, "C:\\Apps\\7z.exe", true, Category::AllFiles),
        ];
        let mut checked = HashSet::new();
        checked.insert("k1".to_string());

        let related = find_related_unchecked_indices(&entries, &checked);
        assert_eq!(related, vec![1, 2]);

        let totals = count_group_totals(&entries);
        assert_eq!(totals.get(&entries[0].group_key), Some(&3));
        assert_eq!(totals.get(&entries[3].group_key), Some(&1));
    }

    #[test]
    fn explorer_restart_requirement_matches_entry_kind() {
        assert!(!EntryKind::ShellVerb.needs_explorer_restart());
        assert!(!EntryKind::SendTo.needs_explorer_restart());
        assert!(EntryKind::ShellEx.needs_explorer_restart());
        assert!(EntryKind::Win11Packaged.needs_explorer_restart());
        assert!(EntryKind::ShellNew.needs_explorer_restart());
    }

    #[test]
    fn detects_other_apps_sharing_same_clsid() {
        let mut e1 = sample_entry("w1", "EmEditor (MSI) (GrepEmEditor)", TargetLabel::Directory, "C:\\Em\\ee.dll", true, Category::Win11Modern);
        e1.kind = EntryKind::Win11Packaged;
        e1.clsid = Some("{3282E233-C5D3-4533-9B25-44B8AAAFACFA}".into());
        e1.group_key = "pkg:emeditor:grep".into();

        let mut e2 = sample_entry("w2", "Adobe Acrobat (CreateAdobePDF)", TargetLabel::Custom(".pdf".into()), "C:\\Adobe\\shim.dll", true, Category::Win11Modern);
        e2.kind = EntryKind::Win11Packaged;
        e2.clsid = Some("{3282E233-C5D3-4533-9B25-44B8AAAFACFA}".into());
        e2.group_key = "pkg:adobe:create".into();

        let entries = vec![e1.clone(), e2.clone()];
        let peers = find_shared_clsid_peers(&entries, "{3282E233-C5D3-4533-9B25-44B8AAAFACFA}", &e1.group_key);
        assert_eq!(peers, vec!["Adobe Acrobat (CreateAdobePDF)"]);
    }
}
