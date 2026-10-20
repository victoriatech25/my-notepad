//! P0-UX1 전체 화면 UX 프로토타입.
//!
//! 가짜 데이터로 메뉴·탭·사이드바·에디터·상태 표시줄·찾기 바·Markdown 3모드·대화상자를
//! 클릭해 볼 수 있다. 파일을 실제로 읽거나 저장하지 않는다. 배치 근거는 docs/ux/ui-spec.md.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod data;
#[allow(dead_code)] // 스파이크 공용 모듈: 보고서 기능은 이 앱에서 쓰지 않음
#[path = "../../editor-spike/src/fonts.rs"]
mod fonts;
mod highlight;
mod icons;
mod markdown;
mod theme;
mod window_menu;

use std::ops::Range;

use eframe::egui::{
    self, Align, Color32, Event, FontId, Id, Key, KeyboardShortcut, Layout, Modifiers, RichText,
    Stroke, TextFormat, ThemePreference, ViewportCommand, text::LayoutJob,
};

use data::{Doc, EncodingState, TreeNode, ViewMode};
use highlight::FindState;

const ENCODINGS: &[&str] = &[
    "UTF-8",
    "UTF-8 with BOM",
    "EUC-KR (CP949)",
    "UTF-16 LE",
    "UTF-16 BE",
];
/// 분할 보기를 유지할 최소 본문 폭
const SPLIT_MIN_WIDTH: f32 = 700.0;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("My-Notepad — UX 프로토타입 (P0-UX1)")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([520.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "my-notepad-ux-prototype",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[derive(Clone, Copy)]
enum Dialog {
    Unsaved { doc: usize, then_quit: bool },
    Unmappable,
    ExternalChange,
    Binary,
    LargeFile,
    DamagedSave,
    Shortcuts,
    RegexHelp,
    About,
    WindowList,
}

struct Shortcuts {
    new_tab: KeyboardShortcut,
    open: KeyboardShortcut,
    save: KeyboardShortcut,
    save_as: KeyboardShortcut,
    close_tab: KeyboardShortcut,
    find: KeyboardShortcut,
    replace: KeyboardShortcut,
    find_next: KeyboardShortcut,
    find_prev: KeyboardShortcut,
    sidebar: KeyboardShortcut,
    md_toggle: KeyboardShortcut,
    chord_k: KeyboardShortcut,
    zoom_in: KeyboardShortcut,
    zoom_out: KeyboardShortcut,
    zoom_reset: KeyboardShortcut,
}

impl Shortcuts {
    fn for_os() -> Self {
        let cmd = Modifiers::COMMAND;
        let mac = cfg!(target_os = "macos");
        Self {
            new_tab: KeyboardShortcut::new(cmd, Key::N),
            open: KeyboardShortcut::new(cmd, Key::O),
            save: KeyboardShortcut::new(cmd, Key::S),
            save_as: KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::S),
            close_tab: KeyboardShortcut::new(cmd, Key::W),
            find: KeyboardShortcut::new(cmd, Key::F),
            replace: if mac {
                KeyboardShortcut::new(cmd | Modifiers::ALT, Key::F)
            } else {
                KeyboardShortcut::new(cmd, Key::H)
            },
            find_next: if mac {
                KeyboardShortcut::new(cmd, Key::G)
            } else {
                KeyboardShortcut::new(Modifiers::NONE, Key::F3)
            },
            find_prev: if mac {
                KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::G)
            } else {
                KeyboardShortcut::new(Modifiers::SHIFT, Key::F3)
            },
            sidebar: KeyboardShortcut::new(cmd, Key::B),
            md_toggle: KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::V),
            chord_k: KeyboardShortcut::new(cmd, Key::K),
            zoom_in: KeyboardShortcut::new(cmd, Key::Equals),
            zoom_out: KeyboardShortcut::new(cmd, Key::Minus),
            zoom_reset: KeyboardShortcut::new(cmd, Key::Num0),
        }
    }
}

struct App {
    docs: Vec<Doc>,
    active: usize,
    untitled_counter: usize,
    tree: TreeNode,
    sidebar_open: bool,
    word_wrap: bool,
    zoom: f32,
    scroll_sync: bool,
    theme: ThemePreference,
    find: FindState,
    dialog: Option<Dialog>,
    toast: Option<(String, f64)>,
    many_tabs: bool,
    // 창 메뉴 (UX-003)
    tab_shortcuts: window_menu::TabShortcuts,
    close_queue: Vec<String>,
    window_list_selected: std::collections::HashSet<String>,
    // 탭 바 넘침 처리 (UX-002)
    tabs_last_active: Option<usize>,
    tabs_scroll_to: Option<f32>,
    tabs_offset: f32,
    tabs_max_offset: f32,
    tabs_view_width: f32,
    tab_filter: String,
    open_dirs: std::collections::HashSet<&'static str>,
    chord_k_pending: bool,
    allow_close: bool,
    shortcuts: Shortcuts,
    // 현재 탭의 커서 정보 (에디터가 매 프레임 갱신)
    cursor: (usize, usize),
    selection_chars: usize,
    smart_word: Option<String>,
    /// 다음 프레임에 에디터 선택으로 지정할 문자 범위
    pending_select: Option<Range<usize>>,
    /// 소스 스크롤 비율 (0..1) — 간이 동기화용
    source_scroll_fraction: f32,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (mut defs, _report) = fonts::explicit_chain();
        icons::add_icon_font(&mut defs);
        cc.egui_ctx.set_fonts(defs);
        let mut app = Self {
            docs: data::sample_docs(),
            active: 0,
            untitled_counter: 1,
            tree: data::sample_tree(),
            sidebar_open: true,
            word_wrap: false,
            zoom: 1.0,
            scroll_sync: true,
            theme: ThemePreference::System,
            find: FindState::default(),
            dialog: None,
            toast: None,
            many_tabs: false,
            tab_shortcuts: window_menu::TabShortcuts::for_os(),
            close_queue: Vec::new(),
            window_list_selected: std::collections::HashSet::new(),
            tabs_last_active: None,
            tabs_scroll_to: None,
            tabs_offset: 0.0,
            tabs_max_offset: 0.0,
            tabs_view_width: 0.0,
            tab_filter: String::new(),
            open_dirs: data::initially_open_dirs(),
            chord_k_pending: false,
            allow_close: false,
            shortcuts: Shortcuts::for_os(),
            cursor: (1, 1),
            selection_chars: 0,
            smart_word: None,
            pending_select: None,
            source_scroll_fraction: 0.0,
        };
        // 스크린샷·검증용: --many-tabs (탭 18개), --active-last (마지막 탭 선택)
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--many-tabs") {
            app.set_many_tabs(true);
        }
        if args.iter().any(|a| a == "--active-last") {
            app.active = app.docs.len() - 1;
        }
        app
    }

    fn doc(&self) -> &Doc {
        &self.docs[self.active]
    }

    fn doc_mut(&mut self) -> &mut Doc {
        &mut self.docs[self.active]
    }

    fn toast(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        let now = ctx.input(|i| i.time);
        self.toast = Some((text.into(), now + 3.0));
    }

    fn switch_to(&mut self, index: usize) {
        if index < self.docs.len() && index != self.active {
            self.active = index;
            self.smart_word = None;
            self.find.current = 0;
        }
    }

    fn new_tab(&mut self) {
        self.untitled_counter += 1;
        self.docs.push(Doc::untitled(self.untitled_counter));
        self.active = self.docs.len() - 1;
    }

    fn request_close_tab(&mut self, index: usize) {
        if self.docs[index].modified {
            self.dialog = Some(Dialog::Unsaved {
                doc: index,
                then_quit: false,
            });
        } else {
            self.close_tab(index);
        }
    }

    fn close_tab(&mut self, index: usize) {
        self.docs.remove(index);
        if self.docs.is_empty() {
            self.new_tab();
        }
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
        self.active = self.active.min(self.docs.len() - 1);
    }

    fn save_active(&mut self, ctx: &egui::Context) {
        if self.doc().encoding_state == EncodingState::Damaged {
            self.dialog = Some(Dialog::DamagedSave);
            return;
        }
        if self.doc().path.is_none() {
            self.toast(
                ctx,
                "새 문서는 '다른 이름으로 저장' 대화상자가 열립니다 (프로토타입에서는 생략)",
            );
        } else {
            self.toast(
                ctx,
                format!(
                    "\"{}\" 저장됨 (프로토타입: 실제로 저장하지 않음)",
                    self.doc().title
                ),
            );
        }
        self.doc_mut().modified = false;
    }

    fn open_from_tree(&mut self, name: &'static str, path: &'static str) {
        if let Some(i) = self.docs.iter().position(|d| d.title == name) {
            self.switch_to(i);
        } else {
            self.docs.push(Doc::placeholder(name, path));
            self.active = self.docs.len() - 1;
        }
    }

    fn set_many_tabs(&mut self, many: bool) {
        self.many_tabs = many;
        if many {
            self.docs.extend(data::many_extra_docs());
        } else {
            self.docs.retain(|d| !d.title.starts_with("note-"));
            self.active = self.active.min(self.docs.len() - 1);
        }
    }

    fn find_step(&mut self, forward: bool) {
        let matches = self.find.matches(&self.docs[self.active].text);
        if matches.is_empty() {
            return;
        }
        let n = matches.len();
        self.find.current = if forward {
            (self.find.current + 1) % n
        } else {
            (self.find.current + n - 1) % n
        };
        self.select_match(&matches);
    }

    fn select_match(&mut self, matches: &[Range<usize>]) {
        let text = &self.docs[self.active].text;
        if let Some(m) = matches.get(self.find.current) {
            let start = highlight::byte_to_char(text, m.start);
            let end = highlight::byte_to_char(text, m.end);
            self.pending_select = Some(start..end);
        }
    }

    fn open_find(&mut self, replace: bool) {
        self.find.open = true;
        self.find.replace_open = replace;
        self.find.focus_query = true;
        if let Some(word) = self.smart_word.clone() {
            self.find.query = word;
        }
    }

    // ---------------------------------------------------------------- 단축키

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            return;
        }
        let s = &self.shortcuts;
        let (
            new_tab,
            open,
            save_as,
            save,
            close_tab,
            find,
            replace,
            find_prev,
            find_next,
            sidebar,
            md_toggle,
            chord_k,
            zoom_in,
            zoom_out,
            zoom_reset,
        ) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&s.new_tab),
                i.consume_shortcut(&s.open),
                i.consume_shortcut(&s.save_as),
                i.consume_shortcut(&s.save),
                i.consume_shortcut(&s.close_tab),
                i.consume_shortcut(&s.find),
                i.consume_shortcut(&s.replace),
                i.consume_shortcut(&s.find_prev),
                i.consume_shortcut(&s.find_next),
                i.consume_shortcut(&s.sidebar),
                i.consume_shortcut(&s.md_toggle),
                i.consume_shortcut(&s.chord_k),
                i.consume_shortcut(&s.zoom_in)
                    || i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Plus)),
                i.consume_shortcut(&s.zoom_out),
                i.consume_shortcut(&s.zoom_reset),
            )
        });

        // Ctrl+K 다음에 V → 분할 보기 토글
        if self.chord_k_pending {
            let v_pressed = ctx.input_mut(|i| {
                let pressed = i.consume_key(Modifiers::NONE, Key::V);
                if pressed {
                    i.events
                        .retain(|e| !matches!(e, Event::Text(t) if t.eq_ignore_ascii_case("v")));
                }
                pressed
            });
            let any_other_key = ctx.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, Event::Key { pressed: true, .. }))
            });
            if v_pressed {
                self.chord_k_pending = false;
                if self.doc().is_markdown {
                    let doc = self.doc_mut();
                    doc.view_mode = if doc.view_mode == ViewMode::Split {
                        ViewMode::Source
                    } else {
                        ViewMode::Split
                    };
                }
            } else if any_other_key {
                self.chord_k_pending = false;
            }
        }
        if chord_k {
            self.chord_k_pending = true;
        }

        if new_tab {
            self.new_tab();
        }
        if open {
            self.toast(
                ctx,
                "파일 열기 대화상자 (프로토타입에서는 생략 — 사이드바에서 파일을 클릭해 보세요)",
            );
        }
        if save_as {
            self.toast(ctx, "다른 이름으로 저장 대화상자 (프로토타입에서는 생략)");
        } else if save {
            self.save_active(ctx);
        }
        if close_tab {
            self.request_close_tab(self.active);
        }
        if replace {
            self.open_find(true);
        } else if find {
            self.open_find(false);
        }
        if find_prev {
            self.find_step(false);
        } else if find_next {
            self.find_step(true);
        }
        if sidebar {
            self.sidebar_open = !self.sidebar_open;
        }
        if md_toggle && self.doc().is_markdown {
            let doc = self.doc_mut();
            doc.view_mode = if doc.view_mode == ViewMode::Preview {
                ViewMode::Source
            } else {
                ViewMode::Preview
            };
        }
        if zoom_in {
            self.zoom = (self.zoom + 0.1).min(3.0);
        }
        if zoom_out {
            self.zoom = (self.zoom - 0.1).max(0.5);
        }
        if zoom_reset {
            self.zoom = 1.0;
        }
        if self.find.open && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.find.open = false;
        }
    }

    // ---------------------------------------------------------------- 메뉴

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let fmt = |s: &KeyboardShortcut| ctx.format_shortcut(s);
        let s = Shortcuts::for_os();

        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("파일", |ui| {
                if ui
                    .add(egui::Button::new("새 탭").shortcut_text(fmt(&s.new_tab)))
                    .clicked()
                {
                    self.new_tab();
                }
                if ui
                    .add(egui::Button::new("열기…").shortcut_text(fmt(&s.open)))
                    .clicked()
                {
                    self.toast(&ctx, "파일 열기 대화상자 (프로토타입에서는 생략)");
                }
                if ui.button("폴더 열기…").clicked() {
                    self.sidebar_open = true;
                    self.toast(&ctx, "폴더 선택 대화상자 (프로토타입에서는 생략)");
                }
                ui.menu_button("최근 파일", |ui| {
                    for name in ["회의록.txt", "README.md", "server.log"] {
                        if ui.button(name).clicked()
                            && let Some(i) = self.docs.iter().position(|d| d.title == name)
                        {
                            self.switch_to(i);
                        }
                    }
                    ui.separator();
                    ui.button("목록 지우기").clicked();
                });
                ui.separator();
                if ui
                    .add(egui::Button::new("저장").shortcut_text(fmt(&s.save)))
                    .clicked()
                {
                    self.save_active(&ctx);
                }
                if ui
                    .add(egui::Button::new("다른 이름으로 저장…").shortcut_text(fmt(&s.save_as)))
                    .clicked()
                {
                    self.toast(&ctx, "다른 이름으로 저장 대화상자 (프로토타입에서는 생략)");
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("탭 닫기").shortcut_text(fmt(&s.close_tab)))
                    .clicked()
                {
                    self.request_close_tab(self.active);
                }
                if ui.button("종료").clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });

            ui.menu_button("편집", |ui| {
                for (label, shortcut) in [
                    (
                        "실행 취소",
                        KeyboardShortcut::new(Modifiers::COMMAND, Key::Z),
                    ),
                    (
                        "다시 실행",
                        if cfg!(target_os = "macos") {
                            KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                        } else {
                            KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)
                        },
                    ),
                    (
                        "잘라내기",
                        KeyboardShortcut::new(Modifiers::COMMAND, Key::X),
                    ),
                    ("복사", KeyboardShortcut::new(Modifiers::COMMAND, Key::C)),
                    (
                        "붙여넣기",
                        KeyboardShortcut::new(Modifiers::COMMAND, Key::V),
                    ),
                    (
                        "모두 선택",
                        KeyboardShortcut::new(Modifiers::COMMAND, Key::A),
                    ),
                ] {
                    if ui
                        .add(egui::Button::new(label).shortcut_text(fmt(&shortcut)))
                        .clicked()
                    {
                        self.toast(&ctx, format!("{label}: 에디터에서 단축키로 동작합니다"));
                    }
                }
                ui.separator();
                let goto = if cfg!(target_os = "macos") {
                    KeyboardShortcut::new(Modifiers::COMMAND, Key::L)
                } else {
                    KeyboardShortcut::new(Modifiers::COMMAND, Key::G)
                };
                if ui
                    .add(egui::Button::new("줄 이동…").shortcut_text(fmt(&goto)))
                    .clicked()
                {
                    self.toast(&ctx, "줄 이동 입력창 (Phase 2에서 구현)");
                }
            });

            ui.menu_button("검색", |ui| {
                if ui
                    .add(egui::Button::new("찾기…").shortcut_text(fmt(&s.find)))
                    .clicked()
                {
                    self.open_find(false);
                }
                if ui
                    .add(egui::Button::new("바꾸기…").shortcut_text(fmt(&s.replace)))
                    .clicked()
                {
                    self.open_find(true);
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("다음 찾기").shortcut_text(fmt(&s.find_next)))
                    .clicked()
                {
                    self.find_step(true);
                }
                if ui
                    .add(egui::Button::new("이전 찾기").shortcut_text(fmt(&s.find_prev)))
                    .clicked()
                {
                    self.find_step(false);
                }
            });

            ui.menu_button("보기", |ui| {
                ui.add(
                    egui::Button::selectable(self.sidebar_open, "사이드바")
                        .shortcut_text(fmt(&s.sidebar)),
                )
                .clicked()
                .then(|| self.sidebar_open = !self.sidebar_open);
                ui.checkbox(&mut self.word_wrap, "자동 줄바꿈");
                ui.separator();
                if ui
                    .add(egui::Button::new("확대").shortcut_text(fmt(&s.zoom_in)))
                    .clicked()
                {
                    self.zoom = (self.zoom + 0.1).min(3.0);
                }
                if ui
                    .add(egui::Button::new("축소").shortcut_text(fmt(&s.zoom_out)))
                    .clicked()
                {
                    self.zoom = (self.zoom - 0.1).max(0.5);
                }
                if ui
                    .add(egui::Button::new("원래 크기").shortcut_text(fmt(&s.zoom_reset)))
                    .clicked()
                {
                    self.zoom = 1.0;
                }
                ui.separator();
                ui.label(RichText::new("Markdown 보기").weak());
                let is_md = self.doc().is_markdown;
                ui.add_enabled_ui(is_md, |ui| {
                    let mut mode = self.doc().view_mode;
                    ui.radio_value(&mut mode, ViewMode::Source, "소스");
                    ui.radio_value(&mut mode, ViewMode::Split, "분할");
                    ui.radio_value(&mut mode, ViewMode::Preview, "미리보기");
                    self.doc_mut().view_mode = mode;
                    ui.checkbox(&mut self.scroll_sync, "스크롤 동기화");
                });
                ui.separator();
                ui.label(RichText::new("테마").weak());
                let before = self.theme;
                ui.radio_value(
                    &mut self.theme,
                    ThemePreference::System,
                    "시스템 설정 따르기",
                );
                ui.radio_value(&mut self.theme, ThemePreference::Light, "라이트");
                ui.radio_value(&mut self.theme, ThemePreference::Dark, "다크");
                if self.theme != before {
                    ctx.set_theme(self.theme);
                }
            });

            ui.menu_button("인코딩", |ui| {
                self.encoding_menu(ui);
                ui.separator();
                self.line_ending_menu(ui);
            });

            ui.menu_button("창", |ui| self.window_menu(ui, &ctx));

            ui.menu_button("도움말", |ui| {
                if ui.button("단축키 목록").clicked() {
                    self.dialog = Some(Dialog::Shortcuts);
                }
                if ui.button("정규식 문법").clicked() {
                    self.dialog = Some(Dialog::RegexHelp);
                }
                if ui.button("My-Notepad 정보").clicked() {
                    self.dialog = Some(Dialog::About);
                }
            });

            ui.menu_button(RichText::new("프로토타입").italics().weak(), |ui| {
                ui.label(RichText::new("검증용 메뉴 — 실제 앱에는 없음").weak());
                ui.separator();
                ui.menu_button("대화상자 미리보기", |ui| {
                    for (label, dialog) in [
                        (
                            "미저장 닫기 확인",
                            Dialog::Unsaved {
                                doc: self.active,
                                then_quit: false,
                            },
                        ),
                        ("표현 불가 문자 경고", Dialog::Unmappable),
                        ("외부 변경 감지", Dialog::ExternalChange),
                        ("바이너리 파일 확인", Dialog::Binary),
                        ("대용량 파일 경고", Dialog::LargeFile),
                        ("손상된 문서 저장 경고", Dialog::DamagedSave),
                    ] {
                        if ui.button(label).clicked() {
                            self.dialog = Some(dialog);
                        }
                    }
                });
                let mut many = self.many_tabs;
                if ui.checkbox(&mut many, "탭 많이 열기 (18개)").changed() {
                    self.set_many_tabs(many);
                }
                ui.separator();
                if ui.button("좁은 창 (820px)").clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(820.0, 700.0)));
                }
                if ui.button("넓은 창 (1280px)").clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(1280.0, 800.0)));
                }
            });
        });
    }

    fn encoding_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.label(RichText::new("다시 읽기 — 파일을 이 인코딩으로 다시 해석").weak());
        for &enc in ENCODINGS {
            if ui
                .add(egui::Button::selectable(self.doc().encoding == enc, enc))
                .clicked()
            {
                let doc = self.doc_mut();
                doc.encoding = enc;
                doc.encoding_state = EncodingState::Normal;
                self.toast(
                    &ctx,
                    format!("{enc}(으)로 다시 읽었습니다 (프로토타입: 내용은 그대로)"),
                );
                ui.close();
            }
        }
        ui.separator();
        ui.label(RichText::new("변환 저장 — 내용은 그대로, 저장할 인코딩만 변경").weak());
        for &enc in ENCODINGS {
            if ui.button(format!("{enc}(으)로 변환")).clicked() {
                let has_unmappable = self.doc().text.chars().any(|c| (c as u32) > 0xFFFF);
                if enc.starts_with("EUC-KR") && has_unmappable {
                    self.dialog = Some(Dialog::Unmappable);
                } else {
                    let doc = self.doc_mut();
                    doc.encoding = enc;
                    doc.encoding_state = EncodingState::Normal;
                    doc.modified = true;
                    self.toast(&ctx, format!("저장할 때 {enc}(으)로 저장합니다"));
                }
                ui.close();
            }
        }
    }

    fn line_ending_menu(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("줄바꿈").weak());
        if self.doc().mixed_line_endings {
            ui.label(
                RichText::new(
                    "이 파일은 줄바꿈이 섞여 있습니다.\n저장하면 선택한 방식으로 통일됩니다.",
                )
                .small(),
            );
        }
        for le in ["CRLF", "LF"] {
            let label = if le == "CRLF" {
                "CRLF (Windows)"
            } else {
                "LF (macOS / Linux)"
            };
            if ui
                .add(egui::Button::selectable(
                    self.doc().line_ending == le,
                    label,
                ))
                .clicked()
            {
                let doc = self.doc_mut();
                if doc.line_ending != le || doc.mixed_line_endings {
                    doc.line_ending = le;
                    doc.mixed_line_endings = false;
                    doc.modified = true;
                }
                ui.close();
            }
        }
    }

    // ---------------------------------------------------------------- 탭 바

    /// 탭 바 (UX-002): 한 줄 유지 + 선택 탭 자동 스크롤 + 넘칠 때 ‹ › 버튼 + 휠 가로 스크롤
    /// + 열린 탭 목록(⌄) 드롭다운.
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.visuals());
        let is_md = self.doc().is_markdown;
        let mut switch = None;
        let mut close = None;
        let mut active_rect = None;
        let mut tab_cmd = None;

        // 선택 탭이 바뀌었으면(단축키·사이드바·목록 등 어떤 경로든) 화면 안으로 스크롤한다.
        let scroll_active = self.tabs_last_active != Some(self.active);
        self.tabs_last_active = Some(self.active);

        let overflow = self.tabs_max_offset > 1.0;
        let step = (self.tabs_view_width * 0.6).max(120.0);
        let icon_size = ui.text_style_height(&egui::TextStyle::Body);
        let nav = |ui: &mut egui::Ui, glyph: &str, enabled: bool, tip: &str| {
            ui.add_enabled(
                enabled,
                egui::Button::new(icons::glyph(glyph).size(icon_size)).frame(false),
            )
            .on_hover_text(tip)
            .clicked()
        };

        ui.horizontal(|ui| {
            // 오른쪽 고정 영역: ›, ＋, ⌄, (Markdown 모드 토글)
            let right_width =
                34.0 * if overflow { 3.0 } else { 2.0 } + if is_md { 230.0 } else { 0.0 };

            if overflow
                && nav(
                    ui,
                    icons::ui::CARET_LEFT,
                    self.tabs_offset > 0.5,
                    "왼쪽 탭으로 이동",
                )
            {
                self.tabs_scroll_to = Some((self.tabs_offset - step).max(0.0));
            }

            let mut area = egui::ScrollArea::horizontal()
                .id_salt("tab-bar-scroll")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .max_width((ui.available_width() - right_width).max(100.0))
                .auto_shrink([false, true]);
            if let Some(offset) = self.tabs_scroll_to.take() {
                area = area.horizontal_scroll_offset(offset);
            }
            let output = ui
                .scope(|ui| {
                    // 세로 휠도 가로 스크롤로 쓰게 한다 (Shift 없이).
                    ui.style_mut().always_scroll_the_only_direction = true;
                    area.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, doc) in self.docs.iter().enumerate() {
                                let selected = i == self.active;
                                let frame = egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(6, 3))
                                    .corner_radius(4.0)
                                    .fill(if selected {
                                        ui.visuals().selection.bg_fill.gamma_multiply(0.35)
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .stroke(if selected {
                                        Stroke::new(1.0, ui.visuals().selection.stroke.color)
                                    } else {
                                        Stroke::NONE
                                    });
                                let shown = frame.show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    icons::show(ui, &doc.title, false, false, false);
                                    let title = RichText::new(&doc.title);
                                    let tab = ui.add(
                                        egui::Label::new(if selected {
                                            title.strong()
                                        } else {
                                            title
                                        })
                                        .sense(egui::Sense::click()),
                                    );
                                    let tab = tab.on_hover_text(
                                        doc.path.as_deref().unwrap_or("저장되지 않은 새 문서"),
                                    );
                                    tab.context_menu(|ui| {
                                        if let Some(cmd) =
                                            App::tab_context_menu(ui, i, doc.path.is_some())
                                        {
                                            tab_cmd = Some(cmd);
                                        }
                                    });
                                    if tab.clicked() {
                                        switch = Some(i);
                                    }
                                    if tab.middle_clicked() {
                                        close = Some(i);
                                    }
                                    if doc.modified {
                                        ui.label(RichText::new("●").color(palette.modified_marker))
                                            .on_hover_text("저장되지 않은 변경 내용");
                                    }
                                    if ui
                                        .add(egui::Button::new("×").small().frame(false))
                                        .on_hover_text("탭 닫기")
                                        .clicked()
                                    {
                                        close = Some(i);
                                    }
                                });
                                if selected {
                                    active_rect = Some(shown.response.rect);
                                    if scroll_active {
                                        ui.scroll_to_rect(shown.response.rect, None);
                                    }
                                }
                            }
                        });
                    })
                })
                .inner;
            // 첫 프레임에는 스크롤 범위가 확정되지 않아 덜 스크롤될 수 있으므로, 선택 탭이 완전히 보일 때까지 다음 프레임에 다시 맞춘다.
            if let Some(rect) = active_rect
                && !output.inner_rect.expand(1.0).contains_rect(rect)
                && rect.width() <= output.inner_rect.width()
                && scroll_active
            {
                self.tabs_last_active = None;
                ui.ctx().request_repaint();
            }
            self.tabs_offset = output.state.clamped_offset().x;
            self.tabs_view_width = output.inner_rect.width();
            self.tabs_max_offset = (output.content_size.x - output.inner_rect.width()).max(0.0);

            if overflow
                && nav(
                    ui,
                    icons::ui::CARET_RIGHT,
                    self.tabs_offset < self.tabs_max_offset - 0.5,
                    "오른쪽 탭으로 이동",
                )
            {
                self.tabs_scroll_to = Some((self.tabs_offset + step).min(self.tabs_max_offset));
            }
            if nav(ui, icons::ui::PLUS, true, "새 탭") {
                switch = Some(usize::MAX);
            }
            let list = ui.menu_button(icons::glyph(icons::ui::CARET_DOWN).size(icon_size), |ui| {
                if let Some(i) = self.open_tabs_list(ui) {
                    switch = Some(i);
                    ui.close();
                }
            });
            list.response
                .on_hover_text(format!("열린 탭 목록 ({}개)", self.docs.len()));

            if is_md {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ctx = ui.ctx().clone();
                    let s = &self.shortcuts;
                    let tips = [
                        (
                            ViewMode::Preview,
                            format!("미리보기 ({})", ctx.format_shortcut(&s.md_toggle)),
                        ),
                        (
                            ViewMode::Split,
                            format!(
                                "소스 + 미리보기 분할 ({} 다음 V)",
                                ctx.format_shortcut(&s.chord_k)
                            ),
                        ),
                        (
                            ViewMode::Source,
                            format!("소스 ({})", ctx.format_shortcut(&s.md_toggle)),
                        ),
                    ];
                    let mut mode = self.docs[self.active].view_mode;
                    for (m, tip) in tips {
                        if ui
                            .add(egui::Button::selectable(mode == m, m.label()))
                            .on_hover_text(tip)
                            .clicked()
                        {
                            mode = m;
                        }
                    }
                    ui.label(RichText::new("Markdown").weak().small());
                    self.docs[self.active].view_mode = mode;
                });
            }
        });

        match (switch, close) {
            (_, Some(i)) => self.request_close_tab(i),
            (Some(usize::MAX), _) => self.new_tab(),
            (Some(i), _) => self.switch_to(i),
            _ => {}
        }
        if let Some(cmd) = tab_cmd {
            let ctx = ui.ctx().clone();
            self.run_tab_command(&ctx, cmd);
        }
    }

    /// 열린 탭 목록 드롭다운: 검색어로 거르고, 고른 탭의 번호를 돌려준다.
    fn open_tabs_list(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        let palette = theme::palette(ui.visuals());
        ui.set_min_width(300.0);
        ui.horizontal(|ui| {
            ui.label(icons::glyph(icons::ui::MAGNIFYING_GLASS).weak());
            let filter = ui.add(
                egui::TextEdit::singleline(&mut self.tab_filter)
                    .hint_text("탭 이름으로 찾기")
                    .desired_width(240.0),
            );
            filter.request_focus();
        });
        ui.separator();
        let needle = self.tab_filter.to_lowercase();
        let mut picked = None;
        let mut shown = 0;
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                for (i, doc) in self.docs.iter().enumerate() {
                    if !needle.is_empty() && !doc.title.to_lowercase().contains(&needle) {
                        continue;
                    }
                    shown += 1;
                    let row = ui
                        .horizontal(|ui| {
                            icons::show(ui, &doc.title, false, false, false);
                            let mut text = RichText::new(&doc.title);
                            if i == self.active {
                                text = text.strong();
                            }
                            let r = ui.add(egui::Button::selectable(i == self.active, text));
                            if doc.modified {
                                ui.label(RichText::new("●").color(palette.modified_marker));
                            }
                            r
                        })
                        .inner;
                    if row
                        .on_hover_text(doc.path.as_deref().unwrap_or("저장되지 않은 새 문서"))
                        .clicked()
                    {
                        picked = Some(i);
                    }
                }
            });
        if shown == 0 {
            ui.label(RichText::new("일치하는 탭이 없습니다").weak());
        }
        if picked.is_some() {
            self.tab_filter.clear();
        }
        picked
    }

    // ---------------------------------------------------------------- 사이드바

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let toggle_tip = format!(
            "사이드바 닫기 ({})",
            ctx.format_shortcut(&self.shortcuts.sidebar)
        );
        ui.horizontal(|ui| {
            ui.strong("탐색기");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("«").frame(false))
                    .on_hover_text(toggle_tip)
                    .clicked()
                {
                    self.sidebar_open = false;
                }
                if ui.small_button("⟳").on_hover_text("새로고침").clicked() {
                    self.toast(&ctx, "폴더 내용을 다시 읽었습니다 (프로토타입)");
                }
                if ui
                    .small_button("폴더 열기")
                    .on_hover_text("다른 폴더 열기")
                    .clicked()
                {
                    self.toast(&ctx, "폴더 선택 대화상자 (프로토타입에서는 생략)");
                }
            });
        });
        ui.label(
            RichText::new("숨김: .git, node_modules · 흐림: .gitignore 대상")
                .small()
                .weak(),
        );
        ui.separator();

        let current = self.doc().title.clone();
        let mut cx = TreeCtx {
            current: &current,
            open_dirs: &mut self.open_dirs,
            open_file: None,
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                tree_ui(ui, &self.tree, 0, false, &mut cx);
            });
        if let Some((name, path)) = cx.open_file {
            self.open_from_tree(name, path);
        }
    }

    // ---------------------------------------------------------------- 찾기 바

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let matches = self.find.matches(&self.docs[self.active].text);
        if self.find.current >= matches.len() {
            self.find.current = 0;
        }
        let error = self.find.regex_error();

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 6))
            .fill(ui.visuals().faint_bg_color)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(if self.find.replace_open { "▾" } else { "▸" }).frame(false))
                        .on_hover_text("바꾸기 열기/닫기")
                        .clicked()
                    {
                        self.find.replace_open = !self.find.replace_open;
                    }
                    let query = ui.add(
                        egui::TextEdit::singleline(&mut self.find.query)
                            .hint_text("찾기")
                            .desired_width(260.0),
                    );
                    if self.find.focus_query {
                        query.request_focus();
                        self.find.focus_query = false;
                    }
                    if let Some(message) = error {
                        ui.painter().rect_stroke(
                            query.rect.expand(1.0),
                            3.0,
                            Stroke::new(1.5, theme::palette(ui.visuals()).status_error),
                            egui::StrokeKind::Outside,
                        );
                        query.clone().on_hover_text(message);
                    }
                    if query.changed() {
                        self.find.current = 0;
                    }
                    if query.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        let shift = ui.input(|i| i.modifiers.shift);
                        self.find_step(!shift);
                        query.request_focus();
                    }

                    ui.toggle_value(&mut self.find.regex, RichText::new(".*").monospace())
                        .on_hover_text("정규식 사용");
                    ui.toggle_value(&mut self.find.match_case, RichText::new("Aa").monospace())
                        .on_hover_text("대소문자 구분");
                    ui.toggle_value(&mut self.find.whole_word, RichText::new("\\b").monospace())
                        .on_hover_text("단어 단위로 일치");

                    let count = if error.is_some() {
                        RichText::new("정규식 오류").color(theme::palette(ui.visuals()).status_error)
                    } else if self.find.query.is_empty() {
                        RichText::new("").weak()
                    } else if matches.is_empty() {
                        RichText::new("결과 없음").weak()
                    } else {
                        RichText::new(format!("{} / {}", self.find.current + 1, matches.len()))
                    };
                    ui.label(count);

                    if ui.small_button("↑").on_hover_text("이전 찾기").clicked() {
                        self.find_step(false);
                    }
                    if ui.small_button("↓").on_hover_text("다음 찾기").clicked() {
                        self.find_step(true);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("닫기 (Esc)").clicked() {
                            self.find.open = false;
                        }
                    });
                });

                if self.find.replace_open {
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.find.replacement)
                                .hint_text("바꿀 내용")
                                .desired_width(260.0),
                        );
                        let enabled = !matches.is_empty();
                        if ui
                            .add_enabled(enabled, egui::Button::new("바꾸기"))
                            .on_hover_text("현재 항목을 바꾸고 다음으로 이동")
                            .clicked()
                        {
                            let m = matches[self.find.current].clone();
                            let replacement = self.find.replacement.clone();
                            let doc = self.doc_mut();
                            doc.text.replace_range(m, &replacement);
                            doc.modified = true;
                            let matches = self.find.matches(&self.docs[self.active].text);
                            if !matches.is_empty() {
                                self.find.current %= matches.len();
                                self.select_match(&matches);
                            }
                        }
                        if ui
                            .add_enabled(enabled, egui::Button::new("모두 바꾸기"))
                            .clicked()
                        {
                            let replacement = self.find.replacement.clone();
                            let doc = self.doc_mut();
                            for m in matches.iter().rev() {
                                doc.text.replace_range(m.clone(), &replacement);
                            }
                            doc.modified = true;
                            self.toast(
                                &ctx,
                                format!("{}개 바꿈 — 실행 취소 한 번으로 모두 되돌릴 수 있습니다 (본 구현)", matches.len()),
                            );
                        }
                    });
                }
            });
    }

    // ---------------------------------------------------------------- 에디터

    fn editor(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.visuals());
        let font = FontId::monospace(14.0 * self.zoom);
        let wrap = self.word_wrap;
        let find = self.find.clone();
        let current_match = self.find.current;
        let smart = self.smart_word.clone();
        let text_color = ui.visuals().text_color();
        let editor_id = Id::unique(("editor", &self.docs[self.active].title));

        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = buf.as_str();
            let mut marks: Vec<(Range<usize>, Color32)> = Vec::new();
            if let Some(word) = &smart {
                for r in highlight::find_all(text, word, true, false) {
                    marks.push((r, palette.smart_highlight));
                }
            }
            for (i, r) in find.matches(text).into_iter().enumerate() {
                let color = if i == current_match {
                    palette.search_current
                } else {
                    palette.search_match
                };
                marks.push((r, color));
            }
            let mut job = build_job(text, &marks, &font, text_color);
            job.wrap.max_width = if wrap { wrap_width } else { f32::INFINITY };
            ui.fonts_mut(|f| f.layout_job(job))
        };

        // 대기 중인 선택 지정 (찾기 이동, 미리보기 더블클릭)
        let pending = self.pending_select.take();
        if let Some(range) = &pending
            && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id)
        {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(range.start),
                    egui::text::CCursor::new(range.end),
                )));
            state.store(ui.ctx(), editor_id);
        }

        let line_count = self.docs[self.active].text.lines().count().max(1);
        let digits = line_count.to_string().len().max(3);
        let char_width = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let gutter_width = digits as f32 * char_width + 16.0;
        let current_line = self.cursor.0;

        let scroll = egui::ScrollArea::both()
            .id_salt(("editor-scroll", &self.docs[self.active].title))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let gutter_left = ui.cursor().min.x;
                    ui.add_space(gutter_width);
                    let doc = &mut self.docs[self.active];
                    let output = egui::TextEdit::multiline(&mut doc.text)
                        .id(editor_id)
                        .font(font.clone())
                        .frame(egui::Frame::NONE)
                        .lock_focus(true)
                        .desired_width(if wrap {
                            ui.available_width()
                        } else {
                            f32::INFINITY
                        })
                        .desired_rows(40)
                        .layouter(&mut layouter)
                        .show(ui);

                    // 줄 번호: 논리 줄의 첫 행에만 표시
                    let painter = ui.painter();
                    let mut line = 1;
                    let mut starts_line = true;
                    for row in &output.galley.rows {
                        if starts_line {
                            let y = output.galley_pos.y + row.rect().center().y;
                            painter.text(
                                egui::pos2(gutter_left + gutter_width - 10.0, y),
                                egui::Align2::RIGHT_CENTER,
                                line.to_string(),
                                font.clone(),
                                if line == current_line {
                                    palette.gutter_current
                                } else {
                                    palette.gutter_text
                                },
                            );
                            line += 1;
                        }
                        starts_line = row.ends_with_newline;
                    }

                    if pending.is_some() {
                        output.response.request_focus();
                        if let Some(range) = &pending {
                            let pos = output
                                .galley
                                .pos_from_cursor(egui::text::CCursor::new(range.start));
                            ui.scroll_to_rect(
                                pos.translate(output.galley_pos.to_vec2()),
                                Some(Align::Center),
                            );
                        }
                    }
                    output
                })
                .inner
            });

        let output = scroll.inner;
        let content_height = scroll.content_size.y - scroll.inner_rect.height();
        self.source_scroll_fraction = if content_height > 0.0 {
            (scroll.state.clamped_offset().y / content_height).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let doc = &mut self.docs[self.active];
        if output.response.changed() {
            doc.modified = true;
        }
        if let Some(range) = output.cursor_range {
            let a = range.primary.index.0;
            let b = range.secondary.index.0;
            self.cursor = highlight::line_col(&doc.text, a);
            self.selection_chars = a.abs_diff(b);
            let (lo, hi) = (a.min(b), a.max(b));
            let selected: String = doc.text.chars().skip(lo).take(hi - lo).collect();
            self.smart_word = highlight::smart_word(&selected).map(str::to_owned);
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui, sync: bool) {
        let palette = theme::palette(ui.visuals());
        let cursor_line = Some(self.cursor.0 - 1);
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("preview-scroll", &self.docs[self.active].title))
            .auto_shrink([false, false]);
        let mut target_offset = None;
        if sync {
            // 간이 비율 동기화: 소스 스크롤 비율을 그대로 적용한다 (본 구현은 P0-09 / P3-08)
            let id =
                ui.make_persistent_id(("preview-content-height", &self.docs[self.active].title));
            if let Some(content_height) = ui.data(|d| d.get_temp::<f32>(id)) {
                let offset = self.source_scroll_fraction * content_height;
                scroll = scroll.vertical_scroll_offset(offset);
                target_offset = Some(id);
            } else {
                target_offset = Some(id);
            }
        }
        let output = scroll.show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 8))
                .show(ui, |ui| {
                    markdown::preview(
                        ui,
                        &self.docs[self.active].text,
                        &palette,
                        self.zoom,
                        cursor_line,
                    )
                })
                .inner
        });
        if let Some(id) = target_offset {
            let scrollable = (output.content_size.y - output.inner_rect.height()).max(0.0);
            ui.data_mut(|d| d.insert_temp(id, scrollable));
        }
        if let Some(line) = output.inner {
            let text = &self.docs[self.active].text;
            let start: usize = text
                .split_inclusive('\n')
                .take(line)
                .map(|l| l.chars().count())
                .sum();
            self.pending_select = Some(start..start);
            if self.doc().view_mode == ViewMode::Preview {
                self.doc_mut().view_mode = ViewMode::Split;
            }
        }
    }

    // ---------------------------------------------------------------- 상태 표시줄

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.visuals());
        let now = ui.input(|i| i.time);
        if self.toast.as_ref().is_some_and(|(_, until)| now > *until) {
            self.toast = None;
        }
        ui.horizontal(|ui| {
            ui.label(format!("Ln {}, Col {}", self.cursor.0, self.cursor.1));
            if self.selection_chars > 0 {
                ui.separator();
                ui.label(format!("선택 {}자", self.selection_chars));
            }
            ui.separator();
            ui.label(format!("{}줄", self.doc().text.lines().count().max(1)));
            if let Some(word) = &self.smart_word {
                ui.separator();
                let count = highlight::find_all(&self.doc().text, word, true, false).len();
                ui.label(RichText::new(format!("'{word}' {count}개 강조")).weak());
            }
            if let Some((toast, _)) = &self.toast {
                ui.separator();
                ui.label(RichText::new(toast).color(ui.visuals().hyperlink_color));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let doc = self.doc();
                let mut enc_text = RichText::new(doc.encoding);
                let mut enc_tip = "인코딩 — 클릭해서 다시 읽기 / 변환 저장".to_owned();
                match doc.encoding_state {
                    EncodingState::Guessed => {
                        enc_text = RichText::new(format!("{} · 추정됨", doc.encoding))
                            .color(palette.status_warning);
                        enc_tip = "자동 감지로 추정한 인코딩입니다. 글자가 깨져 보이면 '다시 읽기'로 바꿔 보세요.".to_owned();
                    }
                    EncodingState::Damaged => {
                        enc_text = RichText::new(format!("{} · 손상", doc.encoding))
                            .color(palette.status_error);
                        enc_tip = "일부 바이트를 해석하지 못했습니다(�). 저장하면 원본 바이트가 사라집니다.".to_owned();
                    }
                    EncodingState::Normal => {}
                }
                let le_text = if doc.mixed_line_endings {
                    RichText::new(format!("Mixed ({})", doc.line_ending)).color(palette.status_warning)
                } else {
                    RichText::new(doc.line_ending)
                };

                ui.menu_button(enc_text, |ui| self.encoding_menu(ui))
                    .response
                    .on_hover_text(enc_tip);
                ui.separator();
                ui.menu_button(le_text, |ui| self.line_ending_menu(ui))
                    .response
                    .on_hover_text("줄바꿈 방식 — 클릭해서 변경");
                ui.separator();
                ui.label(format!("{:.0}%", self.zoom * 100.0))
                    .on_hover_text("확대/축소: Ctrl + = / Ctrl + - / Ctrl + 0");
                if self.doc().is_markdown {
                    ui.separator();
                    ui.label(format!("Markdown · {}", self.doc().view_mode.label()));
                }
            });
        });
    }

    // ---------------------------------------------------------------- 대화상자

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog else {
            return;
        };
        let mut close = false;
        let modal = egui::Modal::new(Id::unique("dialog")).show(ctx, |ui| {
            ui.set_width(440.0);
            match dialog {
                Dialog::Unsaved { doc, then_quit } => {
                    let title = self.docs[doc].title.clone();
                    ui.heading("변경 내용을 저장할까요?");
                    ui.label(format!("\"{title}\"의 변경 내용이 저장되지 않았습니다."));
                    if !self.close_queue.is_empty() {
                        ui.label(
                            RichText::new(format!("이어서 닫을 탭 {}개가 더 있습니다. 취소하면 모두 멈춥니다.", self.close_queue.len()))
                                .small()
                                .weak(),
                        );
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary("저장")).clicked() {
                            self.docs[doc].modified = false;
                            self.after_unsaved(ctx, doc, then_quit);
                            close = true;
                        }
                        if ui.button("저장 안 함").clicked() {
                            self.docs[doc].modified = false;
                            self.after_unsaved(ctx, doc, then_quit);
                            close = true;
                        }
                        if ui.button("취소").clicked() {
                            // 일괄 닫기 중이었다면 남은 대기열도 취소한다.
                            self.close_queue.clear();
                            close = true;
                        }
                    });
                }
                Dialog::Unmappable => {
                    ui.heading("EUC-KR로 저장할 수 없는 문자가 있습니다");
                    ui.label("아래 문자는 EUC-KR (CP949)에 없습니다. 그대로 저장하면 '?'로 바뀝니다.");
                    ui.add_space(4.0);
                    egui::Grid::new("unmappable").striped(true).show(ui, |ui| {
                        ui.strong("문자");
                        ui.strong("위치");
                        ui.end_row();
                        ui.label("👍 (U+1F44D)");
                        ui.label("Ln 13, Col 12");
                        ui.end_row();
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary("UTF-8로 저장")).clicked() {
                            close = true;
                        }
                        if ui.button("'?'로 바꿔 저장").clicked() {
                            close = true;
                        }
                        if ui.button("취소").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::ExternalChange => {
                    ui.heading("파일이 다른 프로그램에서 변경되었습니다");
                    ui.label("\"server.log\"가 디스크에서 변경되었습니다. 다시 불러올까요?");
                    ui.label(RichText::new("편집 중인 내용이 있으면 사라집니다.").weak());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary("다시 불러오기")).clicked() {
                            close = true;
                        }
                        if ui.button("내 버전 유지").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Binary => {
                    ui.heading("텍스트 파일이 아닌 것 같습니다");
                    ui.label("\"image.dat\"에 텍스트가 아닌 데이터가 있습니다. 열면 내용이 깨져 보이고, 저장하면 파일이 손상될 수 있습니다.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("읽기 전용으로 열기").clicked() {
                            close = true;
                        }
                        if ui.add(primary("취소")).clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::LargeFile => {
                    ui.heading("파일이 큽니다");
                    ui.label("\"huge.log\" (85MB)는 편집 가능한 크기(10MB)를 넘습니다. 읽기 전용으로 엽니다.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary("읽기 전용으로 열기")).clicked() {
                            close = true;
                        }
                        if ui.button("취소").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::DamagedSave => {
                    ui.heading("손상된 문서를 저장하려고 합니다");
                    ui.label("이 문서는 열 때 해석하지 못한 바이트(�)가 있습니다. 지금 저장하면 원본 바이트가 영구히 사라집니다.");
                    ui.label(RichText::new("인코딩 → '다시 읽기'로 올바른 인코딩을 먼저 시도해 보세요.").weak());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary("취소")).clicked() {
                            close = true;
                        }
                        if ui.button("그래도 저장").clicked() {
                            self.doc_mut().modified = false;
                            close = true;
                        }
                    });
                }
                Dialog::Shortcuts => {
                    ui.heading("단축키 목록");
                    let s = &self.shortcuts;
                    egui::Grid::new("shortcuts").striped(true).show(ui, |ui| {
                        for (label, shortcut) in [
                            ("새 탭", &s.new_tab),
                            ("열기", &s.open),
                            ("저장", &s.save),
                            ("다른 이름으로 저장", &s.save_as),
                            ("탭 닫기", &s.close_tab),
                            ("찾기", &s.find),
                            ("바꾸기", &s.replace),
                            ("다음 찾기", &s.find_next),
                            ("이전 찾기", &s.find_prev),
                            ("사이드바", &s.sidebar),
                            ("Markdown 소스 ↔ 미리보기", &s.md_toggle),
                            ("확대", &s.zoom_in),
                            ("축소", &s.zoom_out),
                        ] {
                            ui.label(label);
                            ui.monospace(ctx.format_shortcut(shortcut));
                            ui.end_row();
                        }
                        ui.label("Markdown 분할 보기");
                        ui.monospace(format!("{} 다음 V", ctx.format_shortcut(&s.chord_k)));
                        ui.end_row();
                        let t = &self.tab_shortcuts;
                        let join = |list: &[KeyboardShortcut]| {
                            list.iter()
                                .map(|s| ctx.format_shortcut(s))
                                .collect::<Vec<_>>()
                                .join(" / ")
                        };
                        for (label, keys) in [
                            ("다음 탭", join(&t.next)),
                            ("이전 탭", join(&t.prev)),
                            ("탭을 왼쪽 / 오른쪽으로 이동", format!("{} / {}", join(&t.move_left), join(&t.move_right))),
                            ("1~8번째 탭으로", format!("{} ~ {}", ctx.format_shortcut(&t.goto[0]), ctx.format_shortcut(&t.goto[7]))),
                            ("마지막 탭으로", ctx.format_shortcut(&t.last)),
                            ("모든 탭 닫기", ctx.format_shortcut(&t.close_all)),
                        ] {
                            ui.label(label);
                            ui.monospace(keys);
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                    if ui.add(primary("닫기")).clicked() {
                        close = true;
                    }
                }
                Dialog::RegexHelp => {
                    ui.heading("정규식 문법");
                    ui.label("Rust regex 문법을 사용합니다.");
                    ui.label(RichText::new("전후방 탐색(lookaround)과 역참조(\\1)는 지원하지 않습니다.").color(theme::palette(ui.visuals()).status_warning));
                    egui::Grid::new("regex").striped(true).show(ui, |ui| {
                        for (p, d) in [
                            (".", "아무 문자 하나"),
                            ("\\d  \\w  \\s", "숫자 · 단어 문자 · 공백"),
                            ("[가-힣]+", "한글 음절 연속"),
                            ("^  $", "줄 시작 · 줄 끝"),
                            ("(a|b)", "그룹 · 선택"),
                            ("$1  ${name}", "바꾸기에서 캡처 그룹 참조"),
                        ] {
                            ui.monospace(p);
                            ui.label(d);
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                    if ui.add(primary("닫기")).clicked() {
                        close = true;
                    }
                }
                Dialog::WindowList => {
                    if self.window_list_ui(ui, ctx) {
                        close = true;
                    }
                }
                Dialog::About => {
                    ui.heading("My-Notepad");
                    ui.label("UX 프로토타입 (P0-UX1) · egui 0.37");
                    ui.label(RichText::new("가짜 데이터로 화면만 확인하는 빌드입니다.").weak());
                    ui.add_space(8.0);
                    if ui.add(primary("닫기")).clicked() {
                        close = true;
                    }
                }
            }
        });
        if modal.should_close() {
            // Esc·바깥 클릭은 취소와 같다.
            self.close_queue.clear();
            close = true;
        }
        if close && matches!(self.dialog, Some(d) if same_dialog(d, dialog)) {
            self.dialog = None;
        }
    }

    fn after_unsaved(&mut self, ctx: &egui::Context, doc: usize, then_quit: bool) {
        if then_quit {
            if let Some(next) = self.docs.iter().position(|d| d.modified) {
                // 다음 미저장 문서를 이어서 묻는다
                self.dialog = Some(Dialog::Unsaved {
                    doc: next,
                    then_quit: true,
                });
            } else {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        } else {
            self.close_tab(doc);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // 창 닫기 가로채기: 미저장 문서가 있으면 취소하고 확인 대화상자
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && let Some(doc) = self.docs.iter().position(|d| d.modified)
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::Unsaved {
                doc,
                then_quit: true,
            });
        }

        self.handle_tab_shortcuts(&ctx);
        self.handle_shortcuts(&ctx);
        self.process_close_queue();

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("tabs").show(ui, |ui| self.tab_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.sidebar_open {
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(220.0)
                .size_range(140.0..=480.0)
                .show(ui, |ui| self.sidebar(ui));
        } else {
            // 닫힌 상태에서도 화면에서 다시 열 수 있도록 좁은 세로 띠를 남긴다.
            let tip = format!(
                "사이드바 열기 ({})",
                ctx.format_shortcut(&self.shortcuts.sidebar)
            );
            egui::Panel::left("sidebar-rail")
                .resizable(false)
                .exact_size(30.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(4.0);
                        if ui
                            .add(egui::Button::new("»").frame(false))
                            .on_hover_text(&tip)
                            .clicked()
                        {
                            self.sidebar_open = true;
                        }
                        // 띠의 빈 곳을 클릭해도 열린다.
                        let rest = ui.allocate_response(ui.available_size(), egui::Sense::click());
                        if rest.on_hover_text(&tip).clicked() {
                            self.sidebar_open = true;
                        }
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0))
            .show(ui, |ui| {
                if self.find.open {
                    self.find_bar(ui);
                    ui.separator();
                }
                let doc = self.doc();
                let mode = if doc.is_markdown {
                    doc.view_mode
                } else {
                    ViewMode::Source
                };
                let narrow = ui.available_width() < SPLIT_MIN_WIDTH;
                match mode {
                    ViewMode::Split if narrow => {
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("창이 좁아 분할 대신 소스만 표시합니다 — 탭 바 오른쪽에서 미리보기로 전환")
                                    .small()
                                    .weak(),
                            );
                        });
                        self.editor(ui);
                    }
                    ViewMode::Split => {
                        let half = ui.available_width() / 2.0;
                        let sync = self.scroll_sync;
                        egui::Panel::right("preview-panel")
                            .resizable(true)
                            .default_size(half)
                            .size_range(200.0..=ui.available_width() - 200.0)
                            .show(ui, |ui| self.preview(ui, sync));
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(4, 4)).show(ui, |ui| self.editor(ui));
                    }
                    ViewMode::Preview => self.preview(ui, false),
                    ViewMode::Source => {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(4, 4)).show(ui, |ui| self.editor(ui));
                    }
                }
            });

        self.dialogs(&ctx);
    }
}

fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong())
}

fn same_dialog(a: Dialog, b: Dialog) -> bool {
    std::mem::discriminant(&a) == std::mem::discriminant(&b)
}

struct TreeCtx<'a> {
    current: &'a str,
    open_dirs: &'a mut std::collections::HashSet<&'static str>,
    open_file: Option<(&'static str, &'static str)>,
}

/// 폴더 트리 한 줄씩: 들여쓰기 · 펼침 화살표 · 아이콘 · 이름. `.gitignore` 대상은 흐리게.
fn tree_ui(ui: &mut egui::Ui, node: &TreeNode, depth: usize, ignored: bool, cx: &mut TreeCtx<'_>) {
    let ignored = ignored || data::is_ignored(node);
    let is_dir = node.is_dir();
    let is_open = is_dir && cx.open_dirs.contains(node.path);
    let selected = !is_dir && node.name == cx.current;

    let row = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(depth as f32 * 14.0);
            let arrow = if !is_dir {
                " "
            } else if is_open {
                "▾"
            } else {
                "▸"
            };
            ui.label(RichText::new(arrow).weak());
            icons::show(ui, node.name, is_dir, is_open, ignored);
            let mut text = RichText::new(node.name);
            if ignored {
                text = text.weak();
            }
            ui.add(egui::Button::selectable(selected, text).frame_when_inactive(false))
        })
        .inner;
    let row = row.on_hover_text(if ignored {
        format!("{} (.gitignore 대상)", node.path)
    } else {
        node.path.to_owned()
    });

    if row.clicked() {
        if is_dir {
            if is_open {
                cx.open_dirs.remove(node.path);
            } else {
                cx.open_dirs.insert(node.path);
            }
        } else {
            cx.open_file = Some((node.name, node.path));
        }
    }
    if is_open {
        for child in &node.children {
            tree_ui(ui, child, depth + 1, ignored, cx);
        }
    }
}

/// 배경색 구간(marks)을 반영한 LayoutJob. 겹치는 구간은 나중 것이 우선한다.
fn build_job(
    text: &str,
    marks: &[(Range<usize>, Color32)],
    font: &FontId,
    color: Color32,
) -> LayoutJob {
    let mut bounds: Vec<usize> = vec![0, text.len()];
    for (r, _) in marks {
        bounds.push(r.start);
        bounds.push(r.end);
    }
    bounds.sort_unstable();
    bounds.dedup();

    let mut job = LayoutJob::default();
    for w in bounds.windows(2) {
        let (start, end) = (w[0], w[1]);
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        let background = marks
            .iter()
            .rev()
            .find(|(r, _)| r.start <= start && end <= r.end)
            .map_or(Color32::TRANSPARENT, |(_, c)| *c);
        job.append(
            &text[start..end],
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                background,
                ..Default::default()
            },
        );
    }
    job
}
