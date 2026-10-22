//! 탭 바 UI (ui-spec 탭 바, UX-002 — P0-UX2 프로토타입 이식, P1-10).
//!
//! - 한 줄 유지. 넘치면 양 끝 `‹ ›`(넘칠 때만, 끝에서는 비활성)과 마우스 휠로 가로 스크롤
//! - 활성 탭이 바뀌면(단축키·목록·메뉴 등 어떤 경로든) 화면 안으로 자동 스크롤
//! - 오른쪽 고정 `＋`(새 탭)·`⌄`(열린 탭 목록, 이름 검색), `.md` 탭에서는 Markdown 보기 모드 토글
//! - 탭: 파일 아이콘 · 제목(활성 탭 굵게) · 수정 표시 `●` · 닫기 `×`, 마우스를 올리면 전체 경로,
//!   가운데 클릭으로 닫기, 우클릭 메뉴(닫기 항목들 · 경로 복사)

use eframe::egui::{self, Align, Color32, FontFamily, Layout, RichText, Stroke};

use crate::commands::{Command, Keymap, ViewMode};
use crate::core::file_icons::{FileIcon, file_icon};
use crate::fonts::ICON_FAMILY;
use crate::tabs::Tabs;
use crate::ui::menu_bar;
use crate::ui::theme::Palette;

/// 경로가 없는 새 문서의 툴팁.
pub const UNSAVED_PATH_HINT: &str = "저장되지 않은 새 문서";

/// 읽기 전용 파일 아이콘 (Phosphor `file-lock`).
const READONLY_ICON: FileIcon = FileIcon {
    glyph: "\u{E95C}",
    rgb: [0xC0, 0x6A, 0x00],
};

mod glyph {
    pub const CARET_LEFT: &str = "\u{E138}";
    pub const CARET_RIGHT: &str = "\u{E13A}";
    pub const CARET_DOWN: &str = "\u{E136}";
    pub const PLUS: &str = "\u{E3D4}";
    pub const MAGNIFYING_GLASS: &str = "\u{E30C}";
}

/// 탭 바에서 사용자가 고른 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAction {
    Activate(usize),
    New,
    Close(usize),
    CloseOthers(usize),
    CloseRight(usize),
    CloseSaved,
    CloseAll,
    CopyPath(usize),
    SetViewMode(ViewMode),
}

/// 탭 바 스크롤·목록 검색 상태.
#[derive(Debug, Default)]
pub struct TabBarState {
    offset: f32,
    view_width: f32,
    max_offset: f32,
    scroll_to: Option<f32>,
    /// 마지막으로 화면 안으로 스크롤한 활성 탭 ID.
    scrolled_tab: Option<u64>,
    filter: String,
}

fn glyph_text(glyph: &str) -> RichText {
    RichText::new(glyph).family(FontFamily::Name(ICON_FAMILY.into()))
}

/// 파일 아이콘을 그린다.
pub fn icon_label(ui: &mut egui::Ui, icon: FileIcon) {
    let size = ui.text_style_height(&egui::TextStyle::Body).max(16.0);
    let [r, g, b] = icon.rgb;
    ui.label(
        glyph_text(icon.glyph)
            .size(size)
            .color(Color32::from_rgb(r, g, b)),
    );
}

pub fn tab_icon(tabs: &Tabs, index: usize) -> FileIcon {
    let tab = tabs.get(index).expect("탭 번호 범위");
    if tab.file_readonly || tab.doc.is_readonly() {
        READONLY_ICON
    } else {
        file_icon(&tab.doc.title())
    }
}

fn path_hint(tabs: &Tabs, index: usize) -> String {
    let Some(tab) = tabs.get(index) else {
        return String::new();
    };
    let path = tab
        .doc
        .path()
        .map_or_else(|| UNSAVED_PATH_HINT.to_owned(), |p| p.display().to_string());
    // 읽기 전용 표시 (P1-14: 뷰어·바이너리·읽기 전용 파일)
    if tab.viewer.is_some() {
        format!("{path}\n읽기 전용으로 보는 중 (큰 파일)")
    } else if tab.doc.is_readonly() || tab.file_readonly {
        format!("{path}\n읽기 전용")
    } else {
        path
    }
}

/// 탭 바를 그리고 사용자가 고른 동작을 돌려준다.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut TabBarState,
    tabs: &Tabs,
    keymap: &Keymap,
) -> Option<TabAction> {
    let palette = Palette::from_visuals(ui.visuals());
    let active = tabs.active_index();
    let active_tab = tabs.active();
    let is_md = active_tab.doc.is_markdown();
    let mut action = None;
    let mut active_rect = None;

    let scroll_active = state.scrolled_tab != Some(active_tab.id());
    state.scrolled_tab = Some(active_tab.id());

    let overflow = state.max_offset > 1.0;
    let step = (state.view_width * 0.6).max(120.0);
    let icon_size = ui.text_style_height(&egui::TextStyle::Body);
    let nav = |ui: &mut egui::Ui, g: &str, enabled: bool, tip: &str| {
        ui.add_enabled(
            enabled,
            egui::Button::new(glyph_text(g).size(icon_size)).frame(false),
        )
        .on_hover_text(tip)
        .clicked()
    };

    ui.horizontal(|ui| {
        // 오른쪽 고정 영역: ›, ＋, ⌄, (Markdown 모드 토글)
        let right_width = 34.0 * if overflow { 3.0 } else { 2.0 } + if is_md { 230.0 } else { 0.0 };

        if overflow
            && nav(
                ui,
                glyph::CARET_LEFT,
                state.offset > 0.5,
                "왼쪽 탭으로 이동",
            )
        {
            state.scroll_to = Some((state.offset - step).max(0.0));
        }

        let mut area = egui::ScrollArea::horizontal()
            .id_salt("tab-bar-scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .max_width((ui.available_width() - right_width).max(100.0))
            .auto_shrink([false, true]);
        if let Some(offset) = state.scroll_to.take() {
            area = area.horizontal_scroll_offset(offset);
        }
        let output = ui
            .scope(|ui| {
                // 세로 휠도 가로 스크롤로 쓰게 한다 (Shift 없이).
                ui.style_mut().always_scroll_the_only_direction = true;
                area.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for i in 0..tabs.len() {
                            let selected = i == active;
                            let rect = tab_widget(ui, tabs, i, selected, &palette, &mut action);
                            if selected {
                                active_rect = Some(rect);
                                if scroll_active {
                                    ui.scroll_to_rect(rect, None);
                                }
                            }
                        }
                    });
                })
            })
            .inner;
        // 첫 프레임에는 스크롤 범위가 확정되지 않아 덜 스크롤될 수 있으므로, 활성 탭이 완전히 보일 때까지 다음 프레임에 다시 맞춘다.
        if let Some(rect) = active_rect
            && scroll_active
            && !output.inner_rect.expand(1.0).contains_rect(rect)
            && rect.width() <= output.inner_rect.width()
        {
            state.scrolled_tab = None;
            ui.ctx().request_repaint();
        }
        state.offset = output.state.clamped_offset().x;
        state.view_width = output.inner_rect.width();
        state.max_offset = (output.content_size.x - output.inner_rect.width()).max(0.0);

        if overflow
            && nav(
                ui,
                glyph::CARET_RIGHT,
                state.offset < state.max_offset - 0.5,
                "오른쪽 탭으로 이동",
            )
        {
            state.scroll_to = Some((state.offset + step).min(state.max_offset));
        }
        let new_tip = match keymap.format_shortcut_for(Command::NewTab, ui.ctx()) {
            Some(s) => format!("새 탭 ({s})"),
            None => "새 탭".to_owned(),
        };
        if nav(ui, glyph::PLUS, true, &new_tip) {
            action = Some(TabAction::New);
        }
        let list = ui.menu_button(glyph_text(glyph::CARET_DOWN).size(icon_size), |ui| {
            if let Some(i) = open_tabs_list(ui, state, tabs, &palette) {
                action = Some(TabAction::Activate(i));
                ui.close();
            }
        });
        list.response
            .on_hover_text(format!("열린 탭 목록 ({}개)", tabs.len()));

        if is_md {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ctx = ui.ctx().clone();
                let toggle = keymap
                    .format_shortcut_for(Command::ToggleMdPreview, &ctx)
                    .unwrap_or_default();
                let split = keymap
                    .format_shortcut_for(Command::ToggleMdSplit, &ctx)
                    .unwrap_or_default();
                let tips = [
                    (ViewMode::Preview, format!("미리보기 ({toggle})")),
                    (ViewMode::Split, format!("소스 + 미리보기 분할 ({split})")),
                    (ViewMode::Source, format!("소스 ({toggle})")),
                ];
                for (mode, tip) in tips {
                    if ui
                        .add(egui::Button::selectable(
                            active_tab.view_mode == mode,
                            mode.label(),
                        ))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        action = Some(TabAction::SetViewMode(mode));
                    }
                }
                ui.label(RichText::new("Markdown").weak().small());
            });
        }
    });

    action
}

/// 탭 하나를 그리고 화면 영역을 돌려준다.
fn tab_widget(
    ui: &mut egui::Ui,
    tabs: &Tabs,
    index: usize,
    selected: bool,
    palette: &Palette,
    action: &mut Option<TabAction>,
) -> egui::Rect {
    let tab = tabs.get(index).expect("탭 번호 범위");
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
        icon_label(ui, tab_icon(tabs, index));
        let title = RichText::new(tab.doc.title());
        let label = ui
            .add(
                egui::Label::new(if selected { title.strong() } else { title })
                    .selectable(false)
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(path_hint(tabs, index));
        label.context_menu(|ui| {
            if let Some(a) = context_menu(ui, index, tab.doc.path().is_some()) {
                *action = Some(a);
            }
        });
        if label.clicked() {
            *action = Some(TabAction::Activate(index));
        }
        if label.middle_clicked() {
            *action = Some(TabAction::Close(index));
        }
        if tab.doc.is_modified() {
            ui.label(RichText::new("●").color(palette.modified_marker))
                .on_hover_text("저장되지 않은 변경 내용");
        }
        if ui
            .add(egui::Button::new("×").small().frame(false))
            .on_hover_text("탭 닫기")
            .clicked()
        {
            *action = Some(TabAction::Close(index));
        }
    });
    shown.response.rect
}

/// 탭 우클릭 메뉴.
fn context_menu(ui: &mut egui::Ui, index: usize, has_path: bool) -> Option<TabAction> {
    let mut picked = None;
    let mut item = |ui: &mut egui::Ui, label: &str, a: TabAction| {
        if ui.button(label).clicked() {
            picked = Some(a);
            ui.close();
        }
    };
    item(ui, "닫기", TabAction::Close(index));
    item(ui, "다른 탭 모두 닫기", TabAction::CloseOthers(index));
    item(ui, "오른쪽 탭 모두 닫기", TabAction::CloseRight(index));
    item(ui, "저장된 탭 닫기", TabAction::CloseSaved);
    item(ui, "모든 탭 닫기", TabAction::CloseAll);
    if has_path {
        ui.separator();
        item(ui, "경로 복사", TabAction::CopyPath(index));
    }
    picked
}

/// 열린 탭 목록 드롭다운: 검색어로 거르고, 고른 탭의 번호를 돌려준다.
fn open_tabs_list(
    ui: &mut egui::Ui,
    state: &mut TabBarState,
    tabs: &Tabs,
    palette: &Palette,
) -> Option<usize> {
    ui.set_min_width(300.0);
    ui.horizontal(|ui| {
        ui.label(glyph_text(glyph::MAGNIFYING_GLASS).weak());
        let filter = ui.add(
            egui::TextEdit::singleline(&mut state.filter)
                .hint_text("탭 이름으로 찾기")
                .desired_width(240.0),
        );
        filter.request_focus();
    });
    ui.separator();
    let needle = state.filter.to_lowercase();
    let mut picked = None;
    let mut shown = 0;
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for (i, tab) in tabs.iter().enumerate() {
                let title = tab.doc.title();
                if !needle.is_empty() && !title.to_lowercase().contains(&needle) {
                    continue;
                }
                shown += 1;
                let row = ui
                    .horizontal(|ui| {
                        icon_label(ui, tab_icon(tabs, i));
                        let selected = i == tabs.active_index();
                        let mut text = RichText::new(&title);
                        if selected {
                            text = text.strong();
                        }
                        let r = ui.add(menu_bar::marked_button(ui, selected, text));
                        if tab.doc.is_modified() {
                            ui.label(RichText::new("●").color(palette.modified_marker));
                        }
                        r
                    })
                    .inner;
                if row.on_hover_text(path_hint(tabs, i)).clicked() {
                    picked = Some(i);
                }
            }
        });
    if shown == 0 {
        ui.label(RichText::new("일치하는 탭이 없습니다").weak());
    }
    if picked.is_some() {
        state.filter.clear();
    }
    picked
}
