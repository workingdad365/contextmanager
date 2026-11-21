//! 컨텍스트 메뉴 데이터 모델, 필터링 및 앱별 그룹화 로직

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::path_util::normalize_path_key;
use crate::reg::RegRoot;

/// 항목 저장 범위
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    User,
    System,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::User => "사용자 (HKCU)",
            Scope::System => "시스템 (HKLM)",
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
    pub fn label(self) -> &'static str {
        match self {
            EntryKind::ShellVerb => "Shell 명령",
            EntryKind::ShellEx => "COM 셸 확장",
            EntryKind::Win11Packaged => "Win11 신형",
            EntryKind::SendTo => "보내기",
            EntryKind::ShellNew => "새로 만들기",
        }
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

    pub fn label(self) -> &'static str {
        match self {
            Category::All => "전체",
            Category::Directory => "폴더",
            Category::Background => "폴더 배경 / 바탕화면",
            Category::AllFiles => "모든 파일",
            Category::Drive => "드라이브",
            Category::FileExtension => "확장자별",
            Category::Win11Modern => "Win11 신형 메뉴",
            Category::SendTo => "보내기 (SendTo)",
            Category::ShellNew => "새로 만들기 (ShellNew)",
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
    pub target_label: String,
    pub registry_root: Option<RegRoot>,
    pub registry_subkey: Option<String>,
    /// SendTo 실제 파일 경로 (비활성화 시 보관 경로가 아니라 원본 파일명 포함)
    pub file_path: Option<String>,
    /// SendTo 비활성화 시 보관 폴더 내 현재 파일 경로
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
    /// 실행 파일 또는 DLL 이 디스크에 존재하지 않는 잔여 항목 여부
    pub fn is_orphaned(&self) -> bool {
        self.target_exists == Some(false)
    }

    /// 사람이 읽을 수 있는 전체 위치 문자열 (레지스트리 경로 또는 파일 경로)
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

    /// HKLM 키를 직접 수정/삭제해야 해서 관리자 권한이 필요한지 여부
    pub fn requires_admin_to_delete(&self) -> bool {
        match self.kind {
            EntryKind::Win11Packaged => false, // HKCU Blocked 등록으로 처리
            EntryKind::SendTo => false,
            EntryKind::ShellVerb | EntryKind::ShellEx | EntryKind::ShellNew => {
                self.registry_root == Some(RegRoot::Hklm)
            }
        }
    }

    /// 비활성화/활성화에 관리자 권한이 필요한지 여부
    pub fn requires_admin_to_toggle(&self) -> bool {
        match self.kind {
            // ShellEx 와 Win11Packaged 는 HKCU Blocked 키를 사용하므로 일반 권한으로 가능
            EntryKind::ShellEx | EntryKind::Win11Packaged | EntryKind::SendTo => false,
            EntryKind::ShellVerb | EntryKind::ShellNew => self.registry_root == Some(RegRoot::Hklm),
        }
    }

    /// 그룹화 기준 키 계산
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
                // 같은 실행 파일 + 같은 표시 이름이면 하나로 묶음 (예: Open with Cursor 의 15개 경로)
                return format!("exe:{}:{}", norm, display_name.to_ascii_lowercase());
            }
        }
        if let Some(c) = clsid {
            if !c.is_empty() {
                return format!("clsid:{}", c.to_ascii_lowercase());
            }
        }
        format!("name:{}:{}", kind.label(), display_name.to_ascii_lowercase())
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
pub fn matches_filter(entry: &ContextEntry, filter: &FilterOptions) -> bool {
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
        let hit = entry.display_name.to_ascii_lowercase().contains(&q)
            || entry.raw_name.to_ascii_lowercase().contains(&q)
            || entry.target_label.to_ascii_lowercase().contains(&q)
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
/// 잔여 항목(파일 없음)이 있는 그룹을 최상단에 배치한 뒤 이름순 정렬
pub fn build_groups(entries: &[ContextEntry], filtered_indices: &[usize]) -> Vec<EntryGroup> {
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

            let mut target_set: BTreeSet<&str> = BTreeSet::new();
            for &i in &indices {
                target_set.insert(entries[i].target_label.as_str());
            }
            let targets: Vec<&str> = target_set.into_iter().collect();
            let summary_targets = if targets.len() <= 2 {
                targets.join(", ")
            } else {
                format!("{}, {} 외 {}곳", targets[0], targets[1], targets.len() - 2)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(id: &str, name: &str, target: &str, exe: &str, exists: bool, category: Category) -> ContextEntry {
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
            target_label: target.into(),
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
            sample_entry("k1", "VS Code로 열기", "폴더", "C:\\Apps\\Code.exe", true, Category::Directory),
            sample_entry("k2", "Open with Cursor", "폴더", "C:\\Apps\\Cursor.exe", false, Category::Directory),
            sample_entry("k3", "Open with Cursor", "폴더 배경", "C:\\Apps\\Cursor.exe", false, Category::Background),
            sample_entry("k4", "Open with Cursor", ".py", "C:\\Apps\\Cursor.exe", false, Category::FileExtension),
        ];

        let all_idx: Vec<usize> = (0..entries.len()).collect();
        let groups = build_groups(&entries, &all_idx);
        assert_eq!(groups.len(), 2);
        // 잔여 항목(Cursor)이 맨 위에 와야 함
        assert_eq!(groups[0].display_name, "Open with Cursor");
        assert_eq!(groups[0].entry_indices.len(), 3);
        assert!(groups[0].all_orphaned);
        assert!(groups[0].summary_targets.contains("외 1곳"));
        assert_eq!(groups[1].display_name, "VS Code로 열기");
    }

    #[test]
    fn filter_supports_orphan_and_search() {
        let entries = vec![
            sample_entry("k1", "Open with Cursor", ".rs", "C:\\Apps\\Cursor.exe", false, Category::FileExtension),
            sample_entry("k2", "7-Zip", "모든 파일", "C:\\Apps\\7z.exe", true, Category::AllFiles),
        ];
        let f = FilterOptions {
            only_orphaned: true,
            search: "cursor".into(),
            ..Default::default()
        };
        assert!(matches_filter(&entries[0], &f));
        assert!(!matches_filter(&entries[1], &f));
    }

    #[test]
    fn finds_related_unchecked_entries_across_other_categories() {
        let entries = vec![
            sample_entry("k1", "Open with Cursor", "폴더", "C:\\Apps\\Cursor.exe", false, Category::Directory),
            sample_entry("k2", "Open with Cursor", "폴더 배경", "C:\\Apps\\Cursor.exe", false, Category::Background),
            sample_entry("k3", "Open with Cursor", ".py", "C:\\Apps\\Cursor.exe", false, Category::FileExtension),
            sample_entry("k4", "7-Zip", "모든 파일", "C:\\Apps\\7z.exe", true, Category::AllFiles),
        ];
        let mut checked = HashSet::new();
        checked.insert("k1".to_string());

        let related = find_related_unchecked_indices(&entries, &checked);
        assert_eq!(related, vec![1, 2]);

        let totals = count_group_totals(&entries);
        assert_eq!(totals.get(&entries[0].group_key), Some(&3));
        assert_eq!(totals.get(&entries[3].group_key), Some(&1));
    }
}
