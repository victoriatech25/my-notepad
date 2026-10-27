//! 상단 메뉴바 UI (`파일 · 편집 · 검색 · 보기 · 인코딩 · 창 · 도움말` — PLAN.md §3.1, §3.5, P1-03).
//!
//! 모든 메뉴 항목은 [`Keymap`]을 통해 현재 OS에 맞는 단축키 표기를 오른쪽에 표시하고,
//! 클릭 시 [`Command`]를 반환하여 단축키 입력과 완전히 동일한 경로(`app.rs`)로 처리되게 한다.

use eframe::egui::{self, Align, Color32, FontSelection, RichText, text::LayoutJob};

use crate::ui::theme::Palette;

use crate::commands::{Command, EncodingChoice, Keymap, LineEndingChoice, ThemeMode, ViewMode};

/// `창` 메뉴에서 스크롤 없이 보여 줄 열린 탭 바로가기 수.
const WINDOW_MENU_VISIBLE_TABS: usize = 10;

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
    /// 탭 너비 (공백 칸 수)
    pub tab_width: u8,
    /// `Tab` 키로 공백 넣기
    pub tab_spaces: bool,
    pub is_markdown: bool,
    pub view_mode: ViewMode,
    pub scroll_sync: bool,
    /// 선택 단어 강조: 단어 단위 일치 (끄면 부분 일치)
    pub highlight_whole_word: bool,
    /// 선택 단어 강조: 대소문자 구분
    pub highlight_match_case: bool,
    /// 찾기: 끝에서 처음으로 이어서 찾기
    pub search_wrap_around: bool,
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
            search_menu(ui, &ctx, keymap, state, &mut triggered);
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

/// 메뉴의 선택 가능한 항목 글자: 왼쪽에 선택 표시 점 `•`(녹색)을 두고 글자를 잇는다 (UX-3 피드백, Notepad++ 방식).
///
/// 선택되지 않은 항목도 같은 폭의 투명한 점을 둬서 글자 시작 위치가 맞는다. 줄 전체를 강조하지 않는다.
pub fn marked_text(ui: &egui::Ui, selected: bool, text: impl Into<RichText>) -> LayoutJob {
    let palette = Palette::from_visuals(ui.visuals());
    let marker_color = if selected {
        palette.menu_selected_marker
    } else {
        Color32::TRANSPARENT
    };
    let mut job = LayoutJob::default();
    RichText::new("•  ").strong().color(marker_color).append_to(
        &mut job,
        ui.style(),
        FontSelection::Default,
        Align::Center,
    );
    text.into()
        .append_to(&mut job, ui.style(), FontSelection::Default, Align::Center);
    job
}

/// 선택 표시 점이 붙은 메뉴 버튼.
pub fn marked_button(
    ui: &egui::Ui,
    selected: bool,
    text: impl Into<RichText>,
) -> egui::Button<'static> {
    egui::Button::new(marked_text(ui, selected, text))
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
    let mut button = marked_button(ui, selected, label);
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
    state: &MenuBarState<'_>,
    triggered: &mut Option<Command>,
) {
    menu_item(ui, ctx, keymap, "찾기…", Command::Find, triggered);
    menu_item(ui, ctx, keymap, "바꾸기…", Command::Replace, triggered);
    ui.separator();
    menu_item(ui, ctx, keymap, "다음 찾기", Command::FindNext, triggered);
    menu_item(ui, ctx, keymap, "이전 찾기", Command::FindPrev, triggered);
    selectable_menu_item(
        ui,
        ctx,
        keymap,
        state.search_wrap_around,
        "끝에서 처음으로 이어서 찾기",
        Command::ToggleSearchWrapAround,
        triggered,
    );

    // 선택 단어 일괄 강조 옵션 (PLAN.md §3.2-1, P2-04)
    ui.separator();
    ui.label(RichText::new("선택 단어 강조").weak());
    selectable_menu_item(
        ui,
        ctx,
        keymap,
        state.highlight_whole_word,
        "단어 단위 일치",
        Command::ToggleHighlightWholeWord,
        triggered,
    );
    selectable_menu_item(
        ui,
        ctx,
        keymap,
        state.highlight_match_case,
        "대소문자 구분",
        Command::ToggleHighlightMatchCase,
        triggered,
    );
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
    ui.label(RichText::new("탭").weak());
    ui.menu_button("탭 너비", |ui| {
        for width in [2, 4, 8] {
            selectable_menu_item(
                ui,
                ctx,
                keymap,
                state.tab_width == width,
                &format!("{width}칸"),
                Command::SetTabWidth(width),
                triggered,
            );
        }
    });
    selectable_menu_item(
        ui,
        ctx,
        keymap,
        state.tab_spaces,
        "Tab 키로 공백 넣기",
        Command::ToggleTabSpaces,
        triggered,
    );

    ui.separator();
    menu_item(ui, ctx, keymap, "글꼴…", Command::ShowFontDialog, triggered);
    menu_item(ui, ctx, keymap, "확대", Command::ZoomIn, triggered);
    menu_item(ui, ctx, keymap, "축소", Command::ZoomOut, triggered);
    menu_item(ui, ctx, keymap, "원래 크기", Command::ZoomReset, triggered);

    ui.separator();
    // `.md`가 아닌 파일도 보기 메뉴에서 미리보기를 쓸 수 있다 (PLAN.md §3.8-1, P3-06).
    ui.label(RichText::new("Markdown 보기").weak());
    ui.scope(|ui| {
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
    encoding_items(ui, state.encoding, triggered);
    ui.separator();
    line_ending_items(ui, state.line_ending, state.mixed_line_endings, triggered);
}

/// 인코딩 항목 (`인코딩` 메뉴와 상태 표시줄 인코딩 팝업이 같이 쓴다 — P1-12).
///
/// - 다시 읽기: 디스크의 파일을 고른 인코딩으로 다시 해석 (현재 인코딩 강조)
/// - 변환: 내용은 그대로 두고 저장할 인코딩만 바꿈
pub fn encoding_items(ui: &mut egui::Ui, current: EncodingChoice, triggered: &mut Option<Command>) {
    ui.label(RichText::new("다시 읽기 — 파일을 이 인코딩으로 다시 해석").weak());
    for enc in EncodingChoice::ALL {
        if ui
            .add(marked_button(ui, current == enc, enc.label()))
            .clicked()
        {
            *triggered = Some(Command::ReopenWithEncoding(enc));
            ui.close();
        }
    }

    ui.separator();
    ui.label(RichText::new("변환 저장 — 내용은 그대로, 저장할 인코딩만 변경").weak());
    for enc in EncodingChoice::ALL {
        // 변환 항목은 선택 상태가 없지만 다시 읽기 항목과 글자 위치를 맞춘다
        if ui
            .add(marked_button(
                ui,
                false,
                format!("{}(으)로 변환", enc.label()),
            ))
            .clicked()
        {
            *triggered = Some(Command::ConvertToEncoding(enc));
            ui.close();
        }
    }
}

/// 줄바꿈 항목 (`인코딩` 메뉴와 상태 표시줄 줄바꿈 팝업이 같이 쓴다 — P1-12).
pub fn line_ending_items(
    ui: &mut egui::Ui,
    current: LineEndingChoice,
    mixed: bool,
    triggered: &mut Option<Command>,
) {
    ui.label(RichText::new("줄바꿈").weak());
    if mixed {
        ui.label(
            RichText::new(
                "이 파일은 줄바꿈이 섞여 있습니다.
저장하면 선택한 방식으로 통일됩니다.",
            )
            .small(),
        );
    }
    for le in LineEndingChoice::ALL {
        // 섞여 있으면 어느 것도 "현재 방식"으로 강조하지 않는다 (고르면 통일).
        if ui
            .add(marked_button(ui, !mixed && current == le, le.menu_label()))
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
        // 탭 바로가기는 최소 10줄까지 스크롤 없이 보이게 한다 (UX-3 피드백).
        // 최대 높이만 주면 메뉴 팝업이 남은 공간에 맞춰 목록을 줄여 버리므로 최소 높이도 같이 고정한다.
        let row_height = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
        let list_height = row_height * WINDOW_MENU_VISIBLE_TABS as f32;
        egui::ScrollArea::vertical()
            .max_height(list_height)
            .min_scrolled_height(list_height)
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
                        marked_button(ui, idx == state.active_tab, text)
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
    for cmd in [
        Command::ShowShortcuts,
        Command::ShowRegexHelp,
        Command::ShowAbout,
    ] {
        if ui.button(cmd.label()).clicked() {
            *triggered = Some(cmd);
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::RawInput;

    /// 좁은 공간(메뉴 팝업처럼)에서 창 메뉴를 그렸을 때 탭 목록 영역의 높이.
    fn tab_list_height(tab_count: usize, available_height: f32) -> (f32, f32) {
        let titles: Vec<String> = (0..tab_count).map(|i| format!("note-{i}.txt")).collect();
        let tabs: Vec<MenuTabEntry<'_>> = titles
            .iter()
            .map(|t| MenuTabEntry {
                title: t,
                path_hint: "",
                modified: false,
            })
            .collect();
        let state = MenuBarState {
            sidebar_open: false,
            word_wrap: true,
            tab_width: 4,
            tab_spaces: false,
            is_markdown: false,
            view_mode: ViewMode::Source,
            scroll_sync: false,
            highlight_whole_word: true,
            highlight_match_case: true,
            search_wrap_around: true,
            theme: ThemeMode::System,
            encoding: EncodingChoice::Utf8,
            line_ending: LineEndingChoice::Lf,
            mixed_line_endings: false,
            recent_files: &[],
            tabs: &tabs,
            active_tab: 0,
            modified_marker_color: Color32::WHITE,
        };
        let keymap = Keymap::for_os(false);
        let ctx = egui::Context::default();
        let mut result = (0.0, 0.0);
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 1200.0),
                )),
                ..Default::default()
            };
            let mut full = ctx.run_ui(raw, |ui| {
                ui.allocate_ui(egui::vec2(400.0, available_height), |ui| {
                    ui.set_max_height(available_height);
                    let top = ui.cursor().top();
                    window_menu(ui, &ui.ctx().clone(), &keymap, &state, &mut None);
                    let row = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                    result = (ui.min_rect().bottom() - top, row);
                });
            });
            full.textures_delta.clear();
        }
        result
    }

    /// 선택된 항목은 왼쪽 녹색 점, 아닌 항목은 같은 폭의 투명한 점 (줄 전체 강조 없음).
    #[test]
    fn marked_text_puts_green_dot_only_on_selected() {
        let ctx = egui::Context::default();
        let mut result = None;
        let mut full = ctx.run_ui(RawInput::default(), |ui| {
            let on = marked_text(ui, true, "UTF-8");
            let off = marked_text(ui, false, "UTF-8");
            result = Some((on, off, Palette::from_visuals(ui.visuals())));
        });
        full.textures_delta.clear();
        let (on, off, palette) = result.unwrap();
        assert_eq!(on.text, off.text);
        assert!(on.text.starts_with('•') && on.text.ends_with("UTF-8"));
        assert_eq!(on.sections[0].format.color, palette.menu_selected_marker);
        assert_eq!(off.sections[0].format.color, Color32::TRANSPARENT);
        // 항목 글자 색은 선택 여부와 같다
        assert_eq!(on.sections[1].format.color, off.sections[1].format.color);
    }

    #[test]
    fn window_menu_shows_ten_tabs_without_scrolling() {
        let (few, row) = tab_list_height(3, 450.0);
        let (many, _) = tab_list_height(15, 450.0);
        // 탭이 많으면 목록 영역이 최소 10줄 높이까지 늘어나야 한다 (3줄일 때보다 7줄 이상 큼).
        assert!(
            many - few >= row * 7.0 - 1.0,
            "3개: {few}, 15개: {many}, 줄 높이: {row}"
        );
    }
}
