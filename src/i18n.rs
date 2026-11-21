//! 다국어 지원 (영어 기본, 한국어, 중국어 간체, 일본어)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Ko,
    Zh,
    Ja,
}

impl Language {
    pub const ALL: &'static [Language] = &[Language::En, Language::Ko, Language::Zh, Language::Ja];

    pub fn native_name(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::Ko => "한국어",
            Language::Zh => "中文(简体)",
            Language::Ja => "日本語",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "english" => Some(Language::En),
            "ko" | "kr" | "korean" => Some(Language::Ko),
            "zh" | "cn" | "zh-cn" | "chinese" => Some(Language::Zh),
            "ja" | "jp" | "japanese" => Some(Language::Ja),
            _ => None,
        }
    }

    fn pick<'a>(self, en: &'a str, ko: &'a str, zh: &'a str, ja: &'a str) -> &'a str {
        match self {
            Language::En => en,
            Language::Ko => ko,
            Language::Zh => zh,
            Language::Ja => ja,
        }
    }

    // --- Scope / Kind / Category ---
    pub fn scope_user(self) -> &'static str {
        self.pick("User (HKCU)", "사용자 (HKCU)", "用户 (HKCU)", "ユーザー (HKCU)")
    }
    pub fn scope_system(self) -> &'static str {
        self.pick("System (HKLM)", "시스템 (HKLM)", "系统 (HKLM)", "システム (HKLM)")
    }
    pub fn kind_shell_verb(self) -> &'static str {
        self.pick("Shell Verb", "Shell 명령", "Shell 命令", "Shell コマンド")
    }
    pub fn kind_shellex(self) -> &'static str {
        self.pick("COM ShellEx", "COM 셸 확장", "COM 外壳扩展", "COM シェル拡張")
    }
    pub fn kind_win11(self) -> &'static str {
        self.pick("Win11 Modern", "Win11 신형", "Win11 新版", "Win11 モダン")
    }
    pub fn kind_sendto(self) -> &'static str {
        self.pick("Send To", "보내기", "发送到", "送る")
    }
    pub fn kind_shellnew(self) -> &'static str {
        self.pick("New Menu", "새로 만들기", "新建", "新規作成")
    }

    pub fn cat_all(self) -> &'static str {
        self.pick("All", "전체", "全部", "すべて")
    }
    pub fn cat_directory(self) -> &'static str {
        self.pick("Folders", "폴더", "文件夹", "フォルダー")
    }
    pub fn cat_background(self) -> &'static str {
        self.pick("Folder / Desktop BG", "폴더 배경 / 바탕화면", "文件夹背景 / 桌面", "フォルダー背景 / デスクトップ")
    }
    pub fn cat_all_files(self) -> &'static str {
        self.pick("All Files", "모든 파일", "所有文件", "すべてのファイル")
    }
    pub fn cat_drive(self) -> &'static str {
        self.pick("Drives", "드라이브", "驱动器", "ドライブ")
    }
    pub fn cat_file_ext(self) -> &'static str {
        self.pick("File Extensions", "확장자별", "按扩展名", "拡張子別")
    }
    pub fn cat_win11(self) -> &'static str {
        self.pick("Win11 Modern Menu", "Win11 신형 메뉴", "Win11 新版菜单", "Win11 モダンメニュー")
    }
    pub fn cat_sendto(self) -> &'static str {
        self.pick("Send To (SendTo)", "보내기 (SendTo)", "发送到 (SendTo)", "送る (SendTo)")
    }
    pub fn cat_shellnew(self) -> &'static str {
        self.pick("New (ShellNew)", "새로 만들기 (ShellNew)", "新建 (ShellNew)", "新規作成 (ShellNew)")
    }

    // --- TargetLabel ---
    pub fn tgt_all_files(self) -> &'static str {
        self.pick("All Files (*)", "모든 파일 (*)", "所有文件 (*)", "すべてのファイル (*)")
    }
    pub fn tgt_all_fs(self) -> &'static str {
        self.pick("All Files & Folders", "모든 파일 및 폴더", "所有文件和文件夹", "すべてのファイルとフォルダー")
    }
    pub fn tgt_directory(self) -> &'static str {
        self.pick("Folder", "폴더", "文件夹", "フォルダー")
    }
    pub fn tgt_dir_bg(self) -> &'static str {
        self.pick("Folder Background", "폴더 배경", "文件夹背景", "フォルダーの背景")
    }
    pub fn tgt_folder_virtual(self) -> &'static str {
        self.pick("Folder (incl. Virtual)", "폴더 (가상 포함)", "文件夹 (含虚拟)", "フォルダー (仮想含む)")
    }
    pub fn tgt_drive(self) -> &'static str {
        self.pick("Drive", "드라이브", "驱动器", "ドライブ")
    }
    pub fn tgt_desktop_bg(self) -> &'static str {
        self.pick("Desktop Background", "바탕화면 배경", "桌面背景", "デスクトップの背景")
    }
    pub fn tgt_lib_folder(self) -> &'static str {
        self.pick("Library Folder", "라이브러리 폴더", "库文件夹", "ライブラリフォルダー")
    }
    pub fn tgt_lib_bg(self) -> &'static str {
        self.pick("Library Background", "라이브러리 배경", "库背景", "ライブラリの背景")
    }
    pub fn tgt_sendto(self) -> &'static str {
        self.pick("Send To Menu", "보내기 메뉴", "发送到菜单", "送るメニュー")
    }
    pub fn fmt_tgt_file_assoc(self, name: &str) -> String {
        match self {
            Language::En => format!("File Assoc ({name})"),
            Language::Ko => format!("파일 연결 ({name})"),
            Language::Zh => format!("文件关联 ({name})"),
            Language::Ja => format!("関連付け ({name})"),
        }
    }
    pub fn fmt_tgt_shellnew(self, ext: &str) -> String {
        match self {
            Language::En => format!("New ({ext})"),
            Language::Ko => format!("새로 만들기 ({ext})"),
            Language::Zh => format!("新建 ({ext})"),
            Language::Ja => format!("新規作成 ({ext})"),
        }
    }
    pub fn fmt_summary_more(self, first: &str, second: &str, rest: usize) -> String {
        match self {
            Language::En => format!("{first}, {second} +{rest} more"),
            Language::Ko => format!("{first}, {second} 외 {rest}곳"),
            Language::Zh => format!("{first}、{second} 等 {rest} 处"),
            Language::Ja => format!("{first}、{second} ほか {rest} 件"),
        }
    }

    // --- Toolbar ---
    pub fn search_label(self) -> &'static str {
        self.pick("Search:", "검색:", "搜索:", "検索:")
    }
    pub fn search_hint(self) -> &'static str {
        self.pick(
            "Name, menu text, path, ext, CLSID...",
            "이름, 표시 메뉴, 실행 경로, 확장자, CLSID...",
            "名称、菜单文本、路径、扩展名、CLSID...",
            "名前、表示メニュー、パス、拡張子、CLSID...",
        )
    }
    pub fn btn_clear(self) -> &'static str {
        self.pick("Clear", "지우기", "清除", "クリア")
    }
    pub fn filter_orphaned(self) -> &'static str {
        self.pick("Orphaned only (missing file)", "잔여 항목만 (파일 없음)", "仅残留项 (文件缺失)", "残骸のみ (ファイルなし)")
    }
    pub fn filter_orphaned_tip(self) -> &'static str {
        self.pick(
            "Show only leftover entries whose target executable or DLL no longer exists.",
            "실행 파일이나 DLL이 디스크에 없는 찌꺼기 항목만 표시합니다.",
            "仅显示目标程序或 DLL 已不存在的残留菜单项。",
            "実行ファイルや DLL が存在しない残骸項目のみ表示します。",
        )
    }
    pub fn filter_disabled(self) -> &'static str {
        self.pick("Disabled only", "비활성화됨만", "仅已禁用", "無効のみ")
    }
    pub fn filter_hide_system(self) -> &'static str {
        self.pick("Hide Microsoft built-in", "Microsoft 기본 항목 숨기기", "隐藏 Microsoft 内置项", "Microsoft 標準項目を隠す")
    }
    pub fn filter_hide_system_tip(self) -> &'static str {
        self.pick(
            "Hide built-in Windows entries to prevent accidental removal.",
            "Windows 기본 내장 항목을 목록에서 숨겨 실수로 삭제하지 않도록 보호합니다.",
            "隐藏 Windows 内置项，防止误删系统功能。",
            "誤削除を防ぐため Windows 標準項目を一覧から隠します。",
        )
    }
    pub fn view_grouped(self) -> &'static str {
        self.pick("Group by App", "앱별 묶어 보기", "按应用分组", "アプリ別にまとめる")
    }
    pub fn view_grouped_tip(self) -> &'static str {
        self.pick(
            "Group multiple registry locations belonging to the same app for bulk cleanup.",
            "같은 프로그램의 여러 레지스트리 위치를 하나로 묶어 일괄 관리합니다.",
            "将同一应用的多个注册表位置合并显示，便于批量清理。",
            "同じアプリの複数レジストリ登録をまとめて一括管理します。",
        )
    }
    pub fn view_flat(self) -> &'static str {
        self.pick("Flat List", "개별 항목 보기", "平铺列表", "個別表示")
    }
    pub fn btn_backup_restore(self) -> &'static str {
        self.pick("Backup / Restore", "백업 / 복원", "备份 / 还原", "バックアップ / 復元")
    }
    pub fn btn_check_updates(self) -> &'static str {
        self.pick("Check Updates", "업데이트 확인", "检查更新", "更新を確認")
    }
    pub fn checking_updates(self) -> &'static str {
        self.pick("Checking...", "확인 중...", "检查中...", "確認中...")
    }
    pub fn btn_refresh(self) -> &'static str {
        self.pick("Refresh", "새로고침", "刷新", "更新")
    }
    pub fn scanning_label(self) -> &'static str {
        self.pick("Scanning...", "스캔 중...", "扫描中...", "スキャン中...")
    }
    pub fn probing_label(self) -> &'static str {
        self.pick(
            "Loading menu labels...",
            "메뉴 문구 조회 중...",
            "正在读取菜单文本...",
            "メニュー名を取得中...",
        )
    }

    // --- Update banner ---
    pub fn fmt_update_available(self, ver: &str) -> String {
        match self {
            Language::En => format!("New version v{ver} is available."),
            Language::Ko => format!("새 버전 v{ver} 이 있습니다."),
            Language::Zh => format!("发现新版本 v{ver}。"),
            Language::Ja => format!("新しいバージョン v{ver} があります。"),
        }
    }
    pub fn fmt_current_version(self, ver: &str) -> String {
        match self {
            Language::En => format!("(current v{ver})"),
            Language::Ko => format!("(현재 v{ver})"),
            Language::Zh => format!("(当前 v{ver})"),
            Language::Ja => format!("(現在 v{ver})"),
        }
    }
    pub fn btn_install_now(self) -> &'static str {
        self.pick("Install Now", "지금 설치", "立即安装", "今すぐインストール")
    }
    pub fn btn_skip_version(self) -> &'static str {
        self.pick("Skip Version", "이 버전 건너뛰기", "跳过此版本", "このバージョンをスキップ")
    }
    pub fn fmt_downloading_update(self, pct: f32) -> String {
        match self {
            Language::En => format!("Downloading... {:.0}%  (App will restart automatically after install)", pct),
            Language::Ko => format!("다운로드 중... {:.0}%  (완료되면 잠시 종료된 뒤 자동으로 다시 실행됩니다)", pct),
            Language::Zh => format!("正在下载... {:.0}%  (完成后将自动重启应用)", pct),
            Language::Ja => format!("ダウンロード中... {:.0}%  (完了後に自動で再起動されます)", pct),
        }
    }
    pub fn fmt_update_failed(self, err: &str) -> String {
        match self {
            Language::En => format!("Update failed: {err}"),
            Language::Ko => format!("업데이트 실패: {err}"),
            Language::Zh => format!("更新失败: {err}"),
            Language::Ja => format!("更新に失敗しました: {err}"),
        }
    }
    pub fn release_notes_header(self) -> &'static str {
        self.pick("Release Notes", "변경 내용", "更新说明", "変更内容")
    }
    pub fn fmt_already_latest(self, ver: &str) -> String {
        match self {
            Language::En => format!("You are using the latest version (v{ver})."),
            Language::Ko => format!("이미 최신 버전을 사용 중입니다 (v{ver})."),
            Language::Zh => format!("当前已是最新版本 (v{ver})。"),
            Language::Ja => format!("最新バージョンを使用しています (v{ver})。"),
        }
    }
    pub fn fmt_update_check_failed(self, err: &str) -> String {
        match self {
            Language::En => format!("Update check failed: {err}"),
            Language::Ko => format!("업데이트 확인 실패: {err}"),
            Language::Zh => format!("检查更新失败: {err}"),
            Language::Ja => format!("更新の確認に失敗しました: {err}"),
        }
    }

    // --- Sidebar ---
    pub fn sidebar_title(self) -> &'static str {
        self.pick("Context Locations", "우클릭 위치별 분류", "右键菜单位置", "右クリック場所別")
    }
    pub fn fmt_sidebar_orphan_badge(self, count: usize) -> String {
        match self {
            Language::En => format!("[{count} orphan]"),
            Language::Ko => format!("[잔여 {count}]"),
            Language::Zh => format!("[残留 {count}]"),
            Language::Ja => format!("[残骸 {count}]"),
        }
    }
    pub fn fmt_os_label(self, build: u32) -> String {
        if build >= 22000 {
            match self {
                Language::En => format!("Windows 11 (Build {build})"),
                Language::Ko => format!("Windows 11 (빌드 {build})"),
                Language::Zh => format!("Windows 11 (版本 {build})"),
                Language::Ja => format!("Windows 11 (ビルド {build})"),
            }
        } else if build > 0 {
            match self {
                Language::En => format!("Windows 10 (Build {build})"),
                Language::Ko => format!("Windows 10 (빌드 {build})"),
                Language::Zh => format!("Windows 10 (版本 {build})"),
                Language::Ja => format!("Windows 10 (ビルド {build})"),
            }
        } else {
            "Windows".to_string()
        }
    }
    pub fn fmt_scan_stats(self, total: usize, ms: u128) -> String {
        match self {
            Language::En => format!("Scanned {total} entries ({ms}ms)"),
            Language::Ko => format!("전체 {total}건 스캔 ({ms}ms)"),
            Language::Zh => format!("已扫描 {total} 项 ({ms}ms)"),
            Language::Ja => format!("全 {total} 件スキャン ({ms}ms)"),
        }
    }

    // --- List ---
    pub fn btn_select_all_visible(self) -> &'static str {
        self.pick("Select All Visible", "표시된 항목 모두 선택", "全选当前项", "表示中をすべて選択")
    }
    pub fn btn_select_all_orphans(self) -> &'static str {
        self.pick("Select All Orphaned", "잔여 항목 모두 선택", "全选残留项", "残骸をすべて選択")
    }
    pub fn btn_deselect_all(self) -> &'static str {
        self.pick("Deselect All", "선택 해제", "取消选择", "選択解除")
    }
    pub fn fmt_list_summary(self, entries: usize, groups: usize, orphans: usize) -> String {
        match self {
            Language::En => format!("Showing: {entries} items ({groups} groups) / {orphans} orphaned"),
            Language::Ko => format!("표시: {entries}개 항목 ({groups}개 그룹) / 잔여 {orphans}개"),
            Language::Zh => format!("显示: {entries} 项 ({groups} 组) / 残留 {orphans} 项"),
            Language::Ja => format!("表示: {entries} 件 ({groups} グループ) / 残骸 {orphans} 件"),
        }
    }
    pub fn scanning_empty_text(self) -> &'static str {
        self.pick(
            "Scanning Windows context menu entries...",
            "Windows 컨텍스트 메뉴를 스캔하고 있습니다...",
            "正在扫描 Windows 右键菜单项...",
            "Windows コンテキストメニューをスキャンしています...",
        )
    }
    pub fn no_matching_entries(self) -> &'static str {
        self.pick(
            "No context menu entries match the current filter.",
            "조건에 맞는 컨텍스트 메뉴 항목이 없습니다.",
            "没有符合当前筛选条件的右键菜单项。",
            "条件に一致するコンテキストメニュー項目がありません。",
        )
    }
    pub fn badge_all_orphaned(self) -> &'static str {
        self.pick("[Missing File - Orphaned]", "[파일 없음 - 잔여 항목]", "[文件缺失 - 残留项]", "[ファイルなし - 残骸]")
    }
    pub fn badge_some_orphaned(self) -> &'static str {
        self.pick("[Some Missing]", "[일부 파일 없음]", "[部分文件缺失]", "[一部ファイルなし]")
    }
    pub fn badge_orphaned_short(self) -> &'static str {
        self.pick("[Missing File]", "[파일 없음]", "[文件缺失]", "[ファイルなし]")
    }
    pub fn badge_disabled(self) -> &'static str {
        self.pick("[Disabled]", "[비활성화됨]", "[已禁用]", "[無効]")
    }
    pub fn badge_some_disabled(self) -> &'static str {
        self.pick("[Partially Disabled]", "[일부 비활성화]", "[部分已禁用]", "[一部無効]")
    }
    pub fn badge_system(self) -> &'static str {
        self.pick("[System]", "[시스템]", "[系统]", "[システム]")
    }
    pub fn fmt_group_partial_total(self, shown: usize, total: usize) -> String {
        match self {
            Language::En => format!("({shown} here / {total} total)"),
            Language::Ko => format!("(이 분류 {shown}곳 / 전체 {total}곳)"),
            Language::Zh => format!("(当前分类 {shown} 处 / 共 {total} 处)"),
            Language::Ja => format!("(この分類 {shown} 件 / 全体 {total} 件)"),
        }
    }
    pub fn group_partial_tip(self) -> &'static str {
        self.pick(
            "This app has additional context menu entries in other categories.\nYou can select all locations in the Details panel or the Delete confirmation dialog.",
            "다른 분류에도 같은 프로그램의 컨텍스트 메뉴 항목이 더 있습니다.\n우측 상세 패널이나 삭제 확인 창에서 전체 위치를 한꺼번에 선택할 수 있습니다.",
            "该应用在其他分类下还有更多右键菜单项。\n您可以在右侧详情面板或删除确认窗口中一键全选。",
            "他の分類にも同じアプリのメニュー項目が存在します。\n右側の詳細パネルまたは削除確認ダイアログで一括選択できます。",
        )
    }
    pub fn fmt_group_count(self, count: usize) -> String {
        match self {
            Language::En => format!("({count} locations)"),
            Language::Ko => format!("({count}곳)"),
            Language::Zh => format!("({count} 处)"),
            Language::Ja => format!("({count} 箇所)"),
        }
    }
    pub fn fmt_partial_selected(self, checked: usize, shown: usize) -> String {
        match self {
            Language::En => format!("[{checked}/{shown} selected]"),
            Language::Ko => format!("[{checked}/{shown} 선택]"),
            Language::Zh => format!("[已选 {checked}/{shown}]"),
            Language::Ja => format!("[{checked}/{shown} 選択]"),
        }
    }

    // --- Detail panel ---
    pub fn detail_title(self) -> &'static str {
        self.pick("Details", "상세 정보", "详细信息", "詳細情報")
    }
    pub fn detail_empty_hint(self) -> &'static str {
        self.pick(
            "Click an item or group in the list to inspect its command, file status, and all registered locations.",
            "목록에서 항목이나 그룹을 클릭하면\n실행 명령어, 파일 존재 여부, 포함된 모든 레지스트리 경로를 볼 수 있습니다.",
            "点击列表中的任意项或分组，即可查看执行命令、文件状态及所有注册表路径。",
            "一覧から項目またはグループをクリックすると、\n実行コマンド、ファイル状態、登録されているすべてのパスを確認できます。",
        )
    }
    pub fn meta_kind(self) -> &'static str {
        self.pick("Type", "유형", "类型", "種類")
    }
    pub fn meta_scope(self) -> &'static str {
        self.pick("Scope", "범위", "范围", "範囲")
    }
    pub fn meta_file_status(self) -> &'static str {
        self.pick("File Status", "파일 상태", "文件状态", "ファイル状態")
    }
    pub fn meta_package(self) -> &'static str {
        self.pick("Package", "패키지", "应用包", "パッケージ")
    }
    pub fn meta_clsid_desc(self) -> &'static str {
        self.pick("CLSID Description", "CLSID 설명", "CLSID 说明", "CLSID 説明")
    }
    pub fn meta_menu_labels(self) -> &'static str {
        self.pick("Explorer Menu", "탐색기 표시 메뉴", "资源管理器菜单", "エクスプローラー表示")
    }
    pub fn file_exists_ok(self) -> &'static str {
        self.pick("File exists", "실제 파일 존재함", "文件存在", "ファイルが存在します")
    }
    pub fn file_missing_orphan(self) -> &'static str {
        self.pick("File missing (leftover from uninstalled app)", "파일 없음 (제거된 앱의 잔여 항목)", "文件缺失 (已卸载应用的残留项)", "ファイルなし (アンインストール済みアプリの残骸)")
    }
    pub fn file_status_none(self) -> &'static str {
        self.pick("No file to verify", "확인 대상 파일 없음", "无对应文件", "確認対象ファイルなし")
    }
    pub fn target_exe_label(self) -> &'static str {
        self.pick("Target Executable / DLL:", "대상 실행 파일 / DLL:", "目标可执行文件 / DLL:", "対象実行ファイル / DLL:")
    }
    pub fn command_label(self) -> &'static str {
        self.pick("Command:", "실행 명령어:", "执行命令:", "実行コマンド:")
    }
    pub fn icon_label(self) -> &'static str {
        self.pick("Icon:", "아이콘:", "图标:", "アイコン:")
    }
    pub fn btn_copy_path(self) -> &'static str {
        self.pick("Copy Path", "경로 복사", "复制路径", "パスをコピー")
    }
    pub fn btn_open_file_location(self) -> &'static str {
        self.pick("Open File Location", "파일 위치 열기", "打开文件位置", "ファイルの場所を開く")
    }
    pub fn btn_open_regedit(self) -> &'static str {
        self.pick("Open in Regedit", "레지스트리 편집기에서 열기", "在注册表编辑器中打开", "レジストリエディターで開く")
    }
    pub fn btn_open_folder(self) -> &'static str {
        self.pick("Open Folder", "폴더에서 열기", "打开所在文件夹", "フォルダーを開く")
    }
    pub fn fmt_registered_locations(self, count: usize) -> String {
        match self {
            Language::En => format!("Registered Locations ({count} total)"),
            Language::Ko => format!("포함된 등록 위치 (전체 {count}곳)"),
            Language::Zh => format!("包含的注册位置 (共 {count} 处)"),
            Language::Ja => format!("登録されている場所 (全 {count} 箇所)"),
        }
    }
    pub fn fmt_check_all_locations(self, count: usize) -> String {
        match self {
            Language::En => format!("Check All ({count})"),
            Language::Ko => format!("전체 {count}곳 모두 체크"),
            Language::Zh => format!("全选 {count} 处"),
            Language::Ja => format!("全 {count} 箇所をチェック"),
        }
    }
    pub fn btn_uncheck_all(self) -> &'static str {
        self.pick("Uncheck All", "모두 해제", "全部取消", "すべて解除")
    }
    pub fn fmt_other_category_badge(self, cat_label: &str) -> String {
        match self {
            Language::En => format!("[Other: {cat_label}]"),
            Language::Ko => format!("[다른 분류: {cat_label}]"),
            Language::Zh => format!("[其他分类: {cat_label}]"),
            Language::Ja => format!("[他の分類: {cat_label}]"),
        }
    }
    pub fn tag_disabled_paren(self) -> &'static str {
        self.pick("(Disabled)", "(비활성화)", "(已禁用)", "(無効)")
    }
    pub fn tag_missing_paren(self) -> &'static str {
        self.pick("(Missing)", "(파일 없음)", "(文件缺失)", "(ファイルなし)")
    }
    pub fn msg_copied_path(self) -> &'static str {
        self.pick("Copied path to clipboard.", "경로를 클립보드에 복사했습니다.", "已复制路径到剪贴板。", "パスをクリップボードにコピーしました。")
    }

    // --- Bottom action bar ---
    pub fn fmt_selected_count(self, count: usize) -> String {
        match self {
            Language::En => format!("Selected: {count}"),
            Language::Ko => format!("선택됨: {count}개"),
            Language::Zh => format!("已选择: {count} 项"),
            Language::Ja => format!("選択中: {count} 件"),
        }
    }
    pub fn fmt_hidden_selected_warn(self, count: usize) -> String {
        match self {
            Language::En => format!("(incl. {count} outside current view)"),
            Language::Ko => format!("(현재 목록 외 {count}개 포함)"),
            Language::Zh => format!("(含当前列表外 {count} 项)"),
            Language::Ja => format!("(現在の表示外 {count} 件を含む)"),
        }
    }
    pub fn hidden_selected_warn_tip(self) -> &'static str {
        self.pick(
            "Some selected items are hidden by the current search or category filter and will also be affected by bulk actions.",
            "현재 검색어나 카테고리 필터에 표시되지 않는 선택 항목이 포함되어 있으며, 일괄 작업 시 함께 처리됩니다.",
            "当前已选项目中包含被搜索或分类筛选隐藏的项，执行批量操作时会一并处理。",
            "現在の検索や分類フィルターで非表示になっている選択項目が含まれており、一括操作時に一緒に処理されます。",
        )
    }
    pub fn btn_disable(self) -> &'static str {
        self.pick("Disable (Hide)", "비활성화 (숨기기)", "禁用 (隐藏)", "無効化 (非表示)")
    }
    pub fn btn_disable_tip(self) -> &'static str {
        self.pick(
            "Hide items from Explorer without deleting them. Can be re-enabled anytime.",
            "항목을 삭제하지 않고 탐색기 메뉴에서만 숨깁니다. 언제든 다시 켤 수 있습니다.",
            "仅在资源管理器菜单中隐藏项目而不删除，随时可以重新启用。",
            "削除せずにエクスプローラーのメニューから非表示にします。いつでも元に戻せます。",
        )
    }
    pub fn btn_enable(self) -> &'static str {
        self.pick("Enable", "다시 켜기", "重新启用", "有効化")
    }
    pub fn btn_remove(self) -> &'static str {
        self.pick("Remove Selected", "선택 항목 제거", "删除所选项", "選択項目を削除")
    }
    pub fn btn_remove_tip(self) -> &'static str {
        self.pick(
            "Automatically creates a backup (.json & .reg) before deleting selected entries.",
            "제거 전 자동으로 백업을 저장한 뒤 선택한 항목을 삭제합니다.",
            "删除前自动创建备份 (.json 与 .reg)，随后移除所选项目。",
            "削除前に自動バックアップ (.json と .reg) を保存してから削除します。",
        )
    }
    pub fn btn_restart_explorer(self) -> &'static str {
        self.pick("Restart Explorer", "탐색기 다시 시작", "重启资源管理器", "エクスプローラーを再起動")
    }
    pub fn btn_restart_explorer_tip(self) -> &'static str {
        self.pick(
            "Use this to immediately apply changes to COM ShellEx or Windows 11 modern menu items.",
            "COM 셸 확장이나 Windows 11 신형 메뉴 변경 사항을 즉시 반영할 때 사용합니다.",
            "用于立即刷新 COM 扩展或 Windows 11 新版菜单的更改。",
            "COM シェル拡張や Windows 11 モダンメニューの変更を即時反映する際に使用します。",
        )
    }
    pub fn badge_admin(self) -> &'static str {
        self.pick("Administrator", "관리자 권한", "管理员权限", "管理者権限")
    }
    pub fn badge_standard_user(self) -> &'static str {
        self.pick("Standard User", "일반 권한", "普通权限", "標準権限")
    }
    pub fn btn_relaunch_admin(self) -> &'static str {
        self.pick("Relaunch as Admin", "관리자 권한으로 재실행", "以管理员身份重启", "管理者として再起動")
    }
    pub fn btn_relaunch_admin_tip(self) -> &'static str {
        self.pick(
            "Administrator privileges are required to modify or delete HKLM (system-wide) items.",
            "HKLM(시스템 전체) 항목을 수정하거나 제거하려면 관리자 권한이 필요합니다.",
            "修改或删除 HKLM (系统范围) 项需要管理员权限。",
            "HKLM (システム全体) 項目を変更・削除するには管理者権限が必要です。",
        )
    }

    // --- Action messages ---
    pub fn msg_select_first(self) -> &'static str {
        self.pick("Please select at least one item first.", "먼저 항목을 선택해 주세요.", "请先选择至少一个项目。", "先に項目を選択してください。")
    }
    pub fn verb_disabled(self) -> &'static str {
        self.pick("disabled (hidden)", "비활성화(숨김)", "禁用(隐藏)", "無効化(非表示)")
    }
    pub fn verb_enabled(self) -> &'static str {
        self.pick("re-enabled", "다시 활성화", "重新启用", "有効化")
    }
    pub fn verb_removed(self) -> &'static str {
        self.pick("removed", "제거", "删除", "削除")
    }
    pub fn fmt_action_ok(self, count: usize, verb: &str, backup_id: Option<&str>) -> String {
        let mut s = match self {
            Language::En => format!("{count} item(s) {verb}."),
            Language::Ko => format!("{count}개 항목을 {verb} 처리했습니다."),
            Language::Zh => format!("已{verb} {count} 个项目。"),
            Language::Ja => format!("{count} 件の項目を{verb}しました。"),
        };
        if let Some(bid) = backup_id {
            match self {
                Language::En => s.push_str(&format!(" (Auto backup: {bid})")),
                Language::Ko => s.push_str(&format!(" (자동 백업 완료: {bid})")),
                Language::Zh => s.push_str(&format!(" (已自动备份: {bid})")),
                Language::Ja => s.push_str(&format!(" (自動バックアップ: {bid})")),
            }
        }
        s
    }
    pub fn fmt_related_remaining_note(self, count: usize) -> String {
        match self {
            Language::En => format!(" (Note: {count} other location(s) of the same app remain unselected)"),
            Language::Ko => format!(" (참고: 같은 프로그램의 다른 위치 {count}곳은 미선택 상태)"),
            Language::Zh => format!(" (提示: 同一应用另有 {count} 处未选择)"),
            Language::Ja => format!(" (参考: 同じアプリの他 {count} 箇所は未選択)"),
        }
    }
    pub fn fmt_action_partial_err(self, ok: usize, fail: usize, first: &str) -> String {
        match self {
            Language::En => format!("Succeeded: {ok}, Failed: {fail} - {first}"),
            Language::Ko => format!("성공 {ok}건, 실패 {fail}건 - {first}"),
            Language::Zh => format!("成功 {ok} 项，失败 {fail} 项 - {first}"),
            Language::Ja => format!("成功 {ok} 件、失敗 {fail} 件 - {first}"),
        }
    }
    pub fn fmt_backup_or_delete_failed(self, err: &str) -> String {
        match self {
            Language::En => format!("Backup or removal failed: {err}"),
            Language::Ko => format!("백업 또는 삭제 실패: {err}"),
            Language::Zh => format!("备份或删除失败: {err}"),
            Language::Ja => format!("バックアップまたは削除に失敗しました: {err}"),
        }
    }

    // --- Delete modal ---
    pub fn modal_delete_title(self) -> &'static str {
        self.pick("Confirm Removal", "선택 항목 제거 확인", "确认删除所选项", "削除の確認")
    }
    pub fn fmt_modal_delete_desc(self, total: usize) -> String {
        match self {
            Language::En => format!("Remove {total} item(s) from the registry / file system."),
            Language::Ko => format!("총 {total}개 항목을 레지스트리/파일에서 제거합니다."),
            Language::Zh => format!("将从注册表/文件系统移除共 {total} 个项目。"),
            Language::Ja => format!("合計 {total} 件の項目をレジストリ/ファイルから削除します。"),
        }
    }
    pub fn modal_delete_backup_hint(self) -> &'static str {
        self.pick(
            "A backup (.json and .reg) is automatically saved before removal so you can restore anytime from 'Backup / Restore'.",
            "제거 직전에 자동으로 백업(.json 및 .reg)이 저장되므로, '백업 / 복원'에서 언제든 되돌릴 수 있습니다.",
            "删除前将自动保存备份 (.json 与 .reg)，您随时可在“备份 / 还原”中恢复。",
            "削除直前に自動でバックアップ (.json と .reg) が保存されるため、「バックアップ / 復元」からいつでも復元できます。",
        )
    }
    pub fn fmt_modal_related_warn(self, count: usize) -> String {
        match self {
            Language::En => format!("{count} more location(s) of the same app exist in other categories."),
            Language::Ko => format!("같은 프로그램의 다른 위치에 {count}개 항목이 더 남아 있습니다."),
            Language::Zh => format!("同一应用在其他分类下还有 {count} 个关联项目。"),
            Language::Ja => format!("同じアプリの項目が他の分類にさらに {count} 件あります。"),
        }
    }
    pub fn fmt_modal_related_cats(self, summary: &str) -> String {
        match self {
            Language::En => format!("Other categories: {summary}"),
            Language::Ko => format!("포함된 다른 분류: {summary}"),
            Language::Zh => format!("涉及其他分类: {summary}"),
            Language::Ja => format!("他の分類: {summary}"),
        }
    }
    pub fn fmt_modal_include_related(self, related: usize, total: usize) -> String {
        match self {
            Language::En => format!("Include {related} other location(s) as well ({total} total)"),
            Language::Ko => format!("다른 위치 {related}곳도 함께 포함해서 모두 제거 (총 {total}곳)"),
            Language::Zh => format!("一并包含其余 {related} 处并全部删除 (共 {total} 处)"),
            Language::Ja => format!("他の {related} 箇所も一緒にすべて削除 (合計 {total} 箇所)"),
        }
    }
    pub fn btn_remove_disabled_hide_only_tip(self) -> &'static str {
        self.pick(
            "Selected item(s) belong to installed Windows 11 app packages and cannot be deleted. Use 'Disable (Hide)' instead.",
            "선택한 항목은 정상 설치된 Windows 11 앱 패키지이므로 삭제할 수 없으며, '비활성화 (숨기기)'만 가능합니다.",
            "所选项属于正常安装的 Windows 11 应用包，无法删除，仅支持“禁用 (隐藏)”。",
            "選択した項目は正常にインストールされた Windows 11 パッケージのため削除できません。「無効化 (非表示)」のみ可能です。",
        )
    }
    pub fn hide_only_hint_inline(self) -> &'static str {
        self.pick(
            "(Hide only - installed Win11 package cannot be deleted)",
            "(숨기기 전용 - 설치된 Win11 패키지는 삭제 불가)",
            "(仅支持隐藏 - 已安装的 Win11 应用包不可删除)",
            "(非表示のみ - インストール済み Win11 パッケージは削除不可)",
        )
    }
    pub fn detail_hide_only_notice(self) -> &'static str {
        self.pick(
            "Note: This item is part of an installed Windows 11 app package. It cannot be deleted directly; use 'Disable (Hide)' to hide it from Explorer.",
            "안내: 정상 설치된 Windows 11 앱 패키지의 메뉴 항목이므로 삭제할 수 없으며, '비활성화 (숨기기)'만 가능합니다.",
            "提示: 此项属于正常安装的 Windows 11 应用包，无法直接删除，仅支持通过“禁用 (隐藏)”将其从右键菜单中隐藏。",
            "案内: インストール済みの Windows 11 アプリパッケージ項目のため削除できません。「無効化 (非表示)」のみ利用できます。",
        )
    }
    pub fn fmt_modal_win11_skipped(self, count: usize) -> String {
        match self {
            Language::En => format!("{count} installed Windows 11 packaged item(s) cannot be deleted and will be skipped. Use 'Disable (Hide)' to hide them instead."),
            Language::Ko => format!("정상 설치된 Windows 11 패키지 항목 {count}건은 삭제할 수 없으므로 제외됩니다. 숨기려면 '비활성화 (숨기기)'를 사용해 주세요."),
            Language::Zh => format!("{count} 个正常安装的 Windows 11 应用包项无法删除，将被跳过。如需隐藏请使用“禁用 (隐藏)”。"),
            Language::Ja => format!("正常にインストールされた Windows 11 パッケージ項目 {count} 件は削除できないためスキップされます。隠すには「無効化 (非表示)」を使用してください。"),
        }
    }
    pub fn fmt_modal_win11_orphan_remove(self, count: usize) -> String {
        match self {
            Language::En => format!("{count} orphaned Windows 11 package(s) (missing DLL) will be completely uninstalled via Remove-AppxPackage."),
            Language::Ko => format!("파일이 없는 잔여 Windows 11 패키지 {count}건은 Remove-AppxPackage로 Windows에서 완전히 제거됩니다."),
            Language::Zh => format!("{count} 个文件缺失的残留 Windows 11 应用包将通过 Remove-AppxPackage 彻底卸载移除。"),
            Language::Ja => format!("ファイルのない残骸 Windows 11 パッケージ {count} 件は Remove-AppxPackage で Windows から完全に削除されます。"),
        }
    }
    pub fn fmt_shared_clsid_warn(self, peers: &str) -> String {
        match self {
            Language::En => format!("Warning: This CLSID is shared with other item(s) ({peers}). Disabling/blocking it will affect them together."),
            Language::Ko => format!("주의: 이 CLSID는 다른 항목({peers})과 공유되므로 숨기기/차단 시 함께 영향을 받습니다."),
            Language::Zh => format!("注意: 此 CLSID 与其他项（{peers}）共享，禁用或阻止时会一并受影响。"),
            Language::Ja => format!("注意: この CLSID は他の項目（{peers}）と共有されているため、無効化すると一緒に影響を受けます。"),
        }
    }
    pub fn fmt_modal_admin_warn(self, count: usize) -> String {
        match self {
            Language::En => format!("{count} HKLM (system-wide) item(s) require Administrator privileges to remove."),
            Language::Ko => format!("HKLM(시스템 전체) 항목 {count}건은 관리자 권한이 없으면 제거에 실패합니다."),
            Language::Zh => format!("{count} 个 HKLM (系统范围) 项需要管理员权限才能删除。"),
            Language::Ja => format!("HKLM (システム全体) 項目 {count} 件の削除には管理者権限が必要です。"),
        }
    }
    pub fn fmt_and_more(self, rest: usize) -> String {
        match self {
            Language::En => format!("... and {rest} more"),
            Language::Ko => format!("... 외 {rest}건"),
            Language::Zh => format!("... 另有 {rest} 项"),
            Language::Ja => format!("... ほか {rest} 件"),
        }
    }
    pub fn fmt_btn_backup_and_remove(self, total: usize) -> String {
        match self {
            Language::En => format!("Backup & Remove ({total})"),
            Language::Ko => format!("백업 후 {total}개 제거"),
            Language::Zh => format!("备份并删除 ({total})"),
            Language::Ja => format!("バックアップして {total} 件削除"),
        }
    }
    pub fn btn_cancel(self) -> &'static str {
        self.pick("Cancel", "취소", "取消", "キャンセル")
    }
    pub fn btn_close(self) -> &'static str {
        self.pick("Close", "닫기", "关闭", "閉じる")
    }

    // --- Backup modal ---
    pub fn modal_backup_title(self) -> &'static str {
        self.pick("Backup & Restore", "백업 및 복원 관리", "备份与还原管理", "バックアップと復元")
    }
    pub fn fmt_backup_location(self, path: &str) -> String {
        match self {
            Language::En => format!("Location: {path}"),
            Language::Ko => format!("저장 위치: {path}"),
            Language::Zh => format!("保存路径: {path}"),
            Language::Ja => format!("保存先: {path}"),
        }
    }
    pub fn backup_empty(self) -> &'static str {
        self.pick(
            "No backups yet. Backups are created automatically whenever you remove items.",
            "저장된 백업 기록이 없습니다. 항목을 제거하면 자동으로 백업이 생성됩니다.",
            "暂无备份记录。删除项目时系统会自动创建备份。",
            "バックアップ記録がありません。項目を削除すると自動で作成されます。",
        )
    }
    pub fn fmt_backup_items_header(self, count: usize) -> String {
        match self {
            Language::En => format!("Items in selected backup ({count}):"),
            Language::Ko => format!("선택한 백업 포함 항목 ({count}건):"),
            Language::Zh => format!("所选备份包含项目 ({count} 项):"),
            Language::Ja => format!("選択したバックアップの項目 ({count} 件):"),
        }
    }
    pub fn btn_restore_backup(self) -> &'static str {
        self.pick("Restore Selected Backup", "선택한 백업 복원", "还原所选备份", "選択したバックアップを復元")
    }
    pub fn btn_delete_backup(self) -> &'static str {
        self.pick("Delete Record", "백업 기록 삭제", "删除此记录", "記録を削除")
    }
    pub fn btn_open_backup_folder(self) -> &'static str {
        self.pick("Open Backup Folder", "백업 폴더 열기", "打开备份文件夹", "バックアップフォルダーを開く")
    }
    pub fn fmt_restore_ok(self, ts: &str, count: usize) -> String {
        match self {
            Language::En => format!("Restored {count} item(s) from backup ({ts})."),
            Language::Ko => format!("백업({ts})의 {count}개 항목을 복원했습니다."),
            Language::Zh => format!("已从备份 ({ts}) 还原 {count} 个项目。"),
            Language::Ja => format!("バックアップ ({ts}) から {count} 件を復元しました。"),
        }
    }
    pub fn fmt_restore_err(self, ok: usize, fail: usize, first: &str) -> String {
        match self {
            Language::En => format!("Restored: {ok}, Failed: {fail} - {first}"),
            Language::Ko => format!("복원 성공 {ok}건, 실패 {fail}건 - {first}"),
            Language::Zh => format!("还原成功 {ok} 项，失败 {fail} 项 - {first}"),
            Language::Ja => format!("復元成功 {ok} 件、失敗 {fail} 件 - {first}"),
        }
    }

    // --- Restart explorer modal ---
    pub fn modal_explorer_title(self) -> &'static str {
        self.pick("Restart Windows Explorer", "Windows 탐색기 다시 시작", "重启 Windows 资源管理器", "Windows エクスプローラーの再起動")
    }
    pub fn modal_explorer_desc(self) -> &'static str {
        self.pick(
            "Terminate and relaunch explorer.exe.",
            "탐색기(explorer.exe)를 종료한 뒤 다시 실행합니다.",
            "将结束并重新启动 explorer.exe。",
            "explorer.exe を終了して再起動します。",
        )
    }
    pub fn modal_explorer_hint(self) -> &'static str {
        self.pick(
            "The taskbar and open folder windows will briefly close and reopen.",
            "작업 표시줄과 열려 있는 폴더 창이 잠시 닫혔다가 다시 열립니다.",
            "任务栏和已打开的文件夹窗口会短暂关闭后重新打开。",
            "タスクバーと開いているフォルダーが一時的に閉じ、再度開きます。",
        )
    }
    pub fn btn_restart_now(self) -> &'static str {
        self.pick("Restart", "다시 시작", "立即重启", "再起動")
    }
    pub fn msg_explorer_restarted(self) -> &'static str {
        self.pick("Windows Explorer restarted.", "Windows 탐색기를 다시 시작했습니다.", "已重启 Windows 资源管理器。", "Windows エクスプローラーを再起動しました。")
    }
    pub fn modal_explorer_needed_title(self) -> &'static str {
        self.pick(
            "Restart Explorer to Apply Changes",
            "변경 사항 반영을 위해 탐색기 다시 시작 필요",
            "需要重启资源管理器以应用更改",
            "変更を反映するにはエクスプローラーの再起動が必要です",
        )
    }
    pub fn fmt_modal_explorer_needed_desc(self, kinds: &str) -> String {
        match self {
            Language::En => format!(
                "Changes to {kinds} are cached by Windows Explorer. Restart Explorer now to reflect them in the right-click menu immediately?"
            ),
            Language::Ko => format!(
                "{kinds} 항목은 Windows 탐색기가 메모리에 캐시하므로, 지금 탐색기를 다시 시작해야 우클릭 메뉴에 즉시 반영됩니다. 지금 다시 시작할까요?"
            ),
            Language::Zh => format!(
                "{kinds} 项会被 Windows 资源管理器缓存，需要重启资源管理器才能立即在右键菜单中生效。是否立即重启？"
            ),
            Language::Ja => format!(
                "{kinds} 項目は Windows エクスプローラーにキャッシュされるため、右クリックメニューへ即時反映するには再起動が必要です。今すぐ再起動しますか？"
            ),
        }
    }
    pub fn btn_restart_later(self) -> &'static str {
        self.pick("Later", "나중에 하기", "稍后重启", "あとで行う")
    }
    pub fn explorer_pending_banner(self) -> &'static str {
        self.pick(
            "Some changes require restarting Windows Explorer to appear in the right-click menu.",
            "일부 변경 사항은 Windows 탐색기를 다시 시작해야 우클릭 메뉴에 반영됩니다.",
            "部分更改需要重启 Windows 资源管理器后才会生效。",
            "一部の変更は Windows エクスプローラーを再起動すると右クリックメニューに反映されます。",
        )
    }
    pub fn btn_restart_explorer_needed(self) -> &'static str {
        self.pick("Restart Explorer *", "탐색기 다시 시작 *", "重启资源管理器 *", "エクスプローラー再起動 *")
    }

    // --- Add / Edit Shell Command modal ---
    pub fn btn_add_shell(self) -> &'static str {
        self.pick("+ Add Shell Command", "+ Shell 명령 추가", "+ 添加 Shell 命令", "+ Shell コマンド追加")
    }
    pub fn btn_edit(self) -> &'static str {
        self.pick("Edit", "편집", "编辑", "編集")
    }
    pub fn modal_add_shell_title(self) -> &'static str {
        self.pick("Add Shell Command", "Shell 명령 추가", "添加 Shell 命令", "Shell コマンドを追加")
    }
    pub fn modal_edit_shell_title(self) -> &'static str {
        self.pick("Edit Shell Command", "Shell 명령 편집", "编辑 Shell 命令", "Shell コマンドを編集")
    }
    pub fn shell_target_label(self) -> &'static str {
        self.pick("Target", "적용 대상", "适用位置", "適用先")
    }
    pub fn shell_target_ext(self) -> &'static str {
        self.pick("Specific Extension (e.g. .mkv)", "특정 확장자 (예: .mkv)", "指定扩展名 (如 .mkv)", "特定の拡張子 (例: .mkv)")
    }
    pub fn shell_ext_label(self) -> &'static str {
        self.pick("Extension", "확장자", "扩展名", "拡張子")
    }
    pub fn shell_ext_hint(self) -> &'static str {
        self.pick("e.g. .mkv, .mp4, .txt", "예: .mkv, .mp4, .txt", "例如: .mkv, .mp4, .txt", "例: .mkv, .mp4, .txt")
    }
    pub fn shell_scope_label(self) -> &'static str {
        self.pick("Scope", "저장 범위", "存储范围", "保存範囲")
    }
    pub fn shell_display_name_label(self) -> &'static str {
        self.pick("Menu Label", "메뉴 표시 이름", "菜单显示名称", "メニュー表示名")
    }
    pub fn shell_display_name_hint(self) -> &'static str {
        self.pick("e.g. mkv2mp4", "예: mkv2mp4", "例如: mkv2mp4", "例: mkv2mp4")
    }
    pub fn shell_key_name_label(self) -> &'static str {
        self.pick("Registry Key Name", "레지스트리 키 이름", "注册表键名", "レジストリキー名")
    }
    pub fn shell_key_name_hint(self) -> &'static str {
        self.pick("Optional (defaults to Menu Label)", "선택 (비우면 표시 이름 사용)", "可选 (留空则使用显示名称)", "任意 (空欄時は表示名を使用)")
    }
    pub fn shell_command_label(self) -> &'static str {
        self.pick("Command", "실행 명령어", "执行命令", "実行コマンド")
    }
    pub fn shell_command_hint(self) -> &'static str {
        self.pick(
            "e.g. cmd /k c:\\[bat]\\mkv2mp4.bat \"%1\"",
            "예: cmd /k c:\\[bat]\\mkv2mp4.bat \"%1\"",
            "例如: cmd /k c:\\[bat]\\mkv2mp4.bat \"%1\"",
            "例: cmd /k c:\\[bat]\\mkv2mp4.bat \"%1\"",
        )
    }
    pub fn shell_icon_label(self) -> &'static str {
        self.pick("Icon Path", "아이콘 경로", "图标路径", "アイコンパス")
    }
    pub fn shell_icon_hint(self) -> &'static str {
        self.pick(
            "Optional (e.g. C:\\path\\app.exe or .ico / .dll,0)",
            "선택 (예: C:\\Python313\\python.exe 또는 .ico / .dll,0)",
            "可选 (如 C:\\path\\app.exe 或 .ico / .dll,0)",
            "任意 (例: C:\\path\\app.exe や .ico / .dll,0)",
        )
    }
    pub fn btn_browse(self) -> &'static str {
        self.pick("Browse...", "찾아보기...", "浏览...", "参照...")
    }
    pub fn shell_helpers_label(self) -> &'static str {
        self.pick("Quick insert:", "빠른 삽입:", "快捷插入:", "クイック挿入:")
    }
    pub fn shell_extended_checkbox(self) -> &'static str {
        self.pick("Show only on Shift + Right-Click (Extended)", "Shift + 우클릭 시에만 표시 (Extended)", "仅在按住 Shift 右键时显示 (Extended)", "Shift + 右クリック時のみ表示 (Extended)")
    }
    pub fn shell_position_label(self) -> &'static str {
        self.pick("Menu Position", "메뉴 위치", "菜单位置", "メニュー位置")
    }
    pub fn pos_default(self) -> &'static str {
        self.pick("Default", "기본", "默认", "標準")
    }
    pub fn pos_top(self) -> &'static str {
        self.pick("Top", "상단 (Top)", "顶部 (Top)", "上部 (Top)")
    }
    pub fn pos_bottom(self) -> &'static str {
        self.pick("Bottom", "하단 (Bottom)", "底部 (Bottom)", "下部 (Bottom)")
    }
    pub fn shell_overwrite_checkbox(self) -> &'static str {
        self.pick("Overwrite if key already exists", "같은 이름의 키가 이미 있으면 덮어쓰기", "若同名键已存在则覆盖", "同名キーが存在する場合は上書き")
    }
    pub fn shell_preview_label(self) -> &'static str {
        self.pick("Registry path to write:", "등록될 레지스트리 경로:", "将写入的注册表路径:", "書き込まれるレジストリパス:")
    }
    pub fn btn_save(self) -> &'static str {
        self.pick("Save", "저장", "保存", "保存")
    }
    pub fn dialog_pick_command_title(self) -> &'static str {
        self.pick("Select Executable or Script", "실행 파일 또는 스크립트 선택", "选择可执行文件或脚本", "実行ファイルまたはスクリプトを選択")
    }
    pub fn dialog_pick_icon_title(self) -> &'static str {
        self.pick("Select Icon File", "아이콘 파일 선택", "选择图标文件", "アイコンファイルを選択")
    }
    pub fn fmt_shell_saved(self, path: &str) -> String {
        match self {
            Language::En => format!("Saved Shell command: {path}"),
            Language::Ko => format!("Shell 명령을 저장했습니다: {path}"),
            Language::Zh => format!("已保存 Shell 命令: {path}"),
            Language::Ja => format!("Shell コマンドを保存しました: {path}"),
        }
    }
    pub fn translate_shell_err(self, code: &str) -> String {
        match code {
            "display_name_empty" => self.pick("Please enter a Menu Label.", "메뉴 표시 이름을 입력해 주세요.", "请输入菜单显示名称。", "メニュー表示名を入力してください.").into(),
            "command_empty" => self.pick("Please enter a Command.", "실행 명령어를 입력해 주세요.", "请输入执行命令。", "実行コマンドを入力してください.").into(),
            "extension_empty" => self.pick("Please enter a file extension (e.g. .mkv).", "확장자를 입력해 주세요 (예: .mkv).", "请输入扩展名 (例如 .mkv)。", "拡張子を入力してください (例: .mkv)。").into(),
            "extension_invalid" => self.pick("Extension cannot contain spaces or slashes.", "확장자에 공백이나 슬래시를 넣을 수 없습니다.", "扩展名不能包含空格或斜杠。", "拡張子に空白やスラッシュは使用できません。").into(),
            "key_empty" => self.pick("Registry key name cannot be empty.", "유효한 레지스트리 키 이름을 입력해 주세요.", "注册表键名不能为空。", "有効なレジストリキー名を入力してください。").into(),
            "key_invalid" => self.pick("Registry key name cannot contain \\ or /.", "레지스트리 키 이름에는 \\ 또는 / 를 넣을 수 없습니다.", "注册表键名不能包含 \\ 或 /。", "レジストリキー名に \\ や / は使用できません。").into(),
            "already_exists" => self.pick("A key with this name already exists. Enable 'Overwrite' to replace it.", "같은 이름의 레지스트리 키가 이미 존재합니다. 덮어쓰려면 '덮어쓰기'를 체크해 주세요.", "同名注册表键已存在。如需替换请勾选“覆盖”。", "同じ名前のレジストリキーが既に存在します。上書きするにはチェックを入れてください。").into(),
            other => other.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_code_parsing_and_distinct_translations() {
        assert_eq!(Language::default(), Language::En);
        assert_eq!(Language::from_code("en"), Some(Language::En));
        assert_eq!(Language::from_code("ko"), Some(Language::Ko));
        assert_eq!(Language::from_code("zh"), Some(Language::Zh));
        assert_eq!(Language::from_code("ja"), Some(Language::Ja));
        assert_eq!(Language::from_code("unknown"), None);

        assert_ne!(Language::En.btn_remove(), Language::Ko.btn_remove());
        assert_ne!(Language::Ko.btn_remove(), Language::Zh.btn_remove());
        assert_ne!(Language::Zh.btn_remove(), Language::Ja.btn_remove());
    }
}
