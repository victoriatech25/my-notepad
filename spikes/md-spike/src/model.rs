//! B안: pulldown-cmark 이벤트 → 블록 모델 (블록 종류 + 소스 줄 범위 + 인라인 스팬).
//!
//! PLAN.md §2.5: 이 모델이 UI와 분리된 `core::markdown`의 원형이다. 스크롤 동기화(P0-09)는
//! 블록의 소스 줄 범위와 화면 y좌표를 짝지어 만든다.

use std::ops::Range;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub strong: bool,
    pub emph: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
}

pub struct Span {
    pub text: String,
    pub style: SpanStyle,
}

pub type Inlines = Vec<Span>;

pub struct ListItem {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

pub enum BlockKind {
    Heading {
        level: u8,
        text: Inlines,
    },
    Paragraph(Inlines),
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    Code {
        lang: String,
        text: String,
    },
    Table {
        align: Vec<Alignment>,
        head: Vec<Inlines>,
        rows: Vec<Vec<Inlines>>,
    },
    Rule,
    Html(String),
}

impl BlockKind {
    pub fn name(&self) -> &'static str {
        match self {
            BlockKind::Heading { .. } => "제목",
            BlockKind::Paragraph(_) => "문단",
            BlockKind::List { .. } => "목록",
            BlockKind::Quote(_) => "인용",
            BlockKind::Code { .. } => "코드",
            BlockKind::Table { .. } => "표",
            BlockKind::Rule => "수평선",
            BlockKind::Html(_) => "HTML",
        }
    }
}

pub struct Block {
    pub kind: BlockKind,
    /// 소스 줄 범위 (0부터, 끝 포함)
    pub first_line: usize,
    pub last_line: usize,
}

pub struct Document {
    pub blocks: Vec<Block>,
    pub line_count: usize,
}

pub fn parse(text: &str) -> Document {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let events: Vec<(Event<'_>, Range<usize>)> =
        Parser::new_ext(text, options).into_offset_iter().collect();

    // 바이트 위치 → 줄 번호
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let lines = Lines(&line_starts);

    let mut i = 0;
    let blocks = parse_blocks(&events, &mut i, None, &lines);
    Document {
        blocks,
        line_count: line_starts.len(),
    }
}

struct Lines<'a>(&'a [usize]);

impl Lines<'_> {
    fn of(&self, byte: usize) -> usize {
        self.0
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    }
    fn range(&self, r: &Range<usize>) -> (usize, usize) {
        let last = r.end.saturating_sub(1).max(r.start);
        (self.of(r.start), self.of(last))
    }
}

type Events<'a> = [(Event<'a>, Range<usize>)];

fn block(kind: BlockKind, range: &Range<usize>, lines: &Lines<'_>) -> Block {
    let (first_line, last_line) = lines.range(range);
    Block {
        kind,
        first_line,
        last_line,
    }
}

fn is_inline_start(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Text(_)
            | Event::Code(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::InlineHtml(_)
            | Event::Start(
                Tag::Emphasis
                    | Tag::Strong
                    | Tag::Strikethrough
                    | Tag::Link { .. }
                    | Tag::Image { .. }
            )
    )
}

/// `until`(끝 태그)를 만날 때까지 블록을 읽는다. `None`이면 문서 끝까지.
fn parse_blocks(
    events: &Events<'_>,
    i: &mut usize,
    until: Option<TagEnd>,
    lines: &Lines<'_>,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    while *i < events.len() {
        let (event, range) = &events[*i];
        match event {
            Event::End(end) if Some(*end) == until => {
                *i += 1;
                return blocks;
            }
            Event::Start(Tag::Paragraph) => {
                *i += 1;
                let text = parse_inlines(events, i, Some(TagEnd::Paragraph));
                blocks.push(block(BlockKind::Paragraph(text), range, lines));
            }
            Event::Start(Tag::Heading { level: heading, .. }) => {
                let heading = *heading;
                let level = heading as u8;
                *i += 1;
                let text = parse_inlines(events, i, Some(TagEnd::Heading(heading)));
                blocks.push(block(BlockKind::Heading { level, text }, range, lines));
            }
            Event::Start(Tag::BlockQuote(_)) => {
                *i += 1;
                let children = parse_blocks(events, i, Some(TagEnd::BlockQuote(None)), lines);
                blocks.push(block(BlockKind::Quote(children), range, lines));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                *i += 1;
                let mut code = String::new();
                while *i < events.len() {
                    match &events[*i].0 {
                        Event::Text(t) => code.push_str(t),
                        Event::End(TagEnd::CodeBlock) => {
                            *i += 1;
                            break;
                        }
                        _ => {}
                    }
                    *i += 1;
                }
                if code.ends_with('\n') {
                    code.pop();
                }
                blocks.push(block(BlockKind::Code { lang, text: code }, range, lines));
            }
            Event::Start(Tag::List(start)) => {
                let start = *start;
                *i += 1;
                let mut items = Vec::new();
                while *i < events.len() {
                    match &events[*i].0 {
                        Event::Start(Tag::Item) => {
                            *i += 1;
                            let mut task = None;
                            if let Some((Event::TaskListMarker(checked), _)) = events.get(*i) {
                                task = Some(*checked);
                                *i += 1;
                            }
                            let blocks = parse_blocks(events, i, Some(TagEnd::Item), lines);
                            items.push(ListItem { task, blocks });
                        }
                        Event::End(TagEnd::List(_)) => {
                            *i += 1;
                            break;
                        }
                        _ => *i += 1,
                    }
                }
                blocks.push(block(BlockKind::List { start, items }, range, lines));
            }
            Event::Start(Tag::Table(align)) => {
                let align = align.clone();
                *i += 1;
                let mut head = Vec::new();
                let mut rows: Vec<Vec<Inlines>> = Vec::new();
                let mut in_head = false;
                while *i < events.len() {
                    match &events[*i].0 {
                        Event::Start(Tag::TableHead) => {
                            in_head = true;
                            *i += 1;
                        }
                        Event::End(TagEnd::TableHead) => {
                            in_head = false;
                            *i += 1;
                        }
                        Event::Start(Tag::TableRow) => {
                            rows.push(Vec::new());
                            *i += 1;
                        }
                        Event::Start(Tag::TableCell) => {
                            *i += 1;
                            let cell = parse_inlines(events, i, Some(TagEnd::TableCell));
                            if in_head {
                                head.push(cell);
                            } else if let Some(row) = rows.last_mut() {
                                row.push(cell);
                            }
                        }
                        Event::End(TagEnd::Table) => {
                            *i += 1;
                            break;
                        }
                        _ => *i += 1,
                    }
                }
                blocks.push(block(BlockKind::Table { align, head, rows }, range, lines));
            }
            Event::Rule => {
                *i += 1;
                blocks.push(block(BlockKind::Rule, range, lines));
            }
            Event::Start(Tag::HtmlBlock) => {
                *i += 1;
                let mut html = String::new();
                while *i < events.len() {
                    match &events[*i].0 {
                        Event::Html(t) | Event::Text(t) => html.push_str(t),
                        Event::End(TagEnd::HtmlBlock) => {
                            *i += 1;
                            break;
                        }
                        _ => {}
                    }
                    *i += 1;
                }
                blocks.push(block(
                    BlockKind::Html(html.trim_end().to_owned()),
                    range,
                    lines,
                ));
            }
            // 느슨하지 않은(tight) 목록 항목은 문단 태그 없이 인라인이 바로 온다.
            e if is_inline_start(e) => {
                let start = range.start;
                let text = parse_inlines(events, i, None);
                let end = events
                    .get(*i)
                    .map_or(start + 1, |(_, r)| r.start.max(start + 1));
                blocks.push(block(BlockKind::Paragraph(text), &(start..end), lines));
            }
            _ => *i += 1,
        }
    }
    blocks
}

/// 인라인 이벤트를 스팬으로. `until`이 None이면 인라인이 아닌 이벤트를 만날 때 멈춘다.
fn parse_inlines(events: &Events<'_>, i: &mut usize, until: Option<TagEnd>) -> Inlines {
    let mut spans: Inlines = Vec::new();
    let mut style = SpanStyle::default();
    let push = |spans: &mut Inlines, text: &str, style: SpanStyle| {
        if let Some(last) = spans.last_mut()
            && last.style == style
        {
            last.text.push_str(text);
        } else {
            spans.push(Span {
                text: text.to_owned(),
                style,
            });
        }
    };
    while *i < events.len() {
        let event = &events[*i].0;
        match event {
            Event::End(end) if Some(*end) == until => {
                *i += 1;
                break;
            }
            Event::Text(t) => push(&mut spans, t, style),
            Event::Code(t) => push(
                &mut spans,
                t,
                SpanStyle {
                    code: true,
                    ..style
                },
            ),
            Event::SoftBreak => push(&mut spans, " ", style),
            Event::HardBreak => push(&mut spans, "\n", style),
            Event::InlineHtml(t) => push(
                &mut spans,
                t,
                SpanStyle {
                    code: true,
                    ..style
                },
            ),
            Event::Start(Tag::Emphasis) => style.emph = true,
            Event::End(TagEnd::Emphasis) => style.emph = false,
            Event::Start(Tag::Strong) => style.strong = true,
            Event::End(TagEnd::Strong) => style.strong = false,
            Event::Start(Tag::Strikethrough) => style.strike = true,
            Event::End(TagEnd::Strikethrough) => style.strike = false,
            Event::Start(Tag::Link { .. }) => style.link = true,
            Event::End(TagEnd::Link) => style.link = false,
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => {}
            _ if until.is_none() => break,
            _ => {}
        }
        *i += 1;
    }
    spans
}
