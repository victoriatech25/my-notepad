//! 문서 본문 컨테이너: 소스 / 미리보기 / 분할 보기 모드 (PLAN.md §3.8-1, P3-06).
//!
//! - 탭의 [`ViewMode`]에 따라 에디터·미리보기를 배치한다. 분할은 왼쪽 소스, 오른쪽 미리보기이며
//!   분할선을 끌어 비율을 바꾼다(더블클릭하면 50:50). 비율은 앱 설정에 저장한다.
//! - 본문 폭이 [`NARROW_WIDTH`]보다 좁으면 분할 대신 한쪽만 보여 준다. 탭 모드는 분할 그대로 두므로
//!   다시 넓히면 분할로 돌아온다. 좁은 동안 소스 ↔ 미리보기 전환은 [`DocViewState::toggle_narrow_pane`].
//! - 실제로 보이는 화면이 바뀌면 직전 화면 맨 위의 소스 줄을 새로 보이는 쪽의 맨 위로 맞춘다
//!   (전환할 때 문서 맨 위로 튀지 않게). 새 쪽은 그 폭의 레이아웃으로 위치를 계산하느라 한 프레임 숨겨 그린다.

use eframe::egui::{self, CursorIcon, Rect, Sense, Stroke, UiBuilder, pos2, vec2};

use crate::commands::ViewMode;
use crate::tabs::Tab;
use crate::ui::editor::EditorOptions;
use crate::ui::theme::Palette;

/// 이보다 좁으면 분할 대신 한쪽만 보여 준다 (논리 px).
pub const NARROW_WIDTH: f32 = 900.0;
/// 분할 비율 기본값 (소스 쪽 폭 비율).
pub const DEFAULT_SPLIT_RATIO: f32 = 0.5;
/// 분할 비율 범위.
pub const SPLIT_RATIO_RANGE: std::ops::RangeInclusive<f32> = 0.2..=0.8;
/// 분할선을 잡을 수 있는 폭 (px).
const SPLITTER_GRAB: f32 = 8.0;

/// 탭별 보기 상태.
#[derive(Debug, Default)]
pub struct DocViewState {
    /// 지난 프레임에 실제로 보인 화면 (좁은 창 대체 반영)
    shown: Option<ViewMode>,
    /// 지난 프레임에 본문 폭이 좁았는지
    narrow: bool,
    /// 좁은 창에서 분할 대신 미리보기를 보여 줄지 (기본은 소스)
    narrow_preview: bool,
}

impl DocViewState {
    /// 탭 모드가 분할인데 창이 좁아 한쪽만 보이는 중인지.
    pub fn is_narrow_split(&self, mode: ViewMode) -> bool {
        self.narrow && mode == ViewMode::Split
    }

    /// 좁은 창의 분할 대체 화면을 소스 ↔ 미리보기로 바꾼다.
    pub fn toggle_narrow_pane(&mut self) {
        self.narrow_preview = !self.narrow_preview;
    }

    /// 지난 프레임에 실제로 보인 화면.
    pub fn shown(&self) -> Option<ViewMode> {
        self.shown
    }

    fn effective(&self, mode: ViewMode) -> ViewMode {
        match mode {
            ViewMode::Split if self.narrow && self.narrow_preview => ViewMode::Preview,
            ViewMode::Split if self.narrow => ViewMode::Source,
            other => other,
        }
    }
}

fn shows_source(mode: ViewMode) -> bool {
    mode != ViewMode::Preview
}

fn shows_preview(mode: ViewMode) -> bool {
    mode != ViewMode::Source
}

/// 탭 본문을 그린다. 남은 영역을 모두 차지한다. 선택 단어 강조가 상한을 넘었으면 `true`.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    split_ratio: &mut f32,
    options: &EditorOptions,
    zoom: f32,
    palette: &Palette,
) -> bool {
    let full = ui.available_rect_before_wrap();
    tab.doc_view.narrow = full.width() < NARROW_WIDTH;
    let shown = tab.doc_view.effective(tab.view_mode);
    if let Some(prev) = tab.doc_view.shown
        && prev != shown
    {
        let anchor = if shows_source(prev) {
            tab.editor.top_line(&tab.doc)
        } else {
            tab.preview.top_line()
        };
        if let Some(line) = anchor {
            if shows_source(shown) && !shows_source(prev) {
                tab.editor.scroll_to_line(line);
            }
            if shows_preview(shown) && !shows_preview(prev) {
                tab.preview.scroll_to_line(line);
            }
        }
    }
    tab.doc_view.shown = Some(shown);

    let mut exceeded = false;
    match shown {
        ViewMode::Source => exceeded = source_in(ui, full, tab, options, palette),
        ViewMode::Preview => preview_in(ui, full, tab, zoom, palette),
        ViewMode::Split => {
            let x = split_x(full, *split_ratio);
            let left = Rect::from_min_max(full.min, pos2(x, full.bottom()));
            let right = Rect::from_min_max(pos2(x + 1.0, full.top()), full.max);
            exceeded = source_in(ui, left, tab, options, palette);
            preview_in(ui, right, tab, zoom, palette);
            splitter(ui, full, x, split_ratio, palette);
        }
    }
    ui.allocate_rect(full, Sense::hover());
    exceeded
}

/// 분할선 x (정수 픽셀).
fn split_x(full: Rect, ratio: f32) -> f32 {
    let ratio = ratio.clamp(*SPLIT_RATIO_RANGE.start(), *SPLIT_RATIO_RANGE.end());
    (full.left() + full.width() * ratio).round()
}

fn source_in(
    ui: &mut egui::Ui,
    rect: Rect,
    tab: &mut Tab,
    options: &EditorOptions,
    palette: &Palette,
) -> bool {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    tab.editor
        .show(&mut child, &mut tab.doc, options, palette)
        .highlight_exceeded
}

fn preview_in(ui: &mut egui::Ui, rect: Rect, tab: &mut Tab, zoom: f32, palette: &Palette) {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    tab.preview.show(&mut child, &tab.doc, zoom, palette);
}

/// 분할선: 끌면 비율을 바꾸고, 더블클릭하면 기본 비율로 되돌린다.
fn splitter(ui: &mut egui::Ui, full: Rect, x: f32, ratio: &mut f32, palette: &Palette) {
    let grab = Rect::from_center_size(pos2(x, full.center().y), vec2(SPLITTER_GRAB, full.height()));
    let response = ui.interact(
        grab,
        egui::Id::unique("md-splitter"),
        Sense::click_and_drag(),
    );
    let active = response.hovered() || response.dragged();
    if active {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
        && full.width() > 0.0
    {
        *ratio = ((pointer.x - full.left()) / full.width())
            .clamp(*SPLIT_RATIO_RANGE.start(), *SPLIT_RATIO_RANGE.end());
    }
    if response.double_clicked() {
        *ratio = DEFAULT_SPLIT_RATIO;
    }
    let stroke = if active {
        Stroke::new(2.0, palette.modified_marker)
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    ui.painter().vline(x, full.y_range(), stroke);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::highlight::HighlightOptions;
    use crate::tabs::Tabs;
    use eframe::egui::{Event, FontId, PointerButton, Pos2, RawInput};

    fn options() -> EditorOptions {
        EditorOptions {
            word_wrap: true,
            font: FontId::monospace(13.0),
            highlight: HighlightOptions::default(),
            tab_width: 4,
            tab_spaces: false,
        }
    }

    /// 헤드리스 프레임: `width`×800 창 전체에 활성 탭 본문을 그린다.
    fn frame(
        ctx: &egui::Context,
        tabs: &mut Tabs,
        ratio: &mut f32,
        width: f32,
        events: Vec<Event>,
    ) {
        let raw = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 800.0))),
            ..Default::default()
        };
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    show(
                        ui,
                        tabs.active_mut(),
                        ratio,
                        &options(),
                        1.0,
                        &Palette::light(),
                    );
                });
        });
        full.textures_delta.clear();
    }

    fn frames(ctx: &egui::Context, tabs: &mut Tabs, ratio: &mut f32, width: f32) {
        for _ in 0..3 {
            frame(ctx, tabs, ratio, width, Vec::new());
        }
    }

    fn markdown_tabs(paragraphs: usize) -> Tabs {
        let mut tabs = Tabs::new();
        let text: String = (0..paragraphs).map(|i| format!("문단 {i}\n\n")).collect();
        tabs.active_mut().doc.set_text(text);
        tabs
    }

    fn near(actual: Option<f32>, expected: f32) -> bool {
        actual.is_some_and(|a| (a - expected).abs() <= 2.0)
    }

    #[test]
    fn switching_modes_keeps_the_top_line() {
        let ctx = egui::Context::default();
        let mut tabs = markdown_tabs(300);
        let mut ratio = DEFAULT_SPLIT_RATIO;
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        tabs.active_mut().editor.scroll_to_line(300.0);
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        let tab = tabs.active();
        assert!(near(tab.editor.top_line(&tab.doc), 300.0));

        tabs.active_mut().view_mode = ViewMode::Preview;
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        let top = tabs.active().preview.top_line();
        assert!(near(top, 300.0), "미리보기 맨 위 줄 {top:?}");

        tabs.active_mut().preview.scroll_to_line(150.0);
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        tabs.active_mut().view_mode = ViewMode::Source;
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        let tab = tabs.active();
        let top = tab.editor.top_line(&tab.doc);
        assert!(near(top, 150.0), "소스 맨 위 줄 {top:?}");

        // 소스 → 분할: 소스는 그대로, 미리보기가 같은 줄로
        tabs.active_mut().view_mode = ViewMode::Split;
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        let tab = tabs.active();
        assert!(near(tab.editor.top_line(&tab.doc), 150.0));
        assert!(near(tab.preview.top_line(), 150.0));
    }

    #[test]
    fn narrow_window_replaces_split_and_restores_it() {
        let ctx = egui::Context::default();
        let mut tabs = markdown_tabs(10);
        let mut ratio = DEFAULT_SPLIT_RATIO;
        tabs.active_mut().view_mode = ViewMode::Split;
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        assert_eq!(tabs.active().doc_view.shown(), Some(ViewMode::Split));

        frames(&ctx, &mut tabs, &mut ratio, 700.0);
        let tab = tabs.active();
        assert_eq!(tab.doc_view.shown(), Some(ViewMode::Source));
        assert!(tab.doc_view.is_narrow_split(tab.view_mode));

        tabs.active_mut().doc_view.toggle_narrow_pane();
        frames(&ctx, &mut tabs, &mut ratio, 700.0);
        assert_eq!(tabs.active().doc_view.shown(), Some(ViewMode::Preview));
        assert_eq!(tabs.active().view_mode, ViewMode::Split);

        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        assert_eq!(tabs.active().doc_view.shown(), Some(ViewMode::Split));
    }

    #[test]
    fn dragging_splitter_changes_ratio_within_range() {
        let ctx = egui::Context::default();
        let mut tabs = markdown_tabs(10);
        let mut ratio = DEFAULT_SPLIT_RATIO;
        tabs.active_mut().view_mode = ViewMode::Split;
        frames(&ctx, &mut tabs, &mut ratio, 1000.0);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let start = pos2(500.0, 400.0);
        frame(
            &ctx,
            &mut tabs,
            &mut ratio,
            1000.0,
            vec![Event::PointerMoved(start)],
        );
        frame(
            &ctx,
            &mut tabs,
            &mut ratio,
            1000.0,
            vec![button(start, true)],
        );
        for x in [550.0, 650.0, 700.0] {
            frame(
                &ctx,
                &mut tabs,
                &mut ratio,
                1000.0,
                vec![Event::PointerMoved(pos2(x, 400.0))],
            );
        }
        frame(
            &ctx,
            &mut tabs,
            &mut ratio,
            1000.0,
            vec![button(pos2(700.0, 400.0), false)],
        );
        assert!((ratio - 0.7).abs() < 0.01, "비율 {ratio}");

        // 범위 밖으로 끌어도 0.8에서 멈춘다
        let start = pos2(700.0, 400.0);
        frame(
            &ctx,
            &mut tabs,
            &mut ratio,
            1000.0,
            vec![button(start, true)],
        );
        for x in [800.0, 990.0] {
            frame(
                &ctx,
                &mut tabs,
                &mut ratio,
                1000.0,
                vec![Event::PointerMoved(pos2(x, 400.0))],
            );
        }
        frame(
            &ctx,
            &mut tabs,
            &mut ratio,
            1000.0,
            vec![button(pos2(990.0, 400.0), false)],
        );
        assert_eq!(ratio, 0.8);
    }

    /// 모드 전환 소요 시간 — `cargo test --release --lib -- --ignored --nocapture mode_switch_time`
    #[test]
    #[ignore = "측정용: 릴리스 빌드로 실행"]
    fn mode_switch_time_100kb() {
        let unit = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/markdown/gfm_all.md"),
        )
        .unwrap();
        let mut text = String::new();
        while text.len() < 100 * 1024 {
            text.push_str(&unit);
            text.push('\n');
        }
        let ctx = egui::Context::default();
        let mut tabs = Tabs::new();
        tabs.active_mut().doc.set_text(&text);
        let mut ratio = DEFAULT_SPLIT_RATIO;
        frames(&ctx, &mut tabs, &mut ratio, 1400.0);
        tabs.active_mut().editor.scroll_to_line(1000.0);
        frames(&ctx, &mut tabs, &mut ratio, 1400.0);
        let mut report = Vec::new();
        for mode in [
            ViewMode::Preview,
            ViewMode::Source,
            ViewMode::Split,
            ViewMode::Preview,
            ViewMode::Split,
            ViewMode::Source,
        ] {
            tabs.active_mut().view_mode = mode;
            // 숨겨 그리는 프레임 + 위치를 맞춘 프레임
            let start = std::time::Instant::now();
            frame(&ctx, &mut tabs, &mut ratio, 1400.0, Vec::new());
            frame(&ctx, &mut tabs, &mut ratio, 1400.0, Vec::new());
            report.push(format!(
                "{} {:.1}ms",
                mode.label(),
                start.elapsed().as_secs_f64() * 1000.0
            ));
            frames(&ctx, &mut tabs, &mut ratio, 1400.0);
        }
        println!(
            "100KB 모드 전환 (2프레임, {} B): {}",
            text.len(),
            report.join(" · ")
        );
    }
}
