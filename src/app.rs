//! 앱 루트 상태 및 화면 조합 (`NotepadApp` — PLAN.md §4.2).
//!
//! - P1-01: 메뉴바·상태 표시줄·본문 영역 골격
//! - P1-02: OS별 시스템 폰트 Fallback 체인(`fonts.rs`) 적용, 로드 실패 시 상태 표시줄/로그 경고
//! - P1-03: 전역 `Command` 열거형, OS별 `Keymap`, 위젯 선처리 `KeyDispatcher`, 상단 7개 메뉴바 연결

use std::collections::VecDeque;

use eframe::egui::{self, FontFamily, RichText, ViewportCommand};

use crate::commands::{
    Command, EncodingChoice, KeyDispatcher, Keymap, LineEndingChoice, ThemeMode, ViewMode,
};
use crate::fonts::{self, BOLD_FAMILY, FontReport, ICON_FAMILY};
use crate::ui::{
    menu_bar::{self, MenuBarState, MenuTabEntry},
    status_bar::{self, StatusBarInfo},
    theme::Palette,
};

const COMMAND_LOG_LIMIT: usize = 30;

const SAMPLE_TEXT: &str = "\
English: The quick brown fox jumps over the lazy dog. 0123456789
한글: 다람쥐 헌 쳇바퀴에 타고파. 키스의 고유조건은 입술끼리 만나야 하고 특별한 기술은 필요치 않다.
기호: ()[]{}<>!@#$%^&*-_=+|\\/:;\"'`~ ← → ★ ※ ①②③

[정렬 확인: 영문·숫자 줄 (고정폭 세로 정렬)]
ID    NAME        QTY    PRICE
0001  apple         12    1,200
0002  banana       345   34,500
0003  cherry      6789  678,900

[정렬 확인: 한글 포함 줄]
ID    이름        수량   가격
0004  사과          12    1,200
0005  바나나       345   34,500

Tab 키 입력 확인 →\t← 이 사이에 탭 문자가 있습니다.
";

const PROPORTIONAL_SAMPLE: &str = "메뉴·대화상자·Markdown 미리보기 본문에 쓰이는 비례폭(Proportional) 글꼴입니다. \
한글과 English가 섞인 문장도 자연스럽게 이어져야 하며, 숫자 1234567890과 기호 (괄호), \"따옴표\"도 깨짐 없이 표시됩니다.";

/// My-Notepad 메인 애플리케이션 상태.
pub struct NotepadApp {
    frames: u64,
    font_report: FontReport,
    font_warning: Option<String>,
    sample_text: String,

    // P1-03 커맨드·단축키 및 메뉴 상태
    keymap: Keymap,
    key_dispatcher: KeyDispatcher,
    sidebar_open: bool,
    word_wrap: bool,
    zoom: f32,
    view_mode: ViewMode,
    scroll_sync: bool,
    theme: ThemeMode,
    encoding: EncodingChoice,
    line_ending: LineEndingChoice,
    active_tab: usize,
    last_command: Option<String>,
    command_log: VecDeque<String>,
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
        Self {
            frames: 0,
            font_report: report,
            font_warning,
            sample_text: SAMPLE_TEXT.to_owned(),
            keymap: Keymap::current(),
            key_dispatcher: KeyDispatcher::new(),
            sidebar_open: true,
            word_wrap: true,
            zoom: 1.0,
            view_mode: ViewMode::Split,
            scroll_sync: true,
            theme: ThemeMode::System,
            encoding: EncodingChoice::Utf8,
            line_ending: LineEndingChoice::default_for_os(),
            active_tab: 0,
            last_command: None,
            command_log: VecDeque::new(),
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

        self.last_command = Some(summary.clone());
        self.command_log
            .push_front(format!("#{:04}  {summary}", self.frames));
        self.command_log.truncate(COMMAND_LOG_LIMIT);

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
            Command::ReopenWithEncoding(enc) | Command::ConvertToEncoding(enc) => {
                self.encoding = enc;
            }
            Command::SetLineEnding(le) => {
                self.line_ending = le;
            }
            Command::GotoTab(idx) => {
                self.active_tab = idx.min(2);
            }
            Command::LastTab => {
                self.active_tab = 2;
            }
            Command::NextTab => {
                self.active_tab = (self.active_tab + 1) % 3;
            }
            Command::PrevTab => {
                self.active_tab = (self.active_tab + 2) % 3;
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
        let sample_tabs = [
            MenuTabEntry {
                title: "Untitled-1.md",
                path_hint: "저장되지 않은 새 문서",
                modified: false,
            },
            MenuTabEntry {
                title: "회의록.txt",
                path_hint: "C:\\docs\\회의록.txt",
                modified: true,
            },
            MenuTabEntry {
                title: "README.md",
                path_hint: "C:\\devel\\My-Notepad\\README.md",
                modified: false,
            },
        ];
        let recent_files = ["회의록.txt", "README.md", "Cargo.toml"];
        let menu_state = MenuBarState {
            sidebar_open: self.sidebar_open,
            word_wrap: self.word_wrap,
            is_markdown: true,
            view_mode: self.view_mode,
            scroll_sync: self.scroll_sync,
            theme: self.theme,
            encoding: self.encoding,
            line_ending: self.line_ending,
            mixed_line_endings: false,
            recent_files: &recent_files,
            tabs: &sample_tabs,
            active_tab: self.active_tab,
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
        let line_count = self.sample_text.lines().count().max(1);
        let status_info = StatusBarInfo {
            line_count,
            zoom: self.zoom,
            encoding: self.encoding,
            line_ending: self.line_ending,
            font_warning: self.font_warning.as_deref(),
            chord_hint: chord_hint.as_deref(),
            last_command: self.last_command.as_deref(),
        };
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            status_bar::show(ui, &palette, &status_info);
        });

        // 4. 본문 영역
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("P1-03 커맨드·단축키 확인 및 P1-02 폰트 샘플");
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(
                            "상단 메뉴(파일·편집·검색·보기·인코딩·창·도움말)를 열어 각 항목 옆의 OS 단축키 표기를 확인하거나, \
                             아래 에디터에 포커스를 둔 상태에서 단축키(Ctrl+S, Ctrl+Shift+S, Ctrl+K V, F3, Shift+F3 등)를 눌러 보세요.",
                        )
                        .small()
                        .weak(),
                    );
                    if !self.command_log.is_empty() && ui.small_button("기록 지우기").clicked() {
                        self.command_log.clear();
                        self.last_command = None;
                    }
                });

                if !self.command_log.is_empty() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("최근 실행된 커맨드 (최신순)").strong().small());
                        egui::ScrollArea::vertical()
                            .id_salt("p1_03_command_log")
                            .max_height(90.0)
                            .show(ui, |ui| {
                                for entry in &self.command_log {
                                    ui.label(RichText::new(entry).monospace().small());
                                }
                            });
                    });
                    ui.add_space(6.0);
                }

                ui.heading("1. 에디터 고정폭 (Monospace: 영문 고정폭 + 한글 Fallback)");
                ui.add(
                    egui::TextEdit::multiline(&mut self.sample_text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(14)
                        .lock_focus(true),
                );

                ui.add_space(10.0);
                ui.heading("2. 비례폭 (Proportional: 메뉴·대화상자·미리보기 본문)");
                ui.label(PROPORTIONAL_SAMPLE);

                ui.add_space(10.0);
                ui.heading("3. 굵게 (bold 패밀리: 보통 한글 vs 굵은 한글)");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("보통 한글 Regular 123").size(18.0));
                    ui.separator();
                    ui.label(
                        RichText::new("굵은 한글 Bold 123")
                            .size(18.0)
                            .family(FontFamily::Name(BOLD_FAMILY.into())),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new("\u{EB28} \u{ED50} \u{E24A} \u{E256}")
                            .size(18.0)
                            .family(FontFamily::Name(ICON_FAMILY.into()))
                            .color(palette.modified_marker),
                    );
                    ui.weak("(Phosphor 아이콘 폰트)");
                });

                ui.add_space(10.0);
                ui.collapsing("시스템 폰트 로드 보고서 (P1-02)", |ui| {
                    ui.label(
                        RichText::new(self.font_report.to_text())
                            .monospace()
                            .small(),
                    );
                });
            });
        });
    }
}
