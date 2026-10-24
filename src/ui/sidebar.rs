//! 좌측 폴더 탐색 사이드바 (PLAN.md §3.1-2·§3.1-3, ui-spec 사이드바, P2-06 — P0-UX1 프로토타입 이식).
//!
//! - 머리글: `탐색기` · `폴더 열기` · `⟳`(새로고침) · `«`(닫기). 닫힌 상태의 다시 열기 띠(`»`)는 [`rail`]
//! - 트리는 [`FsTree`](지연 로딩·숨김·`.gitignore`)를 그린다. 줄 높이를 고정하고 **화면에 보이는 줄만** 그려
//!   파일이 아주 많은 폴더도 프레임 비용이 보이는 줄 수에 비례한다
//! - 한 줄: 들여쓰기 · 펼침 화살표 · 아이콘(폴더 열림/닫힘, 파일 종류) · 이름. `.gitignore` 대상은 흐리게,
//!   현재 탭의 파일은 선택 색. 폴더는 한 번 클릭으로 펼침/접힘, 파일은 한 번 클릭으로 탭에서 연다 (UX-1 결정)

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Align2, Color32, FontFamily, FontId, Layout, RichText, Sense};

use crate::core::file_icons::{FileIcon, file_icon, folder_icon};
use crate::core::fs_tree::{DEFAULT_HIDDEN, FsTree, NodeId};
use crate::fonts::ICON_FAMILY;
use crate::ui::theme::Palette;

/// 닫힌 사이드바 띠 폭 (ui-spec: 30px).
pub const RAIL_WIDTH: f32 = 30.0;
/// 한 단계 들여쓰기 폭.
const INDENT: f32 = 14.0;

mod glyph {
    pub const CARET_RIGHT: &str = "\u{E13A}";
    pub const CARET_DOWN: &str = "\u{E136}";
}

/// 사이드바에서 사용자가 고른 동작 (앱이 처리).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarAction {
    /// 파일을 탭으로 연다 (이미 열려 있으면 그 탭으로 이동)
    OpenFile(PathBuf),
    /// 폴더 선택 대화상자
    OpenFolderDialog,
    /// 사이드바 닫기
    Close,
}

/// 사이드바 상태 (열린 폴더 트리).
#[derive(Debug, Default)]
pub struct SidebarState {
    tree: Option<FsTree>,
}

impl SidebarState {
    /// 폴더를 연다 (기본 숨김 목록 사용).
    pub fn open_folder(&mut self, path: &Path) -> std::io::Result<()> {
        self.tree = Some(FsTree::open(path, DEFAULT_HIDDEN)?);
        Ok(())
    }

    /// 열린 폴더 경로.
    pub fn folder(&self) -> Option<&Path> {
        self.tree.as_ref().map(FsTree::root_path)
    }

    pub fn tree(&self) -> Option<&FsTree> {
        self.tree.as_ref()
    }
}

/// 사이드바 패널 내용을 그린다. `current`는 활성 탭 파일 경로 (트리에서 선택 표시).
pub fn show(
    ui: &mut egui::Ui,
    state: &mut SidebarState,
    palette: &Palette,
    toggle_shortcut: &str,
    current: Option<&Path>,
) -> Option<SidebarAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.strong("탐색기");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(egui::Button::new("«").frame(false))
                .on_hover_text(format!("사이드바 닫기 ({toggle_shortcut})"))
                .clicked()
            {
                action = Some(SidebarAction::Close);
            }
            if let Some(tree) = &mut state.tree
                && ui
                    .small_button("⟳")
                    .on_hover_text("새로고침 — 다른 프로그램에서 바꾼 파일·폴더를 다시 읽습니다")
                    .clicked()
                && let Err(e) = tree.refresh()
            {
                log::warn!("사이드바 새로고침 실패: {e}");
            }
            if ui
                .small_button("폴더 열기")
                .on_hover_text("다른 폴더 열기")
                .clicked()
            {
                action = Some(SidebarAction::OpenFolderDialog);
            }
        });
    });

    let Some(tree) = &mut state.tree else {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("열린 폴더가 없습니다").weak());
            ui.add_space(6.0);
            if ui.button("폴더 열기…").clicked() {
                action = Some(SidebarAction::OpenFolderDialog);
            }
        });
        return action;
    };

    let root = tree.node(tree.root());
    ui.label(RichText::new(&root.name).strong())
        .on_hover_text(root.path.display().to_string());
    ui.label(
        RichText::new("숨김: .git, node_modules · 흐림: .gitignore 대상")
            .small()
            .weak(),
    );
    ui.separator();

    let rows = tree.visible_rows();
    let row_height = (ui.text_style_height(&egui::TextStyle::Body) + 6.0).max(22.0);
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for &id in &rows[range] {
                if tree_row(ui, tree, id, row_height, palette, current).clicked() {
                    clicked = Some(id);
                }
            }
        });

    if let Some(id) = clicked {
        let node = tree.node(id);
        if node.is_dir() {
            tree.toggle(id);
        } else {
            action = Some(SidebarAction::OpenFile(node.path.clone()));
        }
    }
    action
}

/// 트리 한 줄을 그리고 응답을 돌려준다 (줄 높이 고정).
fn tree_row(
    ui: &mut egui::Ui,
    tree: &FsTree,
    id: NodeId,
    row_height: f32,
    palette: &Palette,
    current: Option<&Path>,
) -> egui::Response {
    let node = tree.node(id);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), row_height), Sense::click());
    let selected = !node.is_dir() && current == Some(node.path.as_path());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 2.0, palette.selection_bg);
    } else if response.hovered() {
        painter.rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }

    let visuals = ui.visuals();
    let text_color = if node.ignored {
        visuals.weak_text_color()
    } else if selected {
        palette.selection_stroke
    } else {
        visuals.text_color()
    };
    let font_size = ui.text_style_height(&egui::TextStyle::Body);
    let icon_font = FontId::new(font_size.max(16.0), FontFamily::Name(ICON_FAMILY.into()));
    let mid = rect.center().y;
    let mut x = rect.min.x + 4.0 + node.depth as f32 * INDENT;

    if node.is_dir() {
        painter.text(
            egui::pos2(x + 6.0, mid),
            Align2::CENTER_CENTER,
            if node.expanded {
                glyph::CARET_DOWN
            } else {
                glyph::CARET_RIGHT
            },
            FontId::new(font_size * 0.8, FontFamily::Name(ICON_FAMILY.into())),
            visuals.weak_text_color(),
        );
    }
    x += 14.0;

    let icon: FileIcon = if node.is_dir() {
        folder_icon(&node.name, node.expanded)
    } else {
        file_icon(&node.name)
    };
    let [r, g, b] = icon.rgb;
    let mut icon_color = Color32::from_rgb(r, g, b);
    if node.ignored {
        icon_color = icon_color.gamma_multiply(0.45);
    }
    painter.text(
        egui::pos2(x, mid),
        Align2::LEFT_CENTER,
        icon.glyph,
        icon_font,
        icon_color,
    );
    x += font_size.max(16.0) + 6.0;

    let mut name = node.name.clone();
    if node.error.is_some() {
        name.push_str("  ⚠");
    }
    painter.text(
        egui::pos2(x, mid),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(font_size),
        text_color,
    );

    let mut tip = node.path.display().to_string();
    if node.symlink {
        tip.push_str("\n링크");
    }
    if node.ignored {
        tip.push_str("\n.gitignore 대상");
    }
    if let Some(error) = &node.error {
        tip.push('\n');
        tip.push_str(error);
    }
    response.on_hover_text(tip)
}

/// 닫힌 사이드바의 좁은 세로 띠. 띠 어디를 눌러도 다시 열린다 (UX-001). 열기를 눌렀으면 `true`.
pub fn rail(ui: &mut egui::Ui, toggle_shortcut: &str) -> bool {
    let tip = format!("사이드바 열기 ({toggle_shortcut})");
    let mut open = false;
    ui.vertical_centered(|ui| {
        ui.add_space(4.0);
        if ui
            .add(egui::Button::new("»").frame(false))
            .on_hover_text(&tip)
            .clicked()
        {
            open = true;
        }
        let rest = ui.allocate_response(ui.available_size(), Sense::click());
        if rest.on_hover_text(&tip).clicked() {
            open = true;
        }
    });
    open
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, Pos2, RawInput, Rect};
    use std::fs;

    /// 헤드리스 프레임: 사이드바를 300×500 왼쪽 패널에 그리고 (동작, 그려진 글자들)을 돌려준다.
    fn frame(
        ctx: &egui::Context,
        state: &mut SidebarState,
        events: Vec<Event>,
        current: Option<&Path>,
    ) -> (Option<SidebarAction>, Vec<(String, Pos2)>) {
        let raw = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 500.0))),
            ..Default::default()
        };
        let mut action = None;
        let mut full = ctx.run_ui(raw, |ui| {
            egui::Panel::left("sidebar")
                .exact_size(300.0)
                .show(ui, |ui| {
                    action = show(ui, state, &Palette::light(), "Ctrl+B", current);
                });
        });
        full.textures_delta.clear();
        let texts = full
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos)),
                _ => None,
            })
            .collect();
        (action, texts)
    }

    fn click_at(pos: Pos2) -> Vec<Event> {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![Event::PointerMoved(pos), button(true), button(false)]
    }

    fn row_pos(texts: &[(String, Pos2)], name: &str) -> Pos2 {
        texts
            .iter()
            .find(|(t, _)| t == name)
            .unwrap_or_else(|| panic!("{name} 줄이 없음"))
            .1
            + egui::vec2(4.0, 6.0)
    }

    #[test]
    fn clicking_folder_toggles_and_file_opens() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), b"").unwrap();
        fs::write(dir.path().join("README.md"), b"").unwrap();
        let ctx = egui::Context::default();
        let mut state = SidebarState::default();
        state.open_folder(dir.path()).unwrap();

        let (_, texts) = frame(&ctx, &mut state, vec![], None);
        assert!(
            !texts.iter().any(|(t, _)| t == "main.rs"),
            "접힌 폴더 안은 안 보임"
        );
        let src = row_pos(&texts, "src");
        frame(&ctx, &mut state, click_at(src), None);
        let (_, texts) = frame(&ctx, &mut state, vec![], None);
        assert!(texts.iter().any(|(t, _)| t == "main.rs"), "클릭으로 펼침");

        let readme = row_pos(&texts, "README.md");
        let (action, _) = frame(&ctx, &mut state, click_at(readme), None);
        assert_eq!(
            action,
            Some(SidebarAction::OpenFile(dir.path().join("README.md")))
        );
    }

    #[test]
    fn draws_only_visible_rows_for_huge_folders() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10_000 {
            fs::write(dir.path().join(format!("f{i:05}.txt")), b"").unwrap();
        }
        let ctx = egui::Context::default();
        let mut state = SidebarState::default();
        state.open_folder(dir.path()).unwrap();
        frame(&ctx, &mut state, vec![], None);
        let (_, texts) = frame(&ctx, &mut state, vec![], None);
        let names = texts.iter().filter(|(t, _)| t.starts_with('f')).count();
        assert!(names > 5 && names < 60, "보이는 줄만 그림 ({names})");
    }

    #[test]
    fn empty_state_offers_open_folder_and_close_works() {
        let ctx = egui::Context::default();
        let mut state = SidebarState::default();
        let (_, texts) = frame(&ctx, &mut state, vec![], None);
        assert!(texts.iter().any(|(t, _)| t == "열린 폴더가 없습니다"));
        let button = row_pos(&texts, "폴더 열기…");
        let (action, _) = frame(&ctx, &mut state, click_at(button), None);
        assert_eq!(action, Some(SidebarAction::OpenFolderDialog));
        let close = row_pos(&texts, "«");
        let (action, _) = frame(&ctx, &mut state, click_at(close), None);
        assert_eq!(action, Some(SidebarAction::Close));
    }

    #[test]
    fn dimmed_and_current_rows_use_their_colors() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
        fs::write(dir.path().join("app.log"), b"").unwrap();
        fs::write(dir.path().join("main.rs"), b"").unwrap();
        let ctx = egui::Context::default();
        let mut state = SidebarState::default();
        state.open_folder(dir.path()).unwrap();
        let current = dir.path().join("main.rs");
        frame(&ctx, &mut state, vec![], Some(&current));
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 500.0))),
            ..Default::default()
        };
        let mut full = ctx.run_ui(raw, |ui| {
            egui::Panel::left("sidebar")
                .exact_size(300.0)
                .show(ui, |ui| {
                    show(ui, &mut state, &Palette::light(), "Ctrl+B", Some(&current));
                });
        });
        full.textures_delta.clear();
        let color_of = |name: &str| {
            full.shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Text(t) if t.galley.text() == name => Some(t.fallback_color),
                    _ => None,
                })
                .unwrap()
        };
        let weak = ctx.global_style().visuals.weak_text_color();
        assert_eq!(color_of("app.log"), weak, ".gitignore 대상은 흐리게");
        assert_eq!(
            color_of("main.rs"),
            Palette::light().selection_stroke,
            "현재 탭 파일"
        );
        // 현재 탭 줄 배경
        assert!(full.shapes.iter().any(|c| matches!(&c.shape,
            egui::Shape::Rect(r) if r.fill == Palette::light().selection_bg)));
    }
}
