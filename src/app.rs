//! 앱 루트 상태 및 화면 조합 (`NotepadApp` — PLAN.md §4.2).
//!
//! - P1-01: 메뉴바·상태 표시줄·본문 영역 골격
//! - P1-02: OS별 시스템 폰트 Fallback 체인(`fonts.rs`) 적용, 로드 실패 시 상태 표시줄/로그 경고
//! - P1-03: 전역 `Command` 열거형, OS별 `Keymap`, 위젯 선처리 `KeyDispatcher`, 상단 7개 메뉴바 연결
//! - P1-09: 본문을 실제 에디터(`ui::editor`)와 문서 모델(`Document`)로 교체, 편집 커맨드·상태 표시줄 커서 연결
//!   (탭은 P1-10 전까지 문서 1개)

use eframe::egui::{self, FontId, ViewportCommand};

use crate::commands::{Command, KeyDispatcher, Keymap, ThemeMode, ViewMode};
use crate::core::document::Document;
use crate::fonts;
use crate::ui::{
    editor::{self, EditorOptions, EditorState},
    menu_bar::{self, MenuBarState, MenuTabEntry},
    status_bar::{self, StatusBarInfo},
    theme::Palette,
};

/// My-Notepad 메인 애플리케이션 상태.
pub struct NotepadApp {
    frames: u64,
    font_warning: Option<String>,

    // P1-09 문서·에디터 (P1-10에서 탭 목록으로 바뀜)
    doc: Document,
    editor: EditorState,

    // P1-03 커맨드·단축키 및 메뉴 상태
    keymap: Keymap,
    key_dispatcher: KeyDispatcher,
    sidebar_open: bool,
    word_wrap: bool,
    zoom: f32,
    view_mode: ViewMode,
    scroll_sync: bool,
    theme: ThemeMode,
    last_command: Option<String>,
}

impl NotepadApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (defs, report) = fonts::explicit_chain();
        cc.egui_ctx.set_fonts(defs);

        log::info!(
            "폰트 체인 로드 완료 ({:.2} ms, 성공 {}개, 실패 {}개)",
            report.total.as_secs_f64() * 1000.0,
            report.loaded.len(),
            report.failures.len()
        );
        for failure in &report.failures {
            log::warn!("시스템 폰트 로드 실패 (기본 폰트로 대체): {failure}");
        }

        let font_warning = report.status_warning();
        // 첫 실행 화면에서 바로 입력할 수 있도록 빈 새 문서에 포커스를 준다 (UX-2).
        let mut editor = EditorState::new(1usize);
        editor.request_focus();
        Self {
            frames: 0,
            font_warning,
            doc: Document::new_untitled(1),
            editor,
            keymap: Keymap::current(),
            key_dispatcher: KeyDispatcher::new(),
            sidebar_open: true,
            word_wrap: true,
            zoom: 1.0,
            view_mode: ViewMode::Split,
            scroll_sync: true,
            theme: ThemeMode::System,
            last_command: None,
        }
    }

    fn execute_command(&mut self, ctx: &egui::Context, command: Command, source: &'static str) {
        let shortcut_hint = self
            .keymap
            .format_shortcut_for(command, ctx)
            .map(|s| format!(" [{s}]"))
            .unwrap_or_default();
        let summary = format!("{} ({source}{shortcut_hint})", command.label());
        log::info!("커맨드 실행: {summary}");

        self.last_command = Some(summary);

        if self.editor.handle_command(ctx, &mut self.doc, command) {
            return;
        }

        match command {
            Command::Exit => {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            Command::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
            }
            Command::ToggleWordWrap => {
                self.word_wrap = !self.word_wrap;
            }
            Command::ZoomIn => {
                self.zoom = (self.zoom + 0.1).min(3.0);
            }
            Command::ZoomOut => {
                self.zoom = (self.zoom - 0.1).max(0.5);
            }
            Command::ZoomReset => {
                self.zoom = 1.0;
            }
            Command::SetViewMode(mode) => {
                self.view_mode = mode;
            }
            Command::ToggleMdPreview => {
                self.view_mode = if self.view_mode == ViewMode::Preview {
                    ViewMode::Source
                } else {
                    ViewMode::Preview
                };
            }
            Command::ToggleMdSplit => {
                self.view_mode = if self.view_mode == ViewMode::Split {
                    ViewMode::Source
                } else {
                    ViewMode::Split
                };
            }
            Command::ToggleScrollSync => {
                self.scroll_sync = !self.scroll_sync;
            }
            Command::SetTheme(theme) => {
                self.theme = theme;
                ctx.set_theme(theme.to_egui());
            }
            // 다시 읽기(ReopenWithEncoding)는 파일 입출력 연결(P1-10) 이후 P1-12에서 구현
            Command::ConvertToEncoding(enc) => {
                self.doc.convert_to_encoding(enc);
            }
            Command::SetLineEnding(le) => {
                self.doc.set_line_ending(le);
            }
            _ => {}
        }
    }
}

impl eframe::App for NotepadApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        let ctx = ui.ctx().clone();

        // 1. 위젯보다 먼저 전역 단축키를 소비·디스패치한다 (PLAN.md §3.5).
        if let Some(cmd) = self.key_dispatcher.dispatch(&ctx, &self.keymap) {
            self.execute_command(&ctx, cmd, "단축키");
        }

        let palette = Palette::from_visuals(ui.visuals());

        // 2. 상단 메뉴바 렌더링 (`파일 · 편집 · 검색 · 보기 · 인코딩 · 창 · 도움말`)
        let title = self.doc.title();
        let path_hint = self.doc.path().map_or_else(
            || "저장되지 않은 새 문서".to_owned(),
            |p| p.display().to_string(),
        );
        let tabs = [MenuTabEntry {
            title: &title,
            path_hint: &path_hint,
            modified: self.doc.is_modified(),
        }];
        let recent_files = ["회의록.txt", "README.md", "Cargo.toml"];
        let menu_state = MenuBarState {
            sidebar_open: self.sidebar_open,
            word_wrap: self.word_wrap,
            is_markdown: self.doc.is_markdown(),
            view_mode: self.view_mode,
            scroll_sync: self.scroll_sync,
            theme: self.theme,
            encoding: self.doc.encoding(),
            line_ending: self.doc.line_ending(),
            mixed_line_endings: self.doc.has_mixed_line_endings(),
            recent_files: &recent_files,
            tabs: &tabs,
            active_tab: 0,
            modified_marker_color: palette.modified_marker,
        };

        let mut menu_cmd = None;
        egui::Panel::top("menu_bar").show(ui, |ui| {
            menu_cmd = menu_bar::show(ui, &self.keymap, &menu_state);
        });
        if let Some(cmd) = menu_cmd {
            self.execute_command(&ctx, cmd, "메뉴");
        }

        // 3. 하단 상태 표시줄
        let chord_hint = self.key_dispatcher.is_chord_k_pending().then(|| {
            format!(
                "{} 입력됨 — 다음 키(V) 대기 중…",
                ctx.format_shortcut(&self.keymap.chord_k())
            )
        });
        let (line, column, selected_chars) = editor::cursor_status(&self.doc);
        let status_info = StatusBarInfo {
            line,
            column,
            selected_chars,
            line_count: self.doc.line_count(),
            zoom: self.zoom,
            encoding: self.doc.encoding(),
            line_ending: self.doc.line_ending(),
            font_warning: self.font_warning.as_deref(),
            chord_hint: chord_hint.as_deref(),
            last_command: self.last_command.as_deref(),
        };
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            status_bar::show(ui, &palette, &status_info);
        });

        // 4. 본문 영역: 에디터
        let font_size = ui.style().text_styles[&egui::TextStyle::Monospace].size * self.zoom;
        let options = EditorOptions {
            word_wrap: self.word_wrap,
            font: FontId::monospace(font_size),
        };
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0))
            .show(ui, |ui| {
                self.editor.show(ui, &mut self.doc, &options);
            });
    }
}
