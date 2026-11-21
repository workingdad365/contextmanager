//! ContextManager 메인 GUI (egui / eframe)

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::action::{self, ActionReport};
use crate::backup::{self, BackupRecord};
use crate::model::{
    Category, ContextEntry, EntryGroup, EntryKind, FilterOptions, build_groups, count_group_totals,
    find_related_unchecked_indices, matches_filter,
};
use crate::scanner::{self, ScanResult};
use crate::system;

pub const APP_NAME: &str = "ContextManager";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const ICON_PNG: &[u8] = include_bytes!("../assets/contextmanager.png");

pub fn run() {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("{APP_NAME} v{APP_VERSION}"))
        .with_inner_size([1160.0, 720.0])
        .with_min_inner_size([860.0, 520.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };

    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(ContextManagerApp::new(cc.egui_ctx.clone())))
        }),
    );
    if let Err(e) = result {
        eprintln!("window error: {e}");
    }
}

/// 기본 폰트에 한글이 없으므로 맑은 고딕을 우선 폰트로 추가
fn setup_fonts(ctx: &egui::Context) {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let path = std::path::Path::new(&windir).join("Fonts").join("malgun.ttf");
    let Ok(bytes) = std::fs::read(path) else { return };

    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("malgun".into(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().insert(0, "malgun".into());
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

struct ContextManagerApp {
    ctx: egui::Context,
    entries: Vec<ContextEntry>,
    scanning: bool,
    scan_rx: Option<Receiver<ScanResult>>,
    last_scan_ms: Option<u128>,

    filter: FilterOptions,
    view_mode: ViewMode,
    /// 체크박스로 선택된 개별 항목 id 집합
    checked_ids: HashSet<String>,
    /// 우측 상세 패널에 표시할 대상
    focused: Option<FocusedTarget>,

    is_elevated: bool,
    os_label: String,
    message: Option<StatusMessage>,

    show_delete_modal: bool,
    include_related_on_delete: bool,
    show_backup_modal: bool,
    show_restart_explorer_modal: bool,
    backups: Vec<BackupRecord>,
    selected_backup_id: Option<String>,
    first_frame: bool,
}

impl ContextManagerApp {
    fn new(ctx: egui::Context) -> Self {
        let build = system::windows_build_number();
        let os_label = if build >= 22000 {
            format!("Windows 11 (빌드 {build})")
        } else if build > 0 {
            format!("Windows 10 (빌드 {build})")
        } else {
            "Windows".to_string()
        };

        let mut app = Self {
            ctx,
            entries: Vec::new(),
            scanning: false,
            scan_rx: None,
            last_scan_ms: None,
            filter: FilterOptions::default(),
            view_mode: ViewMode::Grouped,
            checked_ids: HashSet::new(),
            focused: None,
            is_elevated: system::is_elevated(),
            os_label,
            message: None,
            show_delete_modal: false,
            include_related_on_delete: false,
            show_backup_modal: false,
            show_restart_explorer_modal: false,
            backups: Vec::new(),
            selected_backup_id: None,
            first_frame: true,
        };
        app.start_scan();
        app
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

            // 존재하지 않게 된 선택 ID 정리
            let valid_ids: HashSet<&str> = self.entries.iter().map(|e| e.id.as_str()).collect();
            self.checked_ids.retain(|id| valid_ids.contains(id.as_str()));
        }
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
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches_filter(e, &self.filter))
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
        let targets = self.checked_entries();
        if targets.is_empty() {
            self.message = Some(StatusMessage::Warn("먼저 항목을 선택해 주세요.".into()));
            return;
        }
        let needs_explorer_hint = targets
            .iter()
            .any(|e| matches!(e.kind, EntryKind::ShellEx | EntryKind::Win11Packaged));
        let admin_needed = targets.iter().filter(|e| e.requires_admin_to_toggle()).count();
        let related_remaining = find_related_unchecked_indices(&self.entries, &self.checked_ids).len();
        let report = action::set_entries_disabled(&targets, disable);
        let verb = if disable { "비활성화(숨김)" } else { "다시 활성화" };
        self.report_to_message(verb, &report, None);
        if report.errors.is_empty() {
            if let Some(StatusMessage::Info(msg)) = &mut self.message {
                if related_remaining > 0 {
                    msg.push_str(&format!(" (참고: 같은 프로그램의 다른 위치 {related_remaining}곳은 미선택 상태)"));
                }
                if needs_explorer_hint {
                    msg.push_str(" (COM 셸 확장/Win11 메뉴는 필요 시 '탐색기 다시 시작'으로 즉시 반영)");
                }
            }
        }
        if admin_needed > 0 && !self.is_elevated && !report.errors.is_empty() {
            self.message = Some(StatusMessage::Error(format!(
                "성공 {}건, 실패 {}건 - HKLM 항목 {}건은 관리자 권한으로 재실행한 뒤 변경해 주세요.",
                report.succeeded,
                report.errors.len(),
                admin_needed
            )));
        }
        self.start_scan();
    }

    fn apply_delete(&mut self, include_related: bool) {
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
                self.report_to_message("제거", &report, Some(&rec.id));
                self.start_scan();
            }
            Err(e) => {
                self.message = Some(StatusMessage::Error(format!("백업 또는 삭제 실패: {e}")));
            }
        }
    }

    fn report_to_message(&mut self, verb: &str, report: &ActionReport, backup_id: Option<&str>) {
        if report.errors.is_empty() {
            let mut msg = format!("{}개 항목을 {} 처리했습니다.", report.succeeded, verb);
            if let Some(bid) = backup_id {
                msg.push_str(&format!(" (자동 백업 완료: {bid})"));
            }
            if !report.notes.is_empty() {
                msg.push_str(&format!(" / 안내: {}", report.notes[0]));
            }
            self.message = Some(StatusMessage::Info(msg));
        } else {
            let first_err = &report.errors[0];
            let extra = if report.errors.len() > 1 {
                format!(" 외 {}건 실패", report.errors.len() - 1)
            } else {
                String::new()
            };
            self.message = Some(StatusMessage::Error(format!(
                "성공 {}건, 실패 {}건 - {}{}",
                report.succeeded,
                report.errors.len(),
                first_err,
                extra
            )));
        }
    }

    fn top_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("검색:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter.search)
                        .hint_text("이름, 실행 경로, 확장자, CLSID...")
                        .desired_width(240.0),
                );
                if !self.filter.search.is_empty() && ui.button("지우기").clicked() {
                    self.filter.search.clear();
                }

                ui.separator();
                ui.checkbox(&mut self.filter.only_orphaned, "잔여 항목만 (파일 없음)")
                    .on_hover_text("실행 파일이나 DLL이 디스크에 없는 찌꺼기 항목만 표시합니다.");
                ui.checkbox(&mut self.filter.only_disabled, "비활성화됨만");
                ui.checkbox(&mut self.filter.hide_system, "Microsoft 기본 항목 숨기기")
                    .on_hover_text("Windows 기본 내장 항목을 목록에서 숨겨 실수로 삭제하지 않도록 보호합니다.");

                ui.separator();
                ui.selectable_value(&mut self.view_mode, ViewMode::Grouped, "앱별 묶어 보기")
                    .on_hover_text("같은 프로그램의 여러 레지스트리 위치를 하나로 묶어 일괄 관리합니다.");
                ui.selectable_value(&mut self.view_mode, ViewMode::Flat, "개별 항목 보기");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("백업 / 복원").clicked() {
                        self.refresh_backups();
                        self.show_backup_modal = true;
                    }
                    let refresh_btn = ui.add_enabled(!self.scanning, egui::Button::new("새로고침"));
                    if refresh_btn.clicked() {
                        self.start_scan();
                    }
                    if self.scanning {
                        ui.spinner();
                        ui.weak("스캔 중...");
                    }
                });
            });
            ui.add_space(4.0);
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("categories")
            .resizable(true)
            .default_size(215.0)
            .min_size(170.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.strong("우클릭 위치별 분류");
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for &cat in Category::SIDEBAR {
                        let (total, orphaned) = self.category_counts(cat);
                        let selected = self.filter.category == cat;
                        let label = if orphaned > 0 {
                            format!("{}  ({total})  [잔여 {orphaned}]", cat.label())
                        } else {
                            format!("{}  ({total})", cat.label())
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
                    ui.weak(&self.os_label);
                    if let Some(ms) = self.last_scan_ms {
                        ui.weak(format!("전체 {}건 스캔 ({ms}ms)", self.entries.len()));
                    }
                });
            });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let selected_count = self.checked_ids.len();
        let mut do_disable = false;
        let mut do_enable = false;
        let mut do_delete = false;
        let mut do_elevate = false;
        let mut do_restart_explorer = false;

        egui::Panel::bottom("actions_bar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!("선택됨: {selected_count}개"));
                ui.separator();
                if ui
                    .add_enabled(selected_count > 0 && !self.scanning, egui::Button::new("비활성화 (숨기기)"))
                    .on_hover_text("항목을 삭제하지 않고 탐색기 메뉴에서만 숨깁니다. 언제든 다시 켤 수 있습니다.")
                    .clicked()
                {
                    do_disable = true;
                }
                if ui
                    .add_enabled(selected_count > 0 && !self.scanning, egui::Button::new("다시 켜기"))
                    .clicked()
                {
                    do_enable = true;
                }
                let del_text = egui::RichText::new("선택 항목 제거")
                    .color(if selected_count > 0 {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().weak_text_color()
                    });
                if ui
                    .add_enabled(selected_count > 0 && !self.scanning, egui::Button::new(del_text))
                    .on_hover_text("제거 전 자동으로 백업을 저장한 뒤 선택한 항목을 삭제합니다.")
                    .clicked()
                {
                    do_delete = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("탐색기 다시 시작")
                        .on_hover_text("COM 셸 확장이나 Windows 11 신형 메뉴 변경 사항을 즉시 반영할 때 사용합니다.")
                        .clicked()
                    {
                        do_restart_explorer = true;
                    }
                    ui.separator();
                    if self.is_elevated {
                        ui.colored_label(egui::Color32::from_rgb(34, 197, 94), "관리자 권한");
                    } else {
                        if ui
                            .button("관리자 권한으로 재실행")
                            .on_hover_text("HKLM(시스템 전체) 항목을 수정하거나 제거하려면 관리자 권한이 필요합니다.")
                            .clicked()
                        {
                            do_elevate = true;
                        }
                        ui.weak("일반 권한");
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
        let orphan_in_filtered: Vec<usize> = filtered
            .iter()
            .copied()
            .filter(|&i| self.entries[i].is_orphaned())
            .collect();

        ui.horizontal_wrapped(|ui| {
            if ui.small_button("표시된 항목 모두 선택").clicked() {
                for &i in filtered {
                    self.checked_ids.insert(self.entries[i].id.clone());
                }
            }
            if ui
                .add_enabled(!orphan_in_filtered.is_empty(), egui::Button::new("잔여 항목 모두 선택").small())
                .clicked()
            {
                for &i in &orphan_in_filtered {
                    self.checked_ids.insert(self.entries[i].id.clone());
                }
            }
            if ui
                .add_enabled(!self.checked_ids.is_empty(), egui::Button::new("선택 해제").small())
                .clicked()
            {
                self.checked_ids.clear();
            }
            ui.separator();
            ui.weak(format!(
                "표시: {}개 항목 ({}개 그룹) / 잔여 {}개",
                filtered.len(),
                groups.len(),
                orphan_in_filtered.len()
            ));
        });
        ui.separator();

        if self.scanning && self.entries.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.add_space(6.0);
                ui.label("Windows 컨텍스트 메뉴를 스캔하고 있습니다...");
            });
            return;
        }

        if filtered.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.weak("조건에 맞는 컨텍스트 메뉴 항목이 없습니다.");
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
                                ui.colored_label(error_color, "[파일 없음 - 잔여 항목]");
                            } else if group.any_orphaned {
                                ui.colored_label(warn_color, "[일부 파일 없음]");
                            }

                            if group.all_disabled {
                                ui.weak("[비활성화됨]");
                            } else if group.any_disabled {
                                ui.weak("[일부 비활성화]");
                            }

                            if group.is_system {
                                ui.weak("[시스템]");
                            }

                            let shown_cnt = group.entry_indices.len();
                            let total_cnt = group_totals.get(&group.key).copied().unwrap_or(shown_cnt);
                            if total_cnt > shown_cnt {
                                ui.colored_label(
                                    warn_color,
                                    format!("(이 분류 {shown_cnt}곳 / 전체 {total_cnt}곳)"),
                                )
                                .on_hover_text(
                                    "다른 분류에도 같은 프로그램의 컨텍스트 메뉴 항목이 더 있습니다.\n우측 상세 패널이나 삭제 확인 창에서 전체 위치를 한꺼번에 선택할 수 있습니다.",
                                );
                            } else if shown_cnt > 1 {
                                ui.weak(format!("({shown_cnt}곳)"));
                            }
                            if checked_cnt > 0 && !all_checked {
                                ui.colored_label(warn_color, format!("[{checked_cnt}/{shown_cnt} 선택]"));
                            }
                        });

                        ui.horizontal_wrapped(|ui| {
                            ui.weak(format!(
                                "{} | {} | {}",
                                group.kind.label(),
                                group.scope_label(),
                                group.summary_targets
                            ));
                            if let Some(ref target) = group.resolved_target {
                                ui.weak("|");
                                ui.weak(truncate_middle(target, 68));
                            }
                        });
                    });
                });
            });
            ui.add_space(3.0);
        }
    }

    fn render_flat_list(&mut self, ui: &mut egui::Ui, filtered: &[usize]) {
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
                                ui.colored_label(error_color, "[파일 없음]");
                            }
                            if entry.disabled {
                                ui.weak("[비활성화됨]");
                            }
                            if entry.is_system {
                                ui.weak("[시스템]");
                            }
                        });
                        ui.weak(format!(
                            "{} | {} | {} | {}",
                            entry.kind.label(),
                            entry.scope.short_label(),
                            entry.target_label,
                            truncate_middle(&entry.location_display(), 72)
                        ));
                    });
                });
            });
            ui.add_space(2.0);
        }
    }

    fn detail_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("detail_panel")
            .resizable(true)
            .default_size(360.0)
            .min_size(270.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.strong("상세 정보");
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
                    ui.weak("목록에서 항목이나 그룹을 클릭하면\n실행 명령어, 파일 존재 여부, 포함된 모든 레지스트리 경로를 볼 수 있습니다.");
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
                                ui.weak("유형");
                                ui.label(first.kind.label());
                                ui.end_row();

                                ui.weak("범위");
                                ui.label(first.scope.label());
                                ui.end_row();

                                ui.weak("파일 상태");
                                match first.target_exists {
                                    Some(true) => {
                                        ui.colored_label(egui::Color32::from_rgb(34, 197, 94), "실제 파일 존재함");
                                    }
                                    Some(false) => {
                                        ui.colored_label(ui.visuals().error_fg_color, "파일 없음 (제거된 앱의 잔여 항목)");
                                    }
                                    None => {
                                        ui.weak("확인 대상 파일 없음");
                                    }
                                }
                                ui.end_row();

                                if let Some(ref clsid) = first.clsid {
                                    ui.weak("CLSID");
                                    ui.monospace(clsid);
                                    ui.end_row();
                                }
                                if let Some(ref pkg) = first.package_name {
                                    ui.weak("패키지");
                                    ui.label(pkg);
                                    ui.end_row();
                                }
                            });

                        if let Some(ref target) = first.resolved_target {
                            ui.add_space(6.0);
                            ui.weak("대상 실행 파일 / DLL:");
                            ui.monospace(target);
                            ui.horizontal(|ui| {
                                if ui.small_button("경로 복사").clicked() {
                                    ui.ctx().copy_text(target.clone());
                                    self.message = Some(StatusMessage::Info("경로를 클립보드에 복사했습니다.".into()));
                                }
                                if ui.small_button("파일 위치 열기").clicked() {
                                    if let Err(e) = system::open_in_explorer(target) {
                                        self.message = Some(StatusMessage::Error(e));
                                    }
                                }
                            });
                        }

                        if let Some(ref cmd) = first.command {
                            ui.add_space(6.0);
                            ui.weak("실행 명령어:");
                            ui.monospace(cmd);
                        }

                        if let Some(ref icon) = first.icon {
                            ui.add_space(4.0);
                            ui.weak("아이콘:");
                            ui.monospace(icon);
                        }

                        ui.add_space(10.0);
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(format!("포함된 등록 위치 (전체 {}곳)", target_indices.len()));
                            if ui
                                .small_button(format!("전체 {}곳 모두 체크", target_indices.len()))
                                .clicked()
                            {
                                for &i in &target_indices {
                                    self.checked_ids.insert(self.entries[i].id.clone());
                                }
                            }
                            if ui.small_button("모두 해제").clicked() {
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
                                        ui.strong(&e.target_label);
                                        ui.weak(format!("[{}]", e.scope.short_label()));
                                        if outside_current_cat {
                                            ui.colored_label(
                                                ui.visuals().warn_fg_color,
                                                format!("[다른 분류: {}]", e.category.label()),
                                            );
                                        }
                                        if e.disabled {
                                            ui.weak("(비활성화)");
                                        }
                                        if e.is_orphaned() {
                                            ui.colored_label(ui.visuals().error_fg_color, "(파일 없음)");
                                        }
                                    });
                                    ui.monospace(&loc_display);
                                    ui.horizontal(|ui| {
                                        if ui.small_button("경로 복사").clicked() {
                                            ui.ctx().copy_text(loc_display.clone());
                                            self.message = Some(StatusMessage::Info("등록 경로를 복사했습니다.".into()));
                                        }
                                        if let Some(ref reg_path) = full_reg {
                                            if ui.small_button("레지스트리 편집기에서 열기").clicked() {
                                                if let Err(err) = system::open_regedit_at(reg_path) {
                                                    self.message = Some(StatusMessage::Error(err));
                                                }
                                            }
                                        } else if let Some(ref fp) = e.disabled_file_path.as_ref().or(e.file_path.as_ref()) {
                                            if ui.small_button("폴더에서 열기").clicked() {
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
        let related_indices = find_related_unchecked_indices(&self.entries, &self.checked_ids);
        let related_count = related_indices.len();
        if related_count == 0 {
            self.include_related_on_delete = false;
        }

        let related_summary = if related_count > 0 {
            let mut cat_set: BTreeSet<&str> = BTreeSet::new();
            for &idx in &related_indices {
                cat_set.insert(self.entries[idx].category.label());
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
            .map(|e| format!("- {} [{}] ({})", e.display_name, e.target_label, e.scope.short_label()))
            .collect();

        let mut confirm = false;
        let mut cancel = false;
        let mut elevate = false;
        let mut include_related = self.include_related_on_delete;

        let resp = egui::Modal::new(egui::Id::unique("delete_confirm_modal")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("선택 항목 제거 확인");
            ui.add_space(6.0);
            ui.label(format!("총 {total}개 항목을 레지스트리/파일에서 제거합니다."));
            ui.weak("제거 직전에 자동으로 백업(.json 및 .reg)이 저장되므로, '백업 / 복원'에서 언제든 되돌릴 수 있습니다.");

            if related_count > 0 {
                ui.add_space(8.0);
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().faint_bg_color)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!("같은 프로그램의 다른 위치에 {related_count}개 항목이 더 남아 있습니다."),
                        );
                        ui.weak(format!("포함된 다른 분류: {related_summary}"));
                        ui.add_space(4.0);
                        ui.checkbox(
                            &mut include_related,
                            format!(
                                "다른 위치 {related_count}곳도 함께 포함해서 모두 제거 (총 {}곳)",
                                base_count + related_count
                            ),
                        );
                    });
            }

            if win11_count > 0 {
                ui.add_space(4.0);
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "Windows 11 패키지 메뉴 {win11_count}건은 시스템 보호 영역이므로 파일 삭제 대신 차단(숨김) 목록에 등록됩니다."
                    ),
                );
            }

            if admin_needed > 0 && !self.is_elevated {
                ui.add_space(4.0);
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!(
                        "HKLM(시스템 전체) 항목 {admin_needed}건은 관리자 권한이 없으면 제거에 실패합니다."
                    ),
                );
            }

            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for line in &preview_names {
                    ui.label(line);
                }
                if total > preview_names.len() {
                    ui.weak(format!("... 외 {}건", total - preview_names.len()));
                }
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let btn_label = format!("백업 후 {total}개 제거");
                let btn_text = egui::RichText::new(btn_label).color(ui.visuals().error_fg_color);
                if ui.button(btn_text).clicked() {
                    confirm = true;
                }
                if admin_needed > 0 && !self.is_elevated && ui.button("관리자 권한으로 재실행").clicked() {
                    elevate = true;
                }
                if ui.button("취소").clicked() {
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
        let mut close = false;
        let mut restore_record: Option<BackupRecord> = None;
        let mut delete_record: Option<BackupRecord> = None;
        let mut open_folder = false;

        let resp = egui::Modal::new(egui::Id::unique("backup_modal")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("백업 및 복원 관리");
            ui.add_space(4.0);
            ui.weak(format!("저장 위치: {}", backup::default_backup_dir().display()));
            ui.add_space(6.0);

            if self.backups.is_empty() {
                ui.add_space(16.0);
                ui.weak("저장된 백업 기록이 없습니다. 항목을 제거하면 자동으로 백업이 생성됩니다.");
                ui.add_space(16.0);
            } else {
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    for rec in &self.backups {
                        let selected = self.selected_backup_id.as_deref() == Some(rec.id.as_str());
                        let label = format!("{}  |  {} ({}건)", rec.created_at, rec.summary, rec.items.len());
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
                        ui.weak(format!("선택한 백업 포함 항목 ({}건):", rec.items.len()));
                        egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
                            for item in &rec.items {
                                let desc = match item {
                                    backup::BackupItem::Registry { root, subkey, display_name, .. } => {
                                        format!("- {display_name} ({})", root.format_short_path(subkey))
                                    }
                                    backup::BackupItem::File { original_path, display_name, .. } => {
                                        format!("- {display_name} ({original_path})")
                                    }
                                    backup::BackupItem::Win11Block { clsid, display_name } => {
                                        format!("- {display_name} (Win11 차단 해제: {clsid})")
                                    }
                                };
                                ui.label(desc);
                            }
                        });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("선택한 백업 복원").clicked() {
                                restore_record = Some(rec.clone());
                            }
                            if ui.button("백업 기록 삭제").clicked() {
                                delete_record = Some(rec.clone());
                            }
                        });
                    }
                }
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("백업 폴더 열기").clicked() {
                    open_folder = true;
                }
                if ui.button("닫기").clicked() {
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
                self.message = Some(StatusMessage::Info(format!(
                    "백업({})의 {}개 항목을 복원했습니다.",
                    rec.created_at, rep.succeeded
                )));
            } else {
                self.message = Some(StatusMessage::Error(format!(
                    "복원 성공 {}건, 실패 {}건 - {}",
                    rep.succeeded,
                    rep.errors.len(),
                    rep.errors[0]
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
        let mut confirm = false;
        let mut cancel = false;
        let resp = egui::Modal::new(egui::Id::unique("restart_explorer_modal")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading("Windows 탐색기 다시 시작");
            ui.add_space(6.0);
            ui.label("탐색기(explorer.exe)를 종료한 뒤 다시 실행합니다.");
            ui.weak("작업 표시줄과 열려 있는 폴더 창이 잠시 닫혔다가 다시 열립니다.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("다시 시작").clicked() {
                    confirm = true;
                }
                if ui.button("취소").clicked() {
                    cancel = true;
                }
            });
        });

        if confirm {
            self.show_restart_explorer_modal = false;
            match system::restart_explorer() {
                Ok(()) => {
                    self.message = Some(StatusMessage::Info("Windows 탐색기를 다시 시작했습니다.".into()));
                }
                Err(e) => {
                    self.message = Some(StatusMessage::Error(e));
                }
            }
        } else if cancel || resp.should_close() {
            self.show_restart_explorer_modal = false;
        }
    }
}

impl eframe::App for ContextManagerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // F5 새로고침
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) && !self.scanning {
            self.start_scan();
        }

        self.poll_scan();

        self.top_toolbar(ui);
        self.bottom_bar(ui);
        self.sidebar(ui);
        self.detail_panel(ui);

        let filtered = self.filtered_indices();
        let groups = build_groups(&self.entries, &filtered);
        let group_totals = count_group_totals(&self.entries);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(4.0);
            self.central_list(ui, &filtered, &groups, &group_totals);
        });

        let ctx = ui.ctx().clone();
        self.delete_confirm_modal(&ctx);
        self.backup_modal(&ctx);
        self.restart_explorer_modal(&ctx);
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
