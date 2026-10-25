//! 문서 본문 컨테이너: 소스 / 미리보기 / 분할 보기 모드 (PLAN.md §3.8-1, P3-06).
//!
//! - 탭의 [`ViewMode`]에 따라 에디터·미리보기를 배치한다. 분할은 왼쪽 소스, 오른쪽 미리보기이며
//!   분할선을 끌어 비율을 바꾼다(더블클릭하면 50:50). 비율은 앱 설정에 저장한다.
//! - 본문 폭이 [`NARROW_WIDTH`]보다 좁으면 분할 대신 한쪽만 보여 준다. 탭 모드는 분할 그대로 두므로
//!   다시 넓히면 분할로 돌아온다. 좁은 동안 소스 ↔ 미리보기 전환은 [`DocViewState::toggle_narrow_pane`].
//! - 실제로 보이는 화면이 바뀌면 직전 화면 맨 위의 소스 줄을 새로 보이는 쪽의 맨 위로 맞춘다
//!   (전환할 때 문서 맨 위로 튀지 않게). 새 쪽은 그 폭의 레이아웃으로 위치를 계산하느라 한 프레임 숨겨 그린다.
//! - 분할 모드 양방향 스크롤 동기화(P3-08): 이번 프레임에 사용자가 움직인 쪽(둘 다면 마우스가 올라간 쪽)의
//!   화면 맨 위 소스 줄을 반대쪽 맨 위로 맞춘다. 동기화로 적용한 스크롤 변화는 사용자 조작으로 보지 않으므로
//!   반대 방향으로 되먹임되지 않는다. 대응은 블록 줄 범위 ↔ y([`crate::ui::markdown_view::line_to_y`]).
//! - 분할 모드에서는 소스 커서 줄을 포함한 미리보기 블록에 옅은 배경을 깔고, 미리보기 블록을 더블클릭하면
//!   소스의 그 줄로 커서를 옮긴다 (미리보기 단독이면 분할로 바꾼다).

use eframe::egui::{self, CursorIcon, Rect, Sense, Stroke, UiBuilder, pos2, vec2};

use crate::commands::ViewMode;
use crate::tabs::Tab;
use crate::ui::editor::{self, EditorOptions};
use crate::ui::markdown_view::{LinkAction, PreviewOutput};
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
    /// 지난 프레임의 소스·미리보기 세로 스크롤 위치 (사용자가 움직인 쪽 판정)
    source_offset: f32,
    preview_offset: f32,
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

/// [`show`] 결과.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DocViewOutput {
    /// 선택 단어 강조가 상한을 넘었다
    pub highlight_exceeded: bool,
    /// 미리보기 단독 모드에서 블록을 더블클릭했다 → 탭 모드를 분할로 바꿔야 한다
    pub switch_to_split: bool,
    /// 미리보기에서 누른 링크 중 앱이 처리할 것 (P3-09)
    pub link: Option<LinkAction>,
}

/// 탭 본문을 그린다. 남은 영역을 모두 차지한다. `scroll_sync`는 분할 모드 스크롤 동기화 켜기/끄기.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    split_ratio: &mut f32,
    scroll_sync: bool,
    options: &EditorOptions,
    zoom: f32,
    palette: &Palette,
) -> DocViewOutput {
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

    let mut output = DocViewOutput::default();
    match shown {
        ViewMode::Source => output.highlight_exceeded = source_in(ui, full, tab, options, palette),
        ViewMode::Preview => {
            let preview = preview_in(ui, full, tab, zoom, palette, None);
            if let Some(line) = preview.double_clicked_line {
                tab.editor.go_to_line(&mut tab.doc, line + 1);
                output.switch_to_split = true;
            }
            output.link = preview.link;
        }
        ViewMode::Split => {
            let x = split_x(full, *split_ratio);
            let left = Rect::from_min_max(full.min, pos2(x, full.bottom()));
            let right = Rect::from_min_max(pos2(x + 1.0, full.top()), full.max);
            // 이번 프레임에 적용될 스크롤 요청이 있으면 그 변화는 사용자 조작이 아니다
            let source_applied = tab.editor.has_pending_scroll();
            let preview_applied = tab.preview.has_pending_scroll();
            output.highlight_exceeded = source_in(ui, left, tab, options, palette);
            let cursor_line = editor::cursor_status(&tab.doc).0.saturating_sub(1);
            let preview = preview_in(ui, right, tab, zoom, palette, Some(cursor_line));
            if let Some(line) = preview.double_clicked_line {
                tab.editor.go_to_line(&mut tab.doc, line + 1);
            }
            // `#앵커` 링크로 미리보기가 제목으로 옮겨 갔으면 소스도 그 줄로 (P3-09)
            if let Some(line) = preview.jumped_line {
                tab.editor.scroll_to_line(line as f32);
            }
            output.link = preview.link.clone();
            splitter(ui, full, x, split_ratio, palette);
            // 미리보기가 위쪽 높이 변화로 스크롤을 옮긴 것(P3-07)도 사용자 조작이 아니다
            let applied = (source_applied, preview_applied || preview.anchored);
            sync_scroll(ui, tab, right, scroll_sync, applied);
        }
    }
    ui.allocate_rect(full, Sense::hover());
    output
}

/// 스크롤을 움직인 쪽에 반대쪽을 맞춘다. `applied`는 (소스, 미리보기)에 이번 프레임 스크롤 요청이 적용됐는지.
fn sync_scroll(
    ui: &egui::Ui,
    tab: &mut Tab,
    preview_rect: Rect,
    enabled: bool,
    applied: (bool, bool),
) {
    let source = tab.doc.scroll_offset_y();
    let preview = tab.preview.offset_y();
    let state = &mut tab.doc_view;
    let source_moved = !applied.0 && (source - state.source_offset).abs() > 0.5;
    let preview_moved = !applied.1 && (preview - state.preview_offset).abs() > 0.5;
    state.source_offset = source;
    state.preview_offset = preview;
    if !enabled || tab.editor.is_repositioning() || tab.preview.is_repositioning() {
        return;
    }
    let preview_drives = match (source_moved, preview_moved) {
        (true, false) => false,
        (false, true) => true,
        (true, true) => ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| preview_rect.contains(p)),
        (false, false) => return,
    };
    // 기준점은 움직인 쪽 화면의 가운데(문서 처음·끝 10%에서는 맨 위·맨 아래로 옮겨 가 양 끝을 맞춘다).
    // 두 쪽은 같은 내용이라도 높이가 달라(소스 줄바꿈, 표 등) 기준점에서 멀수록 어긋나므로, 기준점을 가운데에 두어
    // 화면 안 어디든 기준점과의 거리를 화면 절반 이하로 줄인다. 기준점 가까이 제목이 보이면 그 제목에 맞춘다.
    let headings: Vec<usize> = tab.preview.heading_lines().collect();
    if preview_drives {
        let (fraction, height) = tab.preview.scroll_fraction();
        let preview = &tab.preview;
        let anchor = anchor_point(
            anchor_dy(fraction, height),
            height,
            &headings,
            |dy| preview.line_at(dy),
            |line| Some(preview.offset_for_line(line, 0.0) - preview.offset_y()),
        );
        if let Some((line, dy)) = anchor
            && let Some(y) = tab.editor.offset_for_line(&tab.doc, line, dy)
        {
            tab.editor.set_scroll_offset(y);
            ui.ctx().request_repaint();
        }
    } else if let Some((fraction, height)) = tab.editor.scroll_fraction(&tab.doc) {
        let (editor, doc) = (&tab.editor, &tab.doc);
        let offset = doc.scroll_offset_y();
        let anchor = anchor_point(
            anchor_dy(fraction, height),
            height,
            &headings,
            |dy| editor.line_at(doc, dy),
            |line| editor.offset_for_line(doc, line, 0.0).map(|y| y - offset),
        );
        if let Some((line, dy)) = anchor {
            tab.preview
                .set_scroll_offset(tab.preview.offset_for_line(line, dy));
            ui.ctx().request_repaint();
        }
    }
}

/// 이 비율(화면 높이 대비) 안에 제목이 있으면 기준점을 그 제목으로 옮긴다.
const HEADING_SNAP: f32 = 0.15;
/// 문서 처음·끝에서 기준점이 가운데 ↔ 맨 위·맨 아래로 옮겨 가는 스크롤 비율 구간.
const EDGE_RAMP: f32 = 0.05;

/// 스크롤 비율(0~1)에 따른 기준점의 화면 y: 가운데, 문서 처음·끝 [`EDGE_RAMP`] 구간에서는 맨 위·맨 아래 쪽으로.
fn anchor_dy(fraction: f32, height: f32) -> f32 {
    let position = if fraction < 0.5 {
        0.5 * (fraction / EDGE_RAMP).min(1.0)
    } else {
        1.0 - 0.5 * ((1.0 - fraction) / EDGE_RAMP).min(1.0)
    };
    position * height
}

/// 움직인 쪽의 동기화 기준점 (소스 줄, 화면 위로부터 y).
///
/// `dy` 위치의 줄이 기본이고, `dy`에서 화면 높이의 [`HEADING_SNAP`] 안에 제목이 보이면 가장 가까운 제목으로 바꾼다.
/// `line_at(dy)`는 화면 y → 소스 줄, `screen_y(line)`은 소스 줄 → 화면 y.
fn anchor_point(
    dy: f32,
    height: f32,
    headings: &[usize],
    line_at: impl Fn(f32) -> Option<f32>,
    screen_y: impl Fn(f32) -> Option<f32>,
) -> Option<(f32, f32)> {
    let line = line_at(dy)?;
    let snap = height * HEADING_SNAP;
    let (first, last) = (line_at(dy - snap)?, line_at(dy + snap)?);
    let start = headings.partition_point(|&h| (h as f32) < first);
    let end = headings.partition_point(|&h| (h as f32) <= last);
    let nearest = headings[start..end]
        .iter()
        .filter_map(|&h| screen_y(h as f32).map(|y| (h as f32, y)))
        .filter(|&(_, y)| (y - dy).abs() <= snap && (0.0..height).contains(&y))
        .min_by(|a, b| (a.1 - dy).abs().total_cmp(&(b.1 - dy).abs()));
    Some(nearest.unwrap_or((line, dy)))
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
    // `.md` 문서는 소스에 Markdown 구문 색상 (P3-10)
    tab.editor.set_markdown_syntax(tab.doc.is_markdown());
    tab.editor
        .show(&mut child, &mut tab.doc, options, palette)
        .highlight_exceeded
}

fn preview_in(
    ui: &mut egui::Ui,
    rect: Rect,
    tab: &mut Tab,
    zoom: f32,
    palette: &Palette,
    cursor_line: Option<usize>,
) -> PreviewOutput {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    tab.preview
        .show(&mut child, &tab.doc, zoom, palette, cursor_line)
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
    use crate::ui::markdown_view;
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

    thread_local! {
        /// 테스트 프레임에 넘길 스크롤 동기화 켜기/끄기와 입력 시각(초).
        static SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
        static TIME: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    }

    /// 헤드리스 프레임: `width`×800 창 전체에 활성 탭 본문을 그린다. 한 프레임 = 1/60초.
    fn frame(
        ctx: &egui::Context,
        tabs: &mut Tabs,
        ratio: &mut f32,
        width: f32,
        events: Vec<Event>,
    ) -> DocViewOutput {
        let time = TIME.with(|t| {
            t.set(t.get() + 1.0 / 60.0);
            t.get()
        });
        let raw = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 800.0))),
            time: Some(time),
            predicted_dt: 1.0 / 60.0,
            ..Default::default()
        };
        let mut output = DocViewOutput::default();
        let mut full = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    output = show(
                        ui,
                        tabs.active_mut(),
                        ratio,
                        SYNC.with(std::cell::Cell::get),
                        &options(),
                        1.0,
                        &Palette::light(),
                    );
                });
        });
        full.textures_delta.clear();
        output
    }

    /// 마우스를 `pos`에 두고 휠로 `dy`(양수 = 아래로)만큼 굴린 뒤 애니메이션이 끝날 때까지 그린다.
    fn wheel(ctx: &egui::Context, tabs: &mut Tabs, ratio: &mut f32, pos: Pos2, dy: f32) {
        frame(ctx, tabs, ratio, 1200.0, vec![Event::PointerMoved(pos)]);
        let event = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -dy),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Default::default(),
        };
        frame(ctx, tabs, ratio, 1200.0, vec![event]);
        for _ in 0..40 {
            frame(ctx, tabs, ratio, 1200.0, Vec::new());
        }
    }

    fn click(pos: Pos2) -> Vec<Event> {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        vec![Event::PointerMoved(pos), button(true), button(false)]
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

    fn split_tabs(paragraphs: usize) -> (egui::Context, Tabs, f32) {
        let ctx = egui::Context::default();
        let mut tabs = markdown_tabs(paragraphs);
        tabs.active_mut().view_mode = ViewMode::Split;
        let mut ratio = DEFAULT_SPLIT_RATIO;
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        (ctx, tabs, ratio)
    }

    fn top_lines(tabs: &Tabs) -> (f32, f32) {
        let tab = tabs.active();
        (
            tab.editor.top_line(&tab.doc).unwrap(),
            tab.preview.top_line().unwrap(),
        )
    }

    #[test]
    fn scrolling_source_moves_preview_and_back() {
        let (ctx, mut tabs, mut ratio) = split_tabs(300);
        wheel(&ctx, &mut tabs, &mut ratio, pos2(300.0, 400.0), 3000.0);
        let (source, preview) = top_lines(&tabs);
        assert!(source > 50.0, "소스가 스크롤되지 않음: {source}");
        assert!(
            (source - preview).abs() <= 1.0,
            "소스 {source} · 미리보기 {preview}"
        );

        // 미리보기를 굴리면 소스가 따라온다
        wheel(&ctx, &mut tabs, &mut ratio, pos2(900.0, 400.0), 2000.0);
        let (source2, preview2) = top_lines(&tabs);
        assert!(
            preview2 > preview + 20.0,
            "미리보기가 스크롤되지 않음: {preview2}"
        );
        assert!(
            (source2 - preview2).abs() <= 1.0,
            "소스 {source2} · 미리보기 {preview2}"
        );
    }

    #[test]
    fn synced_panes_stay_still_without_feedback() {
        let (ctx, mut tabs, mut ratio) = split_tabs(300);
        wheel(&ctx, &mut tabs, &mut ratio, pos2(300.0, 400.0), 2500.0);
        let offsets = |tabs: &Tabs| {
            let tab = tabs.active();
            (tab.doc.scroll_offset_y(), tab.preview.offset_y())
        };
        let settled = offsets(&tabs);
        for _ in 0..20 {
            frame(&ctx, &mut tabs, &mut ratio, 1200.0, Vec::new());
            assert_eq!(offsets(&tabs), settled, "멈춘 뒤 스크롤이 움직임");
        }
    }

    #[test]
    fn sync_off_scrolls_independently() {
        let (ctx, mut tabs, mut ratio) = split_tabs(300);
        SYNC.with(|s| s.set(false));
        wheel(&ctx, &mut tabs, &mut ratio, pos2(300.0, 400.0), 3000.0);
        SYNC.with(|s| s.set(true));
        let (source, preview) = top_lines(&tabs);
        assert!(source > 50.0);
        assert_eq!(preview, 0.0);
    }

    #[test]
    fn double_click_on_preview_block_moves_source_cursor() {
        let (ctx, mut tabs, mut ratio) = split_tabs(50);
        // 미리보기 두 번째 블록(`문단 1`, 소스 줄 2)의 가운데
        let block = tabs.active().preview.positions()[1];
        let pos = pos2(900.0, block.top + block.height / 2.0);
        frame(&ctx, &mut tabs, &mut ratio, 1200.0, click(pos));
        frame(&ctx, &mut tabs, &mut ratio, 1200.0, click(pos));
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        assert_eq!(editor::cursor_status(&tabs.active().doc).0, 3);

        // 미리보기 단독이면 분할로 바꾸라고 알린다 (앞 클릭과 이어진 세 번 클릭으로 보지 않게 1초 기다린다)
        tabs.active_mut().view_mode = ViewMode::Preview;
        for _ in 0..60 {
            frame(&ctx, &mut tabs, &mut ratio, 1200.0, Vec::new());
        }
        let block = tabs.active().preview.positions()[4];
        let pos = pos2(
            600.0,
            block.top + block.height / 2.0 - tabs.active().preview.offset_y(),
        );
        frame(&ctx, &mut tabs, &mut ratio, 1200.0, click(pos));
        let output = frame(&ctx, &mut tabs, &mut ratio, 1200.0, click(pos));
        assert!(output.switch_to_split);
        assert_eq!(editor::cursor_status(&tabs.active().doc).0, 9);
    }

    #[test]
    fn anchor_link_in_split_moves_source_too() {
        let ctx = egui::Context::default();
        let mut tabs = Tabs::new();
        let filler: String = (0..80)
            .map(|i| {
                format!(
                    "문단 {i}

"
                )
            })
            .collect();
        tabs.active_mut().doc.set_text(format!(
            "[끝으로](#끝)

{filler}## 끝
"
        ));
        tabs.active_mut().view_mode = ViewMode::Split;
        let mut ratio = DEFAULT_SPLIT_RATIO;
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        // 미리보기 첫 블록(링크)의 첫 글자 근처를 누른다
        let link = tabs.active().preview.positions()[0];
        let point = pos2(600.0 + 16.0 + 6.0, link.top + link.height / 2.0);
        for _ in 0..30 {
            frame(&ctx, &mut tabs, &mut ratio, 1200.0, Vec::new());
        }
        frame(&ctx, &mut tabs, &mut ratio, 1200.0, click(point));
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        let heading = 2 + 80 * 2;
        let tab = tabs.active();
        let source = tab.editor.top_line(&tab.doc).unwrap();
        assert!(
            source > 100.0,
            "소스가 제목 쪽으로 따라가야 함: 맨 위 줄 {source} (제목 {heading})"
        );
        assert!(tab.preview.offset_y() > 0.0);
    }

    /// PLAN.md Phase 3 완료 기준(👤 확인 항목)의 자동 측정: `docs/PLAN.md`를 처음부터 끝까지 소스 쪽에서 스크롤하며
    /// 두 쪽 모두 화면에 보이는 모든 제목의 위치 차이(화면 높이 기준)를 잰다.
    #[test]
    fn plan_md_headings_align_within_ten_percent() {
        let plan = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/PLAN.md"),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut tabs = Tabs::new();
        tabs.active_mut().doc.set_text(&plan);
        tabs.active_mut().view_mode = ViewMode::Split;
        let mut ratio = DEFAULT_SPLIT_RATIO;
        frames(&ctx, &mut tabs, &mut ratio, 1200.0);
        let headings = crate::core::markdown::parse(&plan).anchors;
        let height = 800.0;
        let mut checked = 0;
        let mut worst = 0.0_f32;
        let mut over = 0;
        loop {
            wheel(&ctx, &mut tabs, &mut ratio, pos2(300.0, 400.0), 200.0);
            let tab = tabs.active();
            let source_offset = tab.doc.scroll_offset_y();
            let preview_offset = tab.preview.offset_y();
            for h in &headings {
                let line = h.line as f32;
                let source_y =
                    tab.editor.offset_for_line(&tab.doc, line, 0.0).unwrap() - source_offset;
                let preview_y =
                    markdown_view::line_to_y(tab.preview.positions(), line) - preview_offset;
                // 두 쪽 모두 화면 안에 보이는 제목만
                if (0.0..height).contains(&source_y) && (0.0..height).contains(&preview_y) {
                    let diff = (source_y - preview_y).abs() / height;
                    if diff > 0.10 {
                        over += 1;
                    }
                    worst = worst.max(diff);
                    checked += 1;
                }
            }
            if tab.editor.top_line(&tab.doc).unwrap() + 40.0 >= tab.doc.line_count() as f32 {
                break;
            }
            if checked > 5000 {
                break;
            }
        }
        println!(
            "PLAN.md 제목 정렬: {checked}회 확인, 10% 초과 {over}회, 최대 차이 {:.1}%",
            worst * 100.0
        );
        assert!(checked > 50, "확인한 제목이 너무 적음: {checked}");
        // 두 쪽 레이아웃 차이(소스 줄바꿈, 미리보기 표 칸 줄바꿈 — P3-03 사용자 결정)로 큰 표 근처 제목은 10%를
        // 넘을 수 있다 — 회귀 방지 기준 (P3-08 11.0% → P3-03 표 줄바꿈 후 19.0%)
        assert!(worst <= 0.22, "최대 차이 {:.1}% > 22%", worst * 100.0);
        assert!(
            over * 10 <= checked,
            "10% 초과가 너무 많음: {over}/{checked}"
        );
    }

    /// 모드 전환 소요 시간 —`cargo test --release --lib -- --ignored --nocapture mode_switch_time`
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
