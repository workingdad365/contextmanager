//! ContextManager 메인 GUI (egui / eframe)

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::action::{
    self, ActionReport, ShellCommandSpec, ShellMenuPosition, ShellTargetKind,
};
use crate::backup::{self, BackupRecord};
use crate::config::{APP_NAME, APP_VERSION, Config};
use crate::dialog;
use crate::i18n::Language;
use crate::model::{
    Category, ContextEntry, EntryGroup, EntryKind, FilterOptions, build_groups, count_group_totals,
    find_related_unchecked_indices, matches_filter,
};
use crate::reg::RegRoot;
use crate::scanner::{self, ScanResult};
use crate::system;
use crate::update::{self, UpdateInfo};

const ICON_PNG: &[u8] = include_bytes!("../assets/contextmanager.png");

#[derive(Debug, Default)]
pub struct LaunchOptions {
    pub lang_override: Option<Language>,
    pub ready_file: Option<PathBuf>,
}

pub fn run(opts: LaunchOptions) {
    let mut config = Config::load();
    let persist_lang = opts.lang_override.is_none();
    if let Some(ov) = opts.lang_override {
        config.language = ov;
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("{APP_NAME} v{APP_VERSION}"))
        .with_inner_size([1200.0, 740.0])
        .with_min_inner_size([880.0, 540.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };

    let ready_file = opts.ready_file;
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            let font_cache = FontCache::load();
            apply_fonts(&cc.egui_ctx, &font_cache, config.language);
            Ok(Box::new(ContextManagerApp::new(
                cc.egui_ctx.clone(),
                config,
                persist_lang,
                font_cache,
                ready_file,
            )))
        }),
    );
    if let Err(e) = result {
        eprintln!("window error: {e}");
    }
}

/// Windows 시스템 CJK 폰트 캐시 (언어 전환 시 디스크 재읽기 없이 우선순위만 즉시 교체)
struct FontCache {
    ko: Option<Arc<egui::FontData>>,
    zh: Option<Arc<egui::FontData>>,
    ja: Option<Arc<egui::FontData>>,
}

impl FontCache {
    fn load() -> Self {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let fonts_dir = Path::new(&windir).join("Fonts");
        let load_first = |names: &[&str]| -> Option<Arc<egui::FontData>> {
            for name in names {
                if let Ok(bytes) = std::fs::read(fonts_dir.join(name)) {
                    return Some(Arc::new(egui::FontData::from_owned(bytes)));
                }
            }
            None
        };
        Self {
            ko: load_first(&["malgun.ttf"]),
            zh: load_first(&["msyh.ttc", "simsun.ttc"]),
            ja: load_first(&["YuGothM.ttc", "YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"]),
        }
    }
}

fn apply_fonts(ctx: &egui::Context, cache: &FontCache, lang: Language) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(d) = &cache.ko {
        fonts.font_data.insert("cjk_ko".into(), d.clone());
    }
    if let Some(d) = &cache.zh {
        fonts.font_data.insert("cjk_zh".into(), d.clone());
    }
    if let Some(d) = &cache.ja {
        fonts.font_data.insert("cjk_ja".into(), d.clone());
    }

    // 선택된 언어의 CJK 폰트를 최우선 배치하고 나머지 언어도 폴백으로 유지
    let order: &[&str] = match lang {
        Language::En => &["cjk_ko", "cjk_zh", "cjk_ja"],
        Language::Ko => &["cjk_ko", "cjk_ja", "cjk_zh"],
        Language::Zh => &["cjk_zh", "cjk_ko", "cjk_ja"],
        Language::Ja => &["cjk_ja", "cjk_ko", "cjk_zh"],
    };

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        if lang == Language::En {
            // 영어는 기본 라틴 글꼴을 유지하고 CJK 를 폴백으로 뒤에 추가
            for &name in order {
                if fonts.font_data.contains_key(name) {
                    list.push(name.into());
                }
            }
        } else {
            // 한/중/일은 해당 언어 시스템 폰트를 최우선 삽입
            let mut insert_at = 0;
            for &name in order {
                if fonts.font_data.contains_key(name) {
                    list.insert(insert_at, name.into());
                    insert_at += 1;
                }
            }
        }
    }
    ctx.set_fonts(fonts);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Grouped,
    Flat,
}

#[derive(Debug, Clone)]
enum FocusedTarget {
    Group(String),
    Entry(String),
}

enum StatusMessage {
    Info(String),
    Warn(String),
    Error(String),
}

struct UpdateCheckEvent {
    manual: bool,
    result: Result<Option<UpdateInfo>, String>,
}

enum UpdateInstallEvent {
    Progress(f32),
    Installed,
    Failed(String),
}

struct ContextManagerApp {
    ctx: egui::Context,
    config: Config,
    persist_lang: bool,
    font_cache: FontCache,

    entries: Vec<ContextEntry>,
    scanning: bool,
    scan_rx: Option<Receiver<ScanResult>>,
    last_scan_ms: Option<u128>,

    filter: FilterOptions,
    view_mode: ViewMode,
    checked_ids: HashSet<String>,
    focused: Option<FocusedTarget>,

    is_elevated: bool,
    os_build: u32,
    message: Option<StatusMessage>,

    // 업데이트 상태
    update_available: Option<UpdateInfo>,
    update_checking: bool,
    update_check_rx: Option<Receiver<UpdateCheckEvent>>,
    update_progress: Option<f32>,
    update_error: Option<String>,
    update_install_rx: Option<Receiver<UpdateInstallEvent>>,

    show_delete_modal: bool,
    include_related_on_delete: bool,
    show_backup_modal: bool,
    show_restart_explorer_modal: bool,
    shell_editor: Option<ShellCommandSpec>,
    shell_editor_error: Option<String>,
    backups: Vec<BackupRecord>,
    selected_backup_id: Option<String>,
    first_frame: bool,

    // 자동 스크린샷 촬영 준비 알림용
    ready_file: Option<PathBuf>,
    ready_countdown: Option<u8>,
}

impl ContextManagerApp {
    fn new(
        ctx: egui::Context,
        config: Config,
        persist_lang: bool,
        font_cache: FontCache,
        ready_file: Option<PathBuf>,
    ) -> Self {
        let auto_update = ready_file.is_none();
        let mut app = Self {
            ctx,
            config,
            persist_lang,
            font_cache,
            entries: Vec::new(),
            scanning: false,
            scan_rx: None,
            last_scan_ms: None,
            filter: FilterOptions::default(),
            view_mode: ViewMode::Grouped,
            checked_ids: HashSet::new(),
            focused: None,
            is_elevated: system::is_elevated(),
            os_build: system::windows_build_number(),
            message: None,
            update_available: None,
            update_checking: false,
            update_check_rx: None,
            update_progress: None,
            update_error: None,
            update_install_rx: None,
            show_delete_modal: false,
            include_related_on_delete: false,
            show_backup_modal: false,
            show_restart_explorer_modal: false,
            shell_editor: None,
            shell_editor_error: None,
            backups: Vec::new(),
            selected_backup_id: None,
            first_frame: true,
            ready_file,
            ready_countdown: None,
        };
        app.start_scan();
        if auto_update {
            app.start_update_check(false);
        }
        app
    }

    fn lang(&self) -> Language {
        self.config.language
    }

    fn set_language(&mut self, next: Language) {
        if self.config.language == next {
            return;
        }
        self.config.language = next;
        apply_fonts(&self.ctx, &self.font_cache, next);
        if self.persist_lang {
            let _ = self.config.save();
        }
    }

    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = scanner::scan_all();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else { return };
        if let Ok(result) = rx.try_recv() {
            self.entries = result.entries;
            self.last_scan_ms = Some(result.elapsed_ms);
            self.scanning = false;
            self.scan_rx = None;

            let valid_ids: HashSet<&str> = self.entries.iter().map(|e| e.id.as_str()).collect();
            self.checked_ids.retain(|id| valid_ids.contains(id.as_str()));

            // 스크린샷 모드에서는 대표 그룹(여러 위치 포함 항목 우선)을 자동 선택해 상세 패널까지 채워지도록 준비
            if self.ready_file.is_some() && self.ready_countdown.is_none() {
                let filtered = self.filtered_indices();
                let groups = build_groups(&self.entries, &filtered, self.lang());
                let picked = groups
                    .iter()
                    .find(|g| g.entry_indices.len() >= 2 && !g.display_name.starts_with('"'))
                    .or_else(|| groups.first());
                if let Some(first) = picked {
                    self.focused = Some(FocusedTarget::Group(first.key.clone()));
                }
                self.ready_countdown = Some(5);
                self.ctx.request_repaint();
            }
        }
    }

    fn start_update_check(&mut self, manual: bool) {
        if self.update_checking || self.update_progress.is_some() {
            return;
        }
        self.update_checking = true;
        let (tx, rx) = mpsc::channel();
        self.update_check_rx = Some(rx);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = update::check();
            let _ = tx.send(UpdateCheckEvent { manual, result });
            ctx.request_repaint();
        });
    }

    fn poll_update(&mut self) {
        if let Some(rx) = &self.update_check_rx {
            if let Ok(ev) = rx.try_recv() {
                self.update_checking = false;
                self.update_check_rx = None;
                let lang = self.lang();
                match ev.result {
                    Ok(Some(info)) => {
                        let skipped = !ev.manual
                            && self.config.skipped_version.as_deref() == Some(info.tag.as_str());
                        if !skipped {
                            self.update_available = Some(info);
                            self.update_error = None;
                        }
                    }
                    Ok(None) => {
                        if ev.manual {
                            self.update_available = None;
                            self.message = Some(StatusMessage::Info(lang.fmt_already_latest(APP_VERSION)));
                        }
                    }
                    Err(e) => {
                        if ev.manual {
                            self.message = Some(StatusMessage::Error(lang.fmt_update_check_failed(&e)));
                        }
                    }
                }
            }
        }

        let mut close_app = false;
        if let Some(rx) = &self.update_install_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    UpdateInstallEvent::Progress(p) => {
                        self.update_progress = Some(p);
                    }
                    UpdateInstallEvent::Installed => {
                        self.update_progress = None;
                        close_app = true;
                    }
                    UpdateInstallEvent::Failed(err) => {
                        self.update_progress = None;
                        self.update_error = Some(err);
                    }
                }
            }
        }
        if close_app {
            self.update_install_rx = None;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.update_progress.is_none() && self.update_error.is_some() {
            self.update_install_rx = None;
        }
    }

    fn install_update(&mut self, info: UpdateInfo) {
        if self.update_progress.is_some() {
            return;
        }
        self.update_progress = Some(0.0);
        self.update_error = None;
        let (tx, rx) = mpsc::channel();
        self.update_install_rx = Some(rx);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let res = update::download(&info, |p| {
                let _ = tx.send(UpdateInstallEvent::Progress(p));
                ctx.request_repaint();
            })
            .and_then(|path| update::run_installer(&path));
            match res {
                Ok(()) => {
                    let _ = tx.send(UpdateInstallEvent::Installed);
                }
                Err(e) => {
                    let _ = tx.send(UpdateInstallEvent::Failed(e));
                }
            }
            ctx.request_repaint();
        });
    }

    fn skip_update(&mut self, info: &UpdateInfo) {
        self.config.skipped_version = Some(info.tag.clone());
        let _ = self.config.save();
        self.update_available = None;
        self.update_error = None;
    }

    fn refresh_backups(&mut self) {
        let dir = backup::default_backup_dir();
        self.backups = backup::list_backups(&dir);
        if self
            .selected_backup_id
            .as_deref()
            .is_none_or(|id| !self.backups.iter().any(|b| b.id == id))
        {
            self.selected_backup_id = self.backups.first().map(|b| b.id.clone());
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let lang = self.lang();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches_filter(e, &self.filter, lang))
            .map(|(i, _)| i)
            .collect()
    }

    fn checked_entries(&self) -> Vec<&ContextEntry> {
        self.entries
            .iter()
            .filter(|e| self.checked_ids.contains(&e.id))
            .collect()
    }

    fn category_counts(&self, cat: Category) -> (usize, usize) {
        let mut total = 0;
        let mut orphaned = 0;
        for e in &self.entries {
            if self.filter.hide_system && e.is_system && !e.is_orphaned() {
                continue;
            }
            if cat == Category::All || e.category == cat {
                total += 1;
                if e.is_orphaned() {
                    orphaned += 1;
                }
            }
        }
        (total, orphaned)
    }

    fn apply_toggle(&mut self, disable: bool) {
        let lang = self.lang();
        let targets = self.checked_entries();
        if targets.is_empty() {
            self.message = Some(StatusMessage::Warn(lang.msg_select_first().into()));
            return;
        }
        let needs_explorer_hint = targets
            .iter()
            .any(|e| matches!(e.kind, EntryKind::ShellEx | EntryKind::Win11Packaged));
        let related_remaining = find_related_unchecked_indices(&self.entries, &self.checked_ids).len();
        let report = action::set_entries_disabled(&targets, disable);
        let verb = if disable { lang.verb_disabled() } else { lang.verb_enabled() };
        self.report_to_message(verb, &report, None);
        if report.errors.is_empty() {
            if let Some(StatusMessage::Info(msg)) = &mut self.message {
                if related_remaining > 0 {
                    msg.push_str(&lang.fmt_related_remaining_note(related_remaining));
                }
                if needs_explorer_hint {
                    msg.push_str(lang.explorer_hint_note());
                }
            }
        }
        self.start_scan();
    }

    fn apply_delete(&mut self, include_related: bool) {
        let lang = self.lang();
        if include_related {
            let related = find_related_unchecked_indices(&self.entries, &self.checked_ids);
            for idx in related {
                self.checked_ids.insert(self.entries[idx].id.clone());
            }
        }
        let targets = self.checked_entries();
        if targets.is_empty() {
            return;
        }
        let backup_dir = backup::default_backup_dir();
        match action::delete_entries_with_backup(&targets, &backup_dir) {
            Ok((rec, report)) => {
                self.checked_ids.clear();
                self.report_to_message(lang.verb_removed(), &report, Some(&rec.id));
                self.start_scan();
            }
            Err(e) => {
                self.message = Some(StatusMessage::Error(lang.fmt_backup_or_delete_failed(&e)));
            }
        }
    }

    fn report_to_message(&mut self, verb: &str, report: &ActionReport, backup_id: Option<&str>) {
        let lang = self.lang();
        if report.errors.is_empty() {
            let mut msg = lang.fmt_action_ok(report.succeeded, verb, backup_id);
            if !report.notes.is_empty() {
                msg.push_str(&format!(" / {}", report.notes[0]));
            }
            self.message = Some(StatusMessage::Info(msg));
        } else {
            let first_err = &report.errors[0];
            self.message = Some(StatusMessage::Error(lang.fmt_action_partial_err(
                report.succeeded,
                report.errors.len(),
                first_err,
            )));
        }
    }

    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let Some(info) = self.update_available.clone() else {
            return;
        };
        let lang = self.lang();
        let busy = self.update_progress.is_some();
        let mut do_install = false;
        let mut do_skip = false;

        egui::Panel::top("update_banner").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.strong(lang.fmt_update_available(&info.version_text()));
                ui.weak(lang.fmt_current_version(APP_VERSION));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!busy, egui::Button::new(lang.btn_skip_version()))
                        .clicked()
                    {
                        do_skip = true;
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new(lang.btn_install_now()))
                        .clicked()
                    {
                        do_install = true;
                    }
                });
            });
            if let Some(p) = self.update_progress {
                ui.add(
                    egui::ProgressBar::new(p)
                        .show_percentage()
                        .text(lang.fmt_downloading_update(p * 100.0)),
                );
            }
            if let Some(ref err) = self.update_error {
                ui.colored_label(ui.visuals().error_fg_color, lang.fmt_update_failed(err));
            }
            let notes = info.notes.trim();
            if !notes.is_empty() {
                egui::CollapsingHeader::new(lang.release_notes_header()).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
                        ui.label(notes);
                    });
                });
            }
            ui.add_space(4.0);
        });

        if do_install {
            self.install_update(info);
        } else if do_skip {
            self.skip_update(&info);
        }
    }

    fn top_toolbar(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let mut next_lang = lang;
        let mut do_check_updates = false;

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            // 1줄: 검색창 + 우측 주요 도구 버튼 및 언어 선택
            ui.horizontal(|ui| {
                ui.label(lang.search_label());
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter.search)
                        .hint_text(lang.search_hint())
                        .desired_width(280.0),
                );
                if !self.filter.search.is_empty() && ui.button(lang.btn_clear()).clicked() {
                    self.filter.search.clear();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("lang_selector")
                        .selected_text(lang.native_name())
                        .width(98.0)
                        .show_ui(ui, |ui| {
                            for &l in Language::ALL {
                                ui.selectable_value(&mut next_lang, l, l.native_name());
                            }
                        });

                    if ui
                        .add_enabled(
                            !self.update_checking && self.update_progress.is_none(),
                            egui::Button::new(if self.update_checking {
                                lang.checking_updates()
                            } else {
                                lang.btn_check_updates()
                            }),
                        )
                        .clicked()
                    {
                        do_check_updates = true;
                    }

                    if ui.button(lang.btn_backup_restore()).clicked() {
                        self.refresh_backups();
                        self.show_backup_modal = true;
                    }
                    if ui.button(lang.btn_add_shell()).clicked() {
                        let mut spec = ShellCommandSpec::default();
                        // 현재 선택된 카테고리에 맞춰 기본 대상 자동 설정
                        spec.target_kind = match self.filter.category {
                            Category::Directory => ShellTargetKind::Directory,
                            Category::Background => ShellTargetKind::DirectoryBackground,
                            Category::Drive => ShellTargetKind::Drive,
                            Category::FileExtension => ShellTargetKind::CustomExtension,
                            _ => ShellTargetKind::AllFiles,
                        };
                        self.shell_editor = Some(spec);
                        self.shell_editor_error = None;
                    }
                    let refresh_btn = ui.add_enabled(!self.scanning, egui::Button::new(lang.btn_refresh()));
                    if refresh_btn.clicked() {
                        self.start_scan();
                    }
                    if self.scanning {
                        ui.spinner();
                        ui.weak(lang.scanning_label());
                    }
                });
            });

            ui.add_space(4.0);
            // 2줄: 필터 체크박스 + 우측 보기 모드 전환
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.filter.only_orphaned, lang.filter_orphaned())
                    .on_hover_text(lang.filter_orphaned_tip());
                ui.checkbox(&mut self.filter.only_disabled, lang.filter_disabled());
                ui.checkbox(&mut self.filter.hide_system, lang.filter_hide_system())
                    .on_hover_text(lang.filter_hide_system_tip());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.selectable_value(&mut self.view_mode, ViewMode::Flat, lang.view_flat());
                    ui.selectable_value(&mut self.view_mode, ViewMode::Grouped, lang.view_grouped())
                        .on_hover_text(lang.view_grouped_tip());
                });
            });
            ui.add_space(4.0);
        });

        if next_lang != lang {
            self.set_language(next_lang);
        }
        if do_check_updates {
            self.start_update_check(true);
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        egui::Panel::left("categories")
            .resizable(true)
            .default_size(225.0)
            .min_size(180.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.strong(lang.sidebar_title());
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for &cat in Category::SIDEBAR {
                        let (total, orphaned) = self.category_counts(cat);
                        let selected = self.filter.category == cat;
                        let label = if orphaned > 0 {
                            format!(
                                "{}  ({total})  {}",
                                cat.label(lang),
                                lang.fmt_sidebar_orphan_badge(orphaned)
                            )
                        } else {
                            format!("{}  ({total})", cat.label(lang))
                        };
                        let mut text = egui::RichText::new(label);
                        if orphaned > 0 && !selected {
                            text = text.color(ui.visuals().warn_fg_color);
                        }
                        if ui
                            .add_sized([ui.available_width(), 26.0], egui::Button::selectable(selected, text))
                            .clicked()
                        {
                            self.filter.category = cat;
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.weak(format!("{APP_NAME} v{APP_VERSION}"));
                    ui.weak(lang.fmt_os_label(self.os_build));
                    if let Some(ms) = self.last_scan_ms {
                        ui.weak(lang.fmt_scan_stats(self.entries.len(), ms));
                    }
                });
            });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let selected_count = self.checked_ids.len();
        let mut do_disable = false;
        let mut do_enable = false;
        let mut do_delete = false;
        let mut do_elevate = false;
        let mut do_restart_explorer = false;

        egui::Panel::bottom("actions_bar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.strong(lang.fmt_selected_count(selected_count));
                ui.separator();
                if ui
                    .add_enabled(
                        selected_count > 0 && !self.scanning,
                        egui::Button::new(lang.btn_disable()),
                    )
                    .on_hover_text(lang.btn_disable_tip())
                    .clicked()
                {
                    do_disable = true;
                }
                if ui
                    .add_enabled(
                        selected_count > 0 && !self.scanning,
                        egui::Button::new(lang.btn_enable()),
                    )
                    .clicked()
                {
                    do_enable = true;
                }
                let del_text = egui::RichText::new(lang.btn_remove()).color(if selected_count > 0 {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().weak_text_color()
                });
                if ui
                    .add_enabled(selected_count > 0 && !self.scanning, egui::Button::new(del_text))
                    .on_hover_text(lang.btn_remove_tip())
                    .clicked()
                {
                    do_delete = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(lang.btn_restart_explorer())
                        .on_hover_text(lang.btn_restart_explorer_tip())
                        .clicked()
                    {
                        do_restart_explorer = true;
                    }
                    ui.separator();
                    if self.is_elevated {
                        ui.colored_label(egui::Color32::from_rgb(34, 197, 94), lang.badge_admin());
                    } else {
                        if ui
                            .button(lang.btn_relaunch_admin())
                            .on_hover_text(lang.btn_relaunch_admin_tip())
                            .clicked()
                        {
                            do_elevate = true;
                        }
                        ui.weak(lang.badge_standard_user());
                    }
                });
            });

            if let Some(msg) = &self.message {
                ui.add_space(4.0);
                match msg {
                    StatusMessage::Info(s) => {
                        ui.label(s);
                    }
                    StatusMessage::Warn(s) => {
                        ui.colored_label(ui.visuals().warn_fg_color, s);
                    }
                    StatusMessage::Error(s) => {
                        ui.colored_label(ui.visuals().error_fg_color, s);
                    }
                }
            }
            ui.add_space(4.0);
        });

        if do_disable {
            self.apply_toggle(true);
        } else if do_enable {
            self.apply_toggle(false);
        } else if do_delete {
            self.include_related_on_delete = false;
            self.show_delete_modal = true;
        } else if do_elevate {
            match system::relaunch_as_admin() {
                Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.message = Some(StatusMessage::Error(e)),
            }
        } else if do_restart_explorer {
            self.show_restart_explorer_modal = true;
        }
    }

    fn central_list(
        &mut self,
        ui: &mut egui::Ui,
        filtered: &[usize],
        groups: &[EntryGroup],
        group_totals: &HashMap<String, usize>,
    ) {
        let lang = self.lang();
        let orphan_in_filtered: Vec<usize> = filtered
            .iter()
            .copied()
            .filter(|&i| self.entries[i].is_orphaned())
            .collect();

        ui.horizontal_wrapped(|ui| {
            if ui.small_button(lang.btn_select_all_visible()).clicked() {
                for &i in filtered {
                    self.checked_ids.insert(self.entries[i].id.clone());
                }
            }
            if ui
                .add_enabled(
                    !orphan_in_filtered.is_empty(),
                    egui::Button::new(lang.btn_select_all_orphans()).small(),
                )
                .clicked()
            {
                for &i in &orphan_in_filtered {
                    self.checked_ids.insert(self.entries[i].id.clone());
                }
            }
            if ui
                .add_enabled(
                    !self.checked_ids.is_empty(),
                    egui::Button::new(lang.btn_deselect_all()).small(),
                )
                .clicked()
            {
                self.checked_ids.clear();
            }
            ui.separator();
            ui.weak(lang.fmt_list_summary(filtered.len(), groups.len(), orphan_in_filtered.len()));
        });
        ui.separator();

        if self.scanning && self.entries.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.add_space(6.0);
                ui.label(lang.scanning_empty_text());
            });
            return;
        }

        if filtered.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.weak(lang.no_matching_entries());
            });
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("main_list_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match self.view_mode {
                ViewMode::Grouped => self.render_grouped_list(ui, groups, group_totals),
                ViewMode::Flat => self.render_flat_list(ui, filtered),
            });
    }

    fn render_grouped_list(
        &mut self,
        ui: &mut egui::Ui,
        groups: &[EntryGroup],
        group_totals: &HashMap<String, usize>,
    ) {
        let lang = self.lang();
        let error_color = ui.visuals().error_fg_color;
        let warn_color = ui.visuals().warn_fg_color;

        for group in groups {
            let checked_cnt = group
                .entry_indices
                .iter()
                .filter(|&&i| self.checked_ids.contains(&self.entries[i].id))
                .count();
            let all_checked = checked_cnt == group.entry_indices.len();
            let mut is_checked = all_checked;

            let is_focused = matches!(&self.focused, Some(FocusedTarget::Group(k)) if k == &group.key);

            let frame = egui::Frame::group(ui.style())
                .fill(if is_focused {
                    ui.visuals().selection.bg_fill.linear_multiply(0.35)
                } else {
                    ui.visuals().faint_bg_color
                })
                .inner_margin(egui::Margin::symmetric(8, 6));

            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut is_checked, "").changed() {
                        if is_checked {
                            for &i in &group.entry_indices {
                                self.checked_ids.insert(self.entries[i].id.clone());
                            }
                        } else {
                            for &i in &group.entry_indices {
                                self.checked_ids.remove(&self.entries[i].id);
                            }
                        }
                    }

                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut title = egui::RichText::new(&group.display_name).strong();
                            if group.all_disabled {
                                title = title.weak().strikethrough();
                            }
                            if ui.add(egui::Button::selectable(is_focused, title)).clicked() {
                                self.focused = Some(FocusedTarget::Group(group.key.clone()));
                            }

                            if group.all_orphaned {
                                ui.colored_label(error_color, lang.badge_all_orphaned());
                            } else if group.any_orphaned {
                                ui.colored_label(warn_color, lang.badge_some_orphaned());
                            }

                            if group.all_disabled {
                                ui.weak(lang.badge_disabled());
                            } else if group.any_disabled {
                                ui.weak(lang.badge_some_disabled());
                            }

                            if group.is_system {
                                ui.weak(lang.badge_system());
                            }

                            let shown_cnt = group.entry_indices.len();
                            let total_cnt = group_totals.get(&group.key).copied().unwrap_or(shown_cnt);
                            if total_cnt > shown_cnt {
                                ui.colored_label(warn_color, lang.fmt_group_partial_total(shown_cnt, total_cnt))
                                    .on_hover_text(lang.group_partial_tip());
                            } else if shown_cnt > 1 {
                                ui.weak(lang.fmt_group_count(shown_cnt));
                            }
                            if checked_cnt > 0 && !all_checked {
                                ui.colored_label(warn_color, lang.fmt_partial_selected(checked_cnt, shown_cnt));
                            }
                        });

                        ui.horizontal_wrapped(|ui| {
                            ui.weak(format!(
                                "{} | {} | {}",
                                group.kind.label(lang),
                                group.scope_label(),
                                group.summary_targets
                            ));
                            if let Some(ref target) = group.resolved_target {
                                ui.weak("|");
                                ui.weak(truncate_middle(target, 64));
                            }
                        });
                    });
                });
            });
            ui.add_space(3.0);
        }
    }

    fn render_flat_list(&mut self, ui: &mut egui::Ui, filtered: &[usize]) {
        let lang = self.lang();
        let error_color = ui.visuals().error_fg_color;

        for &idx in filtered {
            let entry = &self.entries[idx];
            let mut is_checked = self.checked_ids.contains(&entry.id);
            let is_focused = matches!(&self.focused, Some(FocusedTarget::Entry(id)) if id == &entry.id);

            let frame = egui::Frame::group(ui.style())
                .fill(if is_focused {
                    ui.visuals().selection.bg_fill.linear_multiply(0.35)
                } else {
                    ui.visuals().faint_bg_color
                })
                .inner_margin(egui::Margin::symmetric(8, 5));

            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut is_checked, "").changed() {
                        if is_checked {
                            self.checked_ids.insert(entry.id.clone());
                        } else {
                            self.checked_ids.remove(&entry.id);
                        }
                    }

                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut title = egui::RichText::new(&entry.display_name).strong();
                            if entry.disabled {
                                title = title.weak().strikethrough();
                            }
                            if ui.add(egui::Button::selectable(is_focused, title)).clicked() {
                                self.focused = Some(FocusedTarget::Entry(entry.id.clone()));
                            }
                            if entry.is_orphaned() {
                                ui.colored_label(error_color, lang.badge_orphaned_short());
                            }
                            if entry.disabled {
                                ui.weak(lang.badge_disabled());
                            }
                            if entry.is_system {
                                ui.weak(lang.badge_system());
                            }
                        });
                        ui.weak(format!(
                            "{} | {} | {} | {}",
                            entry.kind.label(lang),
                            entry.scope.short_label(),
                            entry.target_text(lang),
                            truncate_middle(&entry.location_display(), 70)
                        ));
                    });
                });
            });
            ui.add_space(2.0);
        }
    }

    fn detail_panel(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        egui::Panel::right("detail_panel")
            .resizable(true)
            .default_size(365.0)
            .min_size(270.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.strong(lang.detail_title());
                ui.separator();

                let target_indices: Vec<usize> = match &self.focused {
                    Some(FocusedTarget::Group(gkey)) => self
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| &e.group_key == gkey)
                        .map(|(i, _)| i)
                        .collect(),
                    Some(FocusedTarget::Entry(id)) => self
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| &e.id == id)
                        .map(|(i, _)| i)
                        .collect(),
                    None => Vec::new(),
                };

                if target_indices.is_empty() {
                    ui.add_space(24.0);
                    ui.weak(lang.detail_empty_hint());
                    return;
                }

                let first = self.entries[target_indices[0]].clone();

                egui::ScrollArea::vertical()
                    .id_salt("detail_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.heading(&first.display_name);
                        ui.add_space(4.0);

                        egui::Grid::new("detail_meta")
                            .num_columns(2)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                ui.weak(lang.meta_kind());
                                ui.label(first.kind.label(lang));
                                ui.end_row();

                                ui.weak(lang.meta_scope());
                                ui.label(first.scope.label(lang));
                                ui.end_row();

                                ui.weak(lang.meta_file_status());
                                match first.target_exists {
                                    Some(true) => {
                                        ui.colored_label(
                                            egui::Color32::from_rgb(34, 197, 94),
                                            lang.file_exists_ok(),
                                        );
                                    }
                                    Some(false) => {
                                        ui.colored_label(
                                            ui.visuals().error_fg_color,
                                            lang.file_missing_orphan(),
                                        );
                                    }
                                    None => {
                                        ui.weak(lang.file_status_none());
                                    }
                                }
                                ui.end_row();

                                if let Some(ref clsid) = first.clsid {
                                    ui.weak("CLSID");
                                    ui.monospace(clsid);
                                    ui.end_row();
                                }
                                if let Some(ref pkg) = first.package_name {
                                    ui.weak(lang.meta_package());
                                    ui.label(pkg);
                                    ui.end_row();
                                }
                            });

                        if let Some(ref target) = first.resolved_target {
                            ui.add_space(6.0);
                            ui.weak(lang.target_exe_label());
                            ui.monospace(target);
                            ui.horizontal(|ui| {
                                if ui.small_button(lang.btn_copy_path()).clicked() {
                                    ui.ctx().copy_text(target.clone());
                                    self.message = Some(StatusMessage::Info(lang.msg_copied_path().into()));
                                }
                                if ui.small_button(lang.btn_open_file_location()).clicked() {
                                    if let Err(e) = system::open_in_explorer(target) {
                                        self.message = Some(StatusMessage::Error(e));
                                    }
                                }
                            });
                        }

                        if let Some(ref cmd) = first.command {
                            ui.add_space(6.0);
                            ui.weak(lang.command_label());
                            ui.monospace(cmd);
                        }

                        if let Some(ref icon) = first.icon {
                            ui.add_space(4.0);
                            ui.weak(lang.icon_label());
                            ui.monospace(icon);
                        }

                        ui.add_space(10.0);
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(lang.fmt_registered_locations(target_indices.len()));
                            if ui
                                .small_button(lang.fmt_check_all_locations(target_indices.len()))
                                .clicked()
                            {
                                for &i in &target_indices {
                                    self.checked_ids.insert(self.entries[i].id.clone());
                                }
                            }
                            if ui.small_button(lang.btn_uncheck_all()).clicked() {
                                for &i in &target_indices {
                                    self.checked_ids.remove(&self.entries[i].id);
                                }
                            }
                        });
                        ui.add_space(4.0);

                        for &i in &target_indices {
                            let e = &self.entries[i];
                            let mut checked = self.checked_ids.contains(&e.id);
                            let outside_current_cat =
                                self.filter.category != Category::All && e.category != self.filter.category;
                            let full_reg = e
                                .registry_root
                                .zip(e.registry_subkey.as_deref())
                                .map(|(r, s)| r.format_full_path(s));
                            let loc_display = e.location_display();

                            egui::Frame::group(ui.style())
                                .inner_margin(egui::Margin::symmetric(6, 4))
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.checkbox(&mut checked, "").changed() {
                                            if checked {
                                                self.checked_ids.insert(e.id.clone());
                                            } else {
                                                self.checked_ids.remove(&e.id);
                                            }
                                        }
                                        ui.strong(e.target_text(lang));
                                        ui.weak(format!("[{}]", e.scope.short_label()));
                                        if outside_current_cat {
                                            ui.colored_label(
                                                ui.visuals().warn_fg_color,
                                                lang.fmt_other_category_badge(e.category.label(lang)),
                                            );
                                        }
                                        if e.disabled {
                                            ui.weak(lang.tag_disabled_paren());
                                        }
                                        if e.is_orphaned() {
                                            ui.colored_label(
                                                ui.visuals().error_fg_color,
                                                lang.tag_missing_paren(),
                                            );
                                        }
                                    });
                                    ui.monospace(&loc_display);
                                    ui.horizontal(|ui| {
                                        if ui.small_button(lang.btn_copy_path()).clicked() {
                                            ui.ctx().copy_text(loc_display.clone());
                                            self.message =
                                                Some(StatusMessage::Info(lang.msg_copied_path().into()));
                                        }
                                        if let Some(ref reg_path) = full_reg {
                                            if e.kind == EntryKind::ShellVerb {
                                                if ui.small_button(lang.btn_edit()).clicked() {
                                                    if let (Some(root), Some(subkey)) =
                                                        (e.registry_root, e.registry_subkey.as_deref())
                                                    {
                                                        match action::load_shell_command(root, subkey) {
                                                            Ok(spec) => {
                                                                self.shell_editor = Some(spec);
                                                                self.shell_editor_error = None;
                                                            }
                                                            Err(err) => {
                                                                self.message = Some(StatusMessage::Error(err));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            if ui.small_button(lang.btn_open_regedit()).clicked() {
                                                if let Err(err) = system::open_regedit_at(reg_path) {
                                                    self.message = Some(StatusMessage::Error(err));
                                                }
                                            }
                                        } else if let Some(ref fp) =
                                            e.disabled_file_path.as_ref().or(e.file_path.as_ref())
                                        {
                                            if ui.small_button(lang.btn_open_folder()).clicked() {
                                                if let Err(err) = system::open_in_explorer(fp) {
                                                    self.message = Some(StatusMessage::Error(err));
                                                }
                                            }
                                        }
                                    });
                                });
                            ui.add_space(3.0);
                        }
                    });
            });
    }

    fn delete_confirm_modal(&mut self, ctx: &egui::Context) {
        if !self.show_delete_modal {
            return;
        }
        let lang = self.lang();
        let related_indices = find_related_unchecked_indices(&self.entries, &self.checked_ids);
        let related_count = related_indices.len();
        if related_count == 0 {
            self.include_related_on_delete = false;
        }

        let related_summary = if related_count > 0 {
            let mut cat_set: BTreeSet<&str> = BTreeSet::new();
            for &idx in &related_indices {
                cat_set.insert(self.entries[idx].category.label(lang));
            }
            cat_set.into_iter().collect::<Vec<_>>().join(", ")
        } else {
            String::new()
        };

        let mut effective_targets = self.checked_entries();
        if self.include_related_on_delete {
            for &idx in &related_indices {
                effective_targets.push(&self.entries[idx]);
            }
        }

        let base_count = self.checked_ids.len();
        let total = effective_targets.len();
        let admin_needed = effective_targets.iter().filter(|e| e.requires_admin_to_delete()).count();
        let win11_count = effective_targets
            .iter()
            .filter(|e| e.kind == EntryKind::Win11Packaged)
            .count();
        let preview_names: Vec<String> = effective_targets
            .iter()
            .take(10)
            .map(|e| format!("- {} [{}] ({})", e.display_name, e.target_text(lang), e.scope.short_label()))
            .collect();

        let mut confirm = false;
        let mut cancel = false;
        let mut elevate = false;
        let mut include_related = self.include_related_on_delete;

        let resp = egui::Modal::new(egui::Id::unique("delete_confirm_modal")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(lang.modal_delete_title());
            ui.add_space(6.0);
            ui.label(lang.fmt_modal_delete_desc(total));
            ui.weak(lang.modal_delete_backup_hint());

            if related_count > 0 {
                ui.add_space(8.0);
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().faint_bg_color)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            lang.fmt_modal_related_warn(related_count),
                        );
                        ui.weak(lang.fmt_modal_related_cats(&related_summary));
                        ui.add_space(4.0);
                        ui.checkbox(
                            &mut include_related,
                            lang.fmt_modal_include_related(related_count, base_count + related_count),
                        );
                    });
            }

            if win11_count > 0 {
                ui.add_space(4.0);
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    lang.fmt_modal_win11_note(win11_count),
                );
            }

            if admin_needed > 0 && !self.is_elevated {
                ui.add_space(4.0);
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    lang.fmt_modal_admin_warn(admin_needed),
                );
            }

            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for line in &preview_names {
                    ui.label(line);
                }
                if total > preview_names.len() {
                    ui.weak(lang.fmt_and_more(total - preview_names.len()));
                }
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let btn_text = egui::RichText::new(lang.fmt_btn_backup_and_remove(total))
                    .color(ui.visuals().error_fg_color);
                if ui.button(btn_text).clicked() {
                    confirm = true;
                }
                if admin_needed > 0 && !self.is_elevated && ui.button(lang.btn_relaunch_admin()).clicked() {
                    elevate = true;
                }
                if ui.button(lang.btn_cancel()).clicked() {
                    cancel = true;
                }
            });
        });

        self.include_related_on_delete = include_related;

        if confirm {
            self.show_delete_modal = false;
            self.apply_delete(self.include_related_on_delete);
            self.include_related_on_delete = false;
        } else if elevate {
            self.show_delete_modal = false;
            self.include_related_on_delete = false;
            match system::relaunch_as_admin() {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.message = Some(StatusMessage::Error(e)),
            }
        } else if cancel || resp.should_close() {
            self.show_delete_modal = false;
            self.include_related_on_delete = false;
        }
    }

    fn backup_modal(&mut self, ctx: &egui::Context) {
        if !self.show_backup_modal {
            return;
        }
        let lang = self.lang();
        let mut close = false;
        let mut restore_record: Option<BackupRecord> = None;
        let mut delete_record: Option<BackupRecord> = None;
        let mut open_folder = false;

        let resp = egui::Modal::new(egui::Id::unique("backup_modal")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(lang.modal_backup_title());
            ui.add_space(4.0);
            ui.weak(lang.fmt_backup_location(&backup::default_backup_dir().display().to_string()));
            ui.add_space(6.0);

            if self.backups.is_empty() {
                ui.add_space(16.0);
                ui.weak(lang.backup_empty());
                ui.add_space(16.0);
            } else {
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    for rec in &self.backups {
                        let selected = self.selected_backup_id.as_deref() == Some(rec.id.as_str());
                        let label = format!("{}  |  {} ({})", rec.created_at, rec.summary, rec.items.len());
                        if ui
                            .add_sized([ui.available_width(), 24.0], egui::Button::selectable(selected, label))
                            .clicked()
                        {
                            self.selected_backup_id = Some(rec.id.clone());
                        }
                    }
                });

                if let Some(sel_id) = &self.selected_backup_id {
                    if let Some(rec) = self.backups.iter().find(|b| &b.id == sel_id) {
                        ui.separator();
                        ui.weak(lang.fmt_backup_items_header(rec.items.len()));
                        egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
                            for item in &rec.items {
                                let desc = match item {
                                    backup::BackupItem::Registry {
                                        root,
                                        subkey,
                                        display_name,
                                        ..
                                    } => format!("- {display_name} ({})", root.format_short_path(subkey)),
                                    backup::BackupItem::File {
                                        original_path,
                                        display_name,
                                        ..
                                    } => format!("- {display_name} ({original_path})"),
                                    backup::BackupItem::Win11Block { clsid, display_name } => {
                                        format!("- {display_name} (Win11 Unblock: {clsid})")
                                    }
                                };
                                ui.label(desc);
                            }
                        });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button(lang.btn_restore_backup()).clicked() {
                                restore_record = Some(rec.clone());
                            }
                            if ui.button(lang.btn_delete_backup()).clicked() {
                                delete_record = Some(rec.clone());
                            }
                        });
                    }
                }
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(lang.btn_open_backup_folder()).clicked() {
                    open_folder = true;
                }
                if ui.button(lang.btn_close()).clicked() {
                    close = true;
                }
            });
        });

        if open_folder {
            let dir = backup::default_backup_dir();
            let _ = std::fs::create_dir_all(&dir);
            if let Err(e) = system::open_in_explorer(&dir.to_string_lossy()) {
                self.message = Some(StatusMessage::Error(e));
            }
        }
        if let Some(rec) = restore_record {
            let dir = backup::default_backup_dir();
            let rep = backup::restore_backup(&rec, &dir);
            if rep.errors.is_empty() {
                self.message = Some(StatusMessage::Info(lang.fmt_restore_ok(&rec.created_at, rep.succeeded)));
            } else {
                self.message = Some(StatusMessage::Error(lang.fmt_restore_err(
                    rep.succeeded,
                    rep.errors.len(),
                    &rep.errors[0],
                )));
            }
            self.show_backup_modal = false;
            self.start_scan();
        } else if let Some(rec) = delete_record {
            let dir = backup::default_backup_dir();
            if let Err(e) = backup::delete_backup_record(&rec, &dir) {
                self.message = Some(StatusMessage::Error(e));
            }
            self.refresh_backups();
        } else if close || resp.should_close() {
            self.show_backup_modal = false;
        }
    }

    fn restart_explorer_modal(&mut self, ctx: &egui::Context) {
        if !self.show_restart_explorer_modal {
            return;
        }
        let lang = self.lang();
        let mut confirm = false;
        let mut cancel = false;
        let resp = egui::Modal::new(egui::Id::unique("restart_explorer_modal")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading(lang.modal_explorer_title());
            ui.add_space(6.0);
            ui.label(lang.modal_explorer_desc());
            ui.weak(lang.modal_explorer_hint());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(lang.btn_restart_now()).clicked() {
                    confirm = true;
                }
                if ui.button(lang.btn_cancel()).clicked() {
                    cancel = true;
                }
            });
        });

        if confirm {
            self.show_restart_explorer_modal = false;
            match system::restart_explorer() {
                Ok(()) => {
                    self.message = Some(StatusMessage::Info(lang.msg_explorer_restarted().into()));
                }
                Err(e) => {
                    self.message = Some(StatusMessage::Error(e));
                }
            }
        } else if cancel || resp.should_close() {
            self.show_restart_explorer_modal = false;
        }
    }

    fn shell_command_modal(&mut self, ctx: &egui::Context) {
        let Some(mut spec) = self.shell_editor.clone() else {
            return;
        };
        let lang = self.lang();
        let is_edit = spec.existing_subkey.is_some();
        let mut do_save = false;
        let mut do_cancel = false;

        let target_label = |k: ShellTargetKind| -> &'static str {
            match k {
                ShellTargetKind::AllFiles => lang.tgt_all_files(),
                ShellTargetKind::Directory => lang.tgt_directory(),
                ShellTargetKind::DirectoryBackground => lang.tgt_dir_bg(),
                ShellTargetKind::AllFilesystemObjects => lang.tgt_all_fs(),
                ShellTargetKind::Drive => lang.tgt_drive(),
                ShellTargetKind::DesktopBackground => lang.tgt_desktop_bg(),
                ShellTargetKind::CustomExtension => lang.shell_target_ext(),
            }
        };
        let pos_label = |p: ShellMenuPosition| -> &'static str {
            match p {
                ShellMenuPosition::Default => lang.pos_default(),
                ShellMenuPosition::Top => lang.pos_top(),
                ShellMenuPosition::Bottom => lang.pos_bottom(),
            }
        };

        let resp = egui::Modal::new(egui::Id::unique("shell_command_modal")).show(ctx, |ui| {
            ui.set_width(580.0);
            ui.heading(if is_edit {
                lang.modal_edit_shell_title()
            } else {
                lang.modal_add_shell_title()
            });
            ui.add_space(6.0);

            egui::Grid::new("shell_editor_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    if !is_edit {
                        ui.label(lang.shell_target_label());
                        egui::ComboBox::from_id_salt("shell_target_kind")
                            .selected_text(target_label(spec.target_kind))
                            .width(360.0)
                            .show_ui(ui, |ui| {
                                for &k in ShellTargetKind::ALL {
                                    ui.selectable_value(&mut spec.target_kind, k, target_label(k));
                                }
                            });
                        ui.end_row();

                        if spec.target_kind == ShellTargetKind::CustomExtension {
                            ui.label(lang.shell_ext_label());
                            ui.add(
                                egui::TextEdit::singleline(&mut spec.custom_extension)
                                    .hint_text(lang.shell_ext_hint())
                                    .desired_width(220.0),
                            );
                            ui.end_row();
                        }

                        ui.label(lang.shell_scope_label());
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut spec.root, RegRoot::Hkcu, lang.scope_user());
                            ui.radio_value(&mut spec.root, RegRoot::Hklm, lang.scope_system());
                        });
                        ui.end_row();
                    }

                    ui.label(lang.shell_display_name_label());
                    ui.add(
                        egui::TextEdit::singleline(&mut spec.display_name)
                            .hint_text(lang.shell_display_name_hint())
                            .desired_width(400.0),
                    );
                    ui.end_row();

                    if !is_edit {
                        ui.label(lang.shell_key_name_label());
                        ui.add(
                            egui::TextEdit::singleline(&mut spec.key_name)
                                .hint_text(lang.shell_key_name_hint())
                                .desired_width(400.0),
                        );
                        ui.end_row();
                    }

                    ui.label(lang.shell_command_label());
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut spec.command)
                                    .hint_text(lang.shell_command_hint())
                                    .desired_width(325.0),
                            );
                            if ui.button(lang.btn_browse()).clicked() {
                                if let Some(path) = dialog::pick_command_file(lang.dialog_pick_command_title()) {
                                    let p_str = path.to_string_lossy();
                                    let ph = spec.target_kind.default_placeholder();
                                    let ext = path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("")
                                        .to_ascii_lowercase();
                                    spec.command = if ext == "bat" || ext == "cmd" {
                                        format!("cmd /k \"{p_str}\" \"{ph}\"")
                                    } else {
                                        format!("\"{p_str}\" \"{ph}\"")
                                    };
                                    if spec.display_name.trim().is_empty() {
                                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                            spec.display_name = stem.to_string();
                                        }
                                    }
                                }
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.weak(lang.shell_helpers_label());
                            if ui.small_button("+\"%1\"").clicked() {
                                append_token(&mut spec.command, "\"%1\"");
                            }
                            if ui.small_button("+\"%V\"").clicked() {
                                append_token(&mut spec.command, "\"%V\"");
                            }
                            if ui.small_button("+cmd /k").clicked() {
                                prepend_prefix(&mut spec.command, "cmd /k ");
                            }
                            if ui.small_button("+cmd /c").clicked() {
                                prepend_prefix(&mut spec.command, "cmd /c ");
                            }
                        });
                    });
                    ui.end_row();

                    ui.label(lang.shell_icon_label());
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut spec.icon)
                                .hint_text(lang.shell_icon_hint())
                                .desired_width(325.0),
                        );
                        if ui.button(lang.btn_browse()).clicked() {
                            if let Some(path) = dialog::pick_icon_file(lang.dialog_pick_icon_title()) {
                                spec.icon = path.to_string_lossy().into_owned();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label(lang.shell_position_label());
                    egui::ComboBox::from_id_salt("shell_menu_position")
                        .selected_text(pos_label(spec.position))
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            for &p in ShellMenuPosition::ALL {
                                ui.selectable_value(&mut spec.position, p, pos_label(p));
                            }
                        });
                    ui.end_row();

                    ui.label("");
                    ui.checkbox(&mut spec.extended, lang.shell_extended_checkbox());
                    ui.end_row();

                    if !is_edit {
                        ui.label("");
                        ui.checkbox(&mut spec.overwrite, lang.shell_overwrite_checkbox());
                        ui.end_row();
                    }
                });

            ui.add_space(6.0);
            ui.weak(lang.shell_preview_label());
            let preview = spec.preview_full_path().unwrap_or_else(|| "-".to_string());
            ui.monospace(&preview);

            if let Some(ref err) = self.shell_editor_error {
                ui.add_space(4.0);
                ui.colored_label(ui.visuals().error_fg_color, err);
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(lang.btn_save()).clicked() {
                    do_save = true;
                }
                if ui.button(lang.btn_cancel()).clicked() {
                    do_cancel = true;
                }
            });
        });

        if do_save {
            match action::save_shell_command(&spec) {
                Ok(saved_path) => {
                    self.shell_editor = None;
                    self.shell_editor_error = None;
                    self.message = Some(StatusMessage::Info(lang.fmt_shell_saved(&saved_path)));
                    self.start_scan();
                }
                Err(code) => {
                    self.shell_editor = Some(spec);
                    self.shell_editor_error = Some(lang.translate_shell_err(&code));
                }
            }
        } else if do_cancel || resp.should_close() {
            self.shell_editor = None;
            self.shell_editor_error = None;
        } else {
            self.shell_editor = Some(spec);
        }
    }

    fn handle_ready_signal(&mut self, ctx: &egui::Context) {
        let Some(ref out_path) = self.ready_file.clone() else {
            return;
        };
        if let Some(rem) = self.ready_countdown {
            if rem > 0 {
                self.ready_countdown = Some(rem - 1);
                ctx.request_repaint();
            } else {
                let _ = std::fs::write(out_path, "ready");
                self.ready_file = None;
                self.ready_countdown = None;
            }
        }
    }
}

impl eframe::App for ContextManagerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) && !self.scanning {
            self.start_scan();
        }

        self.poll_scan();
        self.poll_update();

        self.update_banner(ui);
        self.top_toolbar(ui);
        self.bottom_bar(ui);
        self.sidebar(ui);
        self.detail_panel(ui);

        let lang = self.lang();
        let filtered = self.filtered_indices();
        let groups = build_groups(&self.entries, &filtered, lang);
        let group_totals = count_group_totals(&self.entries);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(4.0);
            self.central_list(ui, &filtered, &groups, &group_totals);
        });

        let ctx = ui.ctx().clone();
        self.delete_confirm_modal(&ctx);
        self.backup_modal(&ctx);
        self.restart_explorer_modal(&ctx);
        self.shell_command_modal(&ctx);
        self.handle_ready_signal(&ctx);
    }
}

fn append_token(buf: &mut String, token: &str) {
    if !buf.is_empty() && !buf.ends_with(' ') {
        buf.push(' ');
    }
    buf.push_str(token);
}

fn prepend_prefix(buf: &mut String, prefix: &str) {
    let trimmed = buf.trim_start();
    if !trimmed.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) {
        *buf = format!("{prefix}{trimmed}");
    }
}

fn truncate_middle(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars || max_chars < 8 {
        return s.to_string();
    }
    let head = (max_chars - 3) / 2;
    let tail = max_chars - 3 - head;
    let mut out = String::with_capacity(s.len());
    for &c in &chars[..head] {
        out.push(c);
    }
    out.push_str("...");
    for &c in &chars[chars.len() - tail..] {
        out.push(c);
    }
    out
}
