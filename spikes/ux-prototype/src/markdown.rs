//! 프로토타입용 간이 Markdown 미리보기.
//!
//! 실제 렌더러(pulldown-cmark 기반, 소스 위치 대응)는 P0-08 / Phase 3에서 만든다.
//! 여기서는 화면 첫인상을 보기 위한 최소 블록(제목, 문단, 목록, 인용, 코드, 표)만 그린다.

use eframe::egui::{
    self, Color32, FontFamily, FontId, RichText, Stroke, TextFormat, text::LayoutJob,
};

use crate::fonts::BOLD_FAMILY;
use crate::theme::Palette;

enum Block {
    Heading(usize, String),
    Paragraph(String),
    ListItem(String),
    Quote(String),
    Code(String),
    Table(Vec<Vec<String>>),
    Blank,
}

/// (시작 줄 번호(0부터), 끝 줄 번호(포함), 블록)
fn parse(text: &str) -> Vec<(usize, usize, Block)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let start = i;
        if line.trim_start().starts_with("```") {
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            blocks.push((start, i.min(lines.len() - 1), Block::Code(code.join("\n"))));
            i += 1;
            continue;
        }
        if line.trim_start().starts_with('|') {
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                let cells: Vec<String> = lines[i]
                    .trim()
                    .trim_matches('|')
                    .split('|')
                    .map(|c| c.trim().to_owned())
                    .collect();
                let is_separator = cells
                    .iter()
                    .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')));
                if !is_separator {
                    rows.push(cells);
                }
                i += 1;
            }
            blocks.push((start, i - 1, Block::Table(rows)));
            continue;
        }
        let trimmed = line.trim();
        let block = if trimmed.is_empty() {
            Block::Blank
        } else if let Some(level) = heading_level(trimmed) {
            Block::Heading(level, trimmed[level + 1..].to_owned())
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            Block::ListItem(item.to_owned())
        } else if let Some(quote) = trimmed.strip_prefix('>') {
            Block::Quote(quote.trim().to_owned())
        } else {
            // 연속된 일반 줄은 한 문단으로 합친다.
            let mut paragraph = trimmed.to_owned();
            while i + 1 < lines.len() && is_plain(lines[i + 1]) {
                i += 1;
                paragraph.push(' ');
                paragraph.push_str(lines[i].trim());
            }
            Block::Paragraph(paragraph)
        };
        blocks.push((start, i, block));
        i += 1;
    }
    blocks
}

fn heading_level(line: &str) -> Option<usize> {
    let level = line.chars().take_while(|&c| c == '#').count();
    (1..=6)
        .contains(&level)
        .then_some(level)
        .filter(|&l| line[l..].starts_with(' '))
}

fn is_plain(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty()
        && heading_level(t).is_none()
        && !t.starts_with("- ")
        && !t.starts_with("* ")
        && !t.starts_with('>')
        && !t.starts_with('|')
        && !t.starts_with("```")
}

/// 미리보기를 그린다. 더블클릭한 블록의 시작 줄(0부터)을 돌려준다.
pub fn preview(
    ui: &mut egui::Ui,
    text: &str,
    palette: &Palette,
    zoom: f32,
    cursor_line: Option<usize>,
) -> Option<usize> {
    let mut jump_to = None;
    let base = 15.0 * zoom;
    ui.set_max_width(ui.available_width().min(860.0));

    for (start, end, block) in parse(text) {
        let is_cursor_block = cursor_line.is_some_and(|l| (start..=end).contains(&l));
        let frame = egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 2))
            .fill(if is_cursor_block {
                palette.cursor_block
            } else {
                Color32::TRANSPARENT
            });
        let response = frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                render_block(ui, &block, palette, base);
            })
            .response
            .interact(egui::Sense::click());
        if response.double_clicked() {
            jump_to = Some(start);
        }
    }
    jump_to
}

fn render_block(ui: &mut egui::Ui, block: &Block, palette: &Palette, base: f32) {
    match block {
        Block::Heading(level, text) => {
            let size = match level {
                1 => base * 1.9,
                2 => base * 1.5,
                3 => base * 1.25,
                _ => base * 1.1,
            };
            ui.add_space(base * 0.4);
            ui.label(
                RichText::new(text)
                    .size(size)
                    .family(FontFamily::Name(BOLD_FAMILY.into())),
            );
            if *level <= 2 {
                let rect = ui.available_rect_before_wrap();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                );
                ui.add_space(4.0);
            }
        }
        Block::Paragraph(text) => {
            ui.label(inline(ui, text, palette, base));
        }
        Block::ListItem(text) => {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(base * 0.6);
                ui.label(RichText::new("•").size(base));
                ui.label(inline(ui, text, palette, base));
            });
        }
        Block::Quote(text) => {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 12,
                    right: 4,
                    top: 2,
                    bottom: 2,
                })
                .show(ui, |ui| {
                    let label = ui.label(inline(ui, text, palette, base).weak_color(ui));
                    ui.painter().vline(
                        label.rect.left() - 8.0,
                        label.rect.y_range(),
                        Stroke::new(3.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                    );
                });
        }
        Block::Code(code) => {
            egui::Frame::new()
                .fill(palette.preview_code_bg)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(code).monospace().size(base * 0.9));
                });
        }
        Block::Table(rows) => {
            egui::Grid::new(ui.next_auto_id())
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for (r, row) in rows.iter().enumerate() {
                        for cell in row {
                            if r == 0 {
                                ui.label(
                                    RichText::new(cell)
                                        .size(base)
                                        .family(FontFamily::Name(BOLD_FAMILY.into())),
                                );
                            } else {
                                ui.label(inline(ui, cell, palette, base));
                            }
                        }
                        ui.end_row();
                    }
                });
        }
        Block::Blank => {
            ui.add_space(base * 0.3);
        }
    }
}

/// `**굵게**`, `` `코드` ``, `[링크](주소)`만 처리하는 간이 인라인 렌더러.
fn inline(ui: &egui::Ui, text: &str, palette: &Palette, base: f32) -> LayoutJob {
    let normal = TextFormat::simple(
        FontId::new(base, FontFamily::Proportional),
        ui.visuals().text_color(),
    );
    let bold = TextFormat::simple(
        FontId::new(base, FontFamily::Name(BOLD_FAMILY.into())),
        ui.visuals().strong_text_color(),
    );
    let code = TextFormat {
        background: palette.preview_code_bg,
        ..TextFormat::simple(
            FontId::new(base * 0.9, FontFamily::Monospace),
            ui.visuals().text_color(),
        )
    };
    let link = TextFormat {
        underline: Stroke::new(1.0, ui.visuals().hyperlink_color),
        ..TextFormat::simple(
            FontId::new(base, FontFamily::Proportional),
            ui.visuals().hyperlink_color,
        )
    };

    let mut job = LayoutJob::default();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            job.append(&after[..end], 0.0, bold.clone());
            rest = &after[end + 2..];
        } else if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            job.append(&after[..end], 0.0, code.clone());
            rest = &after[end + 1..];
        } else if let Some(after) = rest.strip_prefix('[')
            && let Some(mid) = after.find("](")
            && let Some(end) = after[mid..].find(')')
        {
            job.append(&after[..mid], 0.0, link.clone());
            rest = &after[mid + end + 1..];
        } else {
            // 첫 문자는 특수 문자로 시작하지 못한 경우이므로 한 글자(멀티바이트 포함) 건너뛰고 찾는다.
            let first = rest.chars().next().map_or(1, char::len_utf8);
            let next = rest[first..]
                .find(['*', '`', '['])
                .map_or(rest.len(), |p| p + first);
            job.append(&rest[..next], 0.0, normal.clone());
            rest = &rest[next..];
        }
    }
    job.wrap.max_width = ui.available_width();
    job
}

trait WeakColor {
    fn weak_color(self, ui: &egui::Ui) -> Self;
}

impl WeakColor for LayoutJob {
    fn weak_color(mut self, ui: &egui::Ui) -> Self {
        let weak = ui.visuals().weak_text_color();
        for section in &mut self.sections {
            section.format.color = weak;
        }
        self
    }
}
