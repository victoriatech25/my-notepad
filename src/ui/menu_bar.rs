//! 상단 메뉴바 UI (`파일 · 편집 · 검색 · 보기 · 인코딩 · 창 · 도움말` — PLAN.md §3.1, §3.5, P1-03).
//!
//! 모든 메뉴 항목은 [`Keymap`]을 통해 현재 OS에 맞는 단축키 표기를 오른쪽에 표시하고,
//! 클릭 시 [`Command`]를 반환하여 단축키 입력과 완전히 동일한 경로(`app.rs`)로 처리되게 한다.

use eframe::egui::{self, Color32, RichText};

use crate::commands::{Command, EncodingChoice, Keymap, LineEndingChoice, ThemeMode, ViewMode};

/// `창` 메뉴 하단 열린 탭 바로가기 목록에 표시할 탭 요약 정보.
#[derive(Debug, Clone, Copy)]
pub struct MenuTabEntry<'a> {
    pub title: &'a str,
    pub path_hint: &'a str,
    pub modified: bool,
}

/// 메뉴바 렌더링에 필요한 현재 앱/활성 문서 상태 스냅샷.
#[derive(Debug, Clone, Copy)]
pub struct MenuBarState<'a> {
    pub sidebar_open: bool,
    pub word_wrap: bool,
    pub is_markdown: bool,
    pub view_mode: ViewMode,
    pub scroll_sync: bool,
    pub theme: ThemeMode,
    pub encoding: EncodingChoice,
    pub line_ending: LineEndingChoice,
    pub mixed_line_endings: bool,
    pub recent_files: &'a [&'a str],
    pub tabs: &'a [MenuTabEntry<'a>],
    pub active_tab: usize,
    pub modified_marker_color: Color32,
}

/// 상단 메뉴바를 렌더링하고 사용자가 클릭한 [`Command`]가 있으면 반환한다.
pub fn show(ui: &mut egui::Ui, keymap: &Keymap, state: &MenuBarState<'_>) -> Option<Command> {
    let ctx = ui.ctx().clone();
    let mut triggered = None;

    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("파일", |ui| {
            file_menu(ui, &ctx, keymap, state, &mut triggered);
        });
        ui.menu_button("편집", |ui| {
            edit_menu(ui, &ctx, keymap, &mut triggered);
        });
        ui.menu_button("검색", |ui| {
            search_menu(ui, &ctx, keymap, &mut triggered);
        });
        ui.menu_button("보기", |ui| {
            view_menu(ui, &ctx, keymap, state, &mut triggered);
        });
        ui.menu_button("인코딩", |ui| {
            encoding_menu(ui, state, &mut triggered);
        });
        ui.menu_button("창", |ui| {
            window_menu(ui, &ctx, keymap, state, &mut triggered);
        });
        ui.menu_button("도움말", |ui| {
            help_menu(ui, &mut triggered);
        });
    });

    triggered
}

fn menu_item(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    keymap: &Keymap,
    label: &str,
    command: Command,
    triggered: &mut Option<Command>,
) {
    let mut button = egui::Button::new(label);
    if let Some(shortcut_text) = keymap.format_shortcut_for(command, ctx) {
        button = button.shortcut_text(shortcut_text);
    }
    if ui.add(button).clicked() {
        *triggered = Some(command);
        ui.close();
    }
}

fn selectable_menu_item(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    keymap: &Keymap,
    selected: bool,
    label: &str,
    command: Command,
    triggered: &mut Option<Command>,
) {
    let mut button = egui::Button::selectable(selected, label);
    if let Some(shortcut_text) = keymap.format_shortcut_for(command, ctx) {
        button = button.shortcut_text(shortcut_text);
    }
    if ui.add(button).clicked() {
        *triggered = Some(command);
        ui.close();
    }
}

fn file_menu(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    keymap: &Keymap,
    state: &MenuBarState<'_>,
    triggered: &mut Option<Command>,
) {
    menu_item(ui, ctx, keymap, "새 탭", Command::NewTab, triggered);
    menu_item(ui, ctx, keymap, "열기…", Command::OpenFile, triggered);
    menu_item(
        ui,
        ctx,
        keymap,
        "폴더 열기…",
        Command::OpenFolder,
        triggered,
    );

    ui.menu_button("최근 파일", |ui| {
        if state.recent_files.is_empty() {
            ui.label(RichText::new("최근에 연 파일이 없습니다").weak());
        } else {
            for (idx, &name) in state.recent_files.iter().enumerate() {
                if ui.button(name).clicked() {
                    *triggered = Some(Command::OpenRecent(idx));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("목록 지우기").clicked() {
                *triggered = Some(Command::ClearRecentFiles);
                ui.close();
            }
        }
    });

    ui.separator();
    menu_item(ui, ctx, keymap, "저장", Command::Save, triggered);
    menu_item(
        ui,
        ctx,
        keymap,
        "다른 이름으로 저장…",
        Command::SaveAs,
        triggered,
    );
    ui.separator();
    menu_item(ui, ctx, keymap, "탭 닫기", Command::CloseTab, triggered);
    menu_item(ui, ctx, keymap, "종료", Command::Exit, triggered);
}

fn edit_menu(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    keymap: &Keymap,
    triggered: &mut Option<Command>,
) {
    menu_item(ui, ctx, keymap, "실행 취소", Command::Undo, triggered);
    menu_item(ui, ctx, keymap, "다시 실행", Command::Redo, triggered);
    ui.separator();
    menu_item(ui, ctx, keymap, "잘라내기", Command::Cut, triggered);
    menu_item(ui, ctx, keymap, "복사", Command::Copy, triggered);
    menu_item(ui, ctx, keymap, "붙여넣기", Command::Paste, triggered);
    menu_item(ui, ctx, keymap, "모두 선택", Command::SelectAll, triggered);
    ui.separator();
    menu_item(ui, ctx, keymap, "줄 이동…", Command::GoToLine, triggered);
}

fn search_menu(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    keymap: &Keymap,
    triggered: &mut Option<Command>,
) {
    menu_item(ui, ctx, keymap, "찾기…", Command::Find, triggered);
    menu_item(ui, ctx, keymap, "바꾸기…", Command::Replace, triggered);
    ui.separator();
    menu_item(ui, ctx, keymap, "다음 찾기", Command::FindNext, triggered);
    menu_item(ui, ctx, keymap, "이전 찾기", Command::FindPrev, triggered);
}

fn view_menu(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    keymap: &Keymap,
    state: &MenuBarState<'_>,
    triggered: &mut Option<Command>,
) {
    selectable_menu_item(
        ui,
        ctx,
        keymap,
        state.sidebar_open,
        "사이드바",
        Command::ToggleSidebar,
        triggered,
    );
    selectable_menu_item(
        ui,
        ctx,
        keymap,
        state.word_wrap,
        "자동 줄바꿈",
        Command::ToggleWordWrap,
        triggered,
    );

    ui.separator();
    menu_item(ui, ctx, keymap, "확대", Command::ZoomIn, triggered);
    menu_item(ui, ctx, keymap, "축소", Command::ZoomOut, triggered);
    menu_item(ui, ctx, keymap, "원래 크기", Command::ZoomReset, triggered);

    ui.separator();
    ui.label(RichText::new("Markdown 보기").weak());
    ui.add_enabled_ui(state.is_markdown, |ui| {
        for mode in [ViewMode::Source, ViewMode::Split, ViewMode::Preview] {
            selectable_menu_item(
                ui,
                ctx,
                keymap,
                state.view_mode == mode,
                mode.label(),
                Command::SetViewMode(mode),
                triggered,
            );
        }
        menu_item(
            ui,
            ctx,
            keymap,
            "소스 ↔ 미리보기 전환",
            Command::ToggleMdPreview,
            triggered,
        );
        menu_item(
            ui,
            ctx,
            keymap,
            "분할 보기 토글",
            Command::ToggleMdSplit,
            triggered,
        );
        selectable_menu_item(
            ui,
            ctx,
            keymap,
            state.scroll_sync,
            "스크롤 동기화",
            Command::ToggleScrollSync,
            triggered,
        );
    });

    ui.separator();
    ui.label(RichText::new("테마").weak());
    for theme in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
        selectable_menu_item(
            ui,
            ctx,
            keymap,
            state.theme == theme,
            theme.label(),
            Command::SetTheme(theme),
            triggered,
        );
    }
}

fn encoding_menu(ui: &mut egui::Ui, state: &MenuBarState<'_>, triggered: &mut Option<Command>) {
    ui.label(RichText::new("다시 읽기 — 파일을 이 인코딩으로 다시 해석").weak());
    for enc in EncodingChoice::ALL {
        if ui
            .add(egui::Button::selectable(state.encoding == enc, enc.label()))
            .clicked()
        {
            *triggered = Some(Command::ReopenWithEncoding(enc));
            ui.close();
        }
    }

    ui.separator();
    ui.label(RichText::new("변환 저장 — 내용은 그대로, 저장할 인코딩만 변경").weak());
    for enc in EncodingChoice::ALL {
        if ui.button(format!("{}(으)로 변환", enc.label())).clicked() {
            *triggered = Some(Command::ConvertToEncoding(enc));
            ui.close();
        }
    }

    ui.separator();
    ui.label(RichText::new("줄바꿈").weak());
    if state.mixed_line_endings {
        ui.label(
            RichText::new(
                "이 파일은 줄바꿈이 섞여 있습니다.\n저장하면 선택한 방식으로 통일됩니다.",
            )
            .small(),
        );
    }
    for le in LineEndingChoice::ALL {
        if ui
            .add(egui::Button::selectable(
                state.line_ending == le,
                le.menu_label(),
            ))
            .clicked()
        {
            *triggered = Some(Command::SetLineEnding(le));
            ui.close();
        }
    }
}

fn window_menu(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    keymap: &Keymap,
    state: &MenuBarState<'_>,
    triggered: &mut Option<Command>,
) {
    menu_item(ui, ctx, keymap, "다음 탭", Command::NextTab, triggered);
    menu_item(ui, ctx, keymap, "이전 탭", Command::PrevTab, triggered);
    menu_item(
        ui,
        ctx,
        keymap,
        "탭을 왼쪽으로 이동",
        Command::MoveTabLeft,
        triggered,
    );
    menu_item(
        ui,
        ctx,
        keymap,
        "탭을 오른쪽으로 이동",
        Command::MoveTabRight,
        triggered,
    );

    ui.separator();
    menu_item(ui, ctx, keymap, "탭 닫기", Command::CloseTab, triggered);
    menu_item(
        ui,
        ctx,
        keymap,
        "다른 탭 모두 닫기",
        Command::CloseOtherTabs,
        triggered,
    );
    menu_item(
        ui,
        ctx,
        keymap,
        "오른쪽 탭 모두 닫기",
        Command::CloseTabsToRight,
        triggered,
    );
    menu_item(
        ui,
        ctx,
        keymap,
        "저장된 탭 닫기",
        Command::CloseSavedTabs,
        triggered,
    );
    menu_item(
        ui,
        ctx,
        keymap,
        "모든 탭 닫기",
        Command::CloseAllTabs,
        triggered,
    );
    menu_item(
        ui,
        ctx,
        keymap,
        "탭 경로 복사",
        Command::CopyTabPath,
        triggered,
    );

    ui.separator();
    menu_item(
        ui,
        ctx,
        keymap,
        "창 목록…",
        Command::ShowWindowList,
        triggered,
    );

    if !state.tabs.is_empty() {
        ui.separator();
        let last_idx = state.tabs.len() - 1;
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                for (idx, tab) in state.tabs.iter().enumerate() {
                    let shortcut_text = if idx < 8 {
                        keymap
                            .format_shortcut_for(Command::GotoTab(idx), ctx)
                            .unwrap_or_default()
                    } else if idx == last_idx {
                        keymap
                            .format_shortcut_for(Command::LastTab, ctx)
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };

                    let mut text = RichText::new(format!("{}  {}", idx + 1, tab.title));
                    if tab.modified {
                        text = text.color(state.modified_marker_color);
                    }
                    let response = ui.add(
                        egui::Button::selectable(idx == state.active_tab, text)
                            .shortcut_text(shortcut_text),
                    );
                    if response.on_hover_text(tab.path_hint).clicked() {
                        *triggered = Some(Command::GotoTab(idx));
                        ui.close();
                    }
                }
            });
    }
}

fn help_menu(ui: &mut egui::Ui, triggered: &mut Option<Command>) {
    for (label, cmd) in [
        ("단축키 목록", Command::ShowShortcuts),
        ("정규식 문법", Command::ShowRegexHelp),
        ("My-Notepad 정보", Command::ShowAbout),
    ] {
        if ui.button(label).clicked() {
            *triggered = Some(cmd);
            ui.close();
        }
    }
}
