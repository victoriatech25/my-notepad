//! Markdown 파싱 및 블록 모델 (PLAN.md §2.5, §3.8, ADR 0002, P3-01).
//!
//! - `pulldown-cmark`(GFM: 표, 취소선, 작업 목록, 각주, YAML Front Matter) 이벤트를 **블록 모델**로 바꾼다.
//!   블록 = 종류 + 소스 줄 범위 + 인라인 스팬. 미리보기 렌더러(`ui::markdown_view`)와 스크롤 동기화(P3-08)는
//!   이 모델만 보고 파서 타입에 의존하지 않는다.
//! - 줄 번호는 0부터이며 문서 텍스트(`LF` 정규화) 기준이다. 블록의 `last_line`은 끝을 포함한다.
//! - 제목마다 GitHub 방식 앵커 슬러그([`slugify`])를 붙인다. 한글은 그대로 두고, 같은 슬러그가 다시 나오면
//!   `-1`, `-2` … 접미사를 붙인다 (인용·목록 안의 제목도 문서 순서대로 센다).
//! - [`MarkdownCache`]는 문서 버전(`revision`)이 바뀔 때만 다시 파싱한다 (§3.8-2 매 프레임 재파싱 금지).
//!
//! 원시 HTML은 실행하지 않고 원문 그대로 보관하며(§3.8-4), Mermaid·수식은 일반 코드 블록·텍스트로 남는다
//! (미지원 라벨은 렌더러가 붙인다).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// 인라인 강조 상태.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub strong: bool,
    pub emph: bool,
    pub strike: bool,
}

/// 인라인 스팬 종류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanKind {
    Text,
    /// 인라인 코드
    Code,
    /// 원시 인라인 HTML (원문 표시, `<br>`만 렌더러가 줄바꿈으로 해석)
    Html,
    /// 이미지. 스팬 `text`는 대체 텍스트
    Image {
        url: String,
        title: String,
    },
    /// 각주 참조. 스팬 `text`는 각주 이름
    FootnoteRef,
}

/// 같은 종류·강조·링크가 이어지는 인라인 조각.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub kind: SpanKind,
    pub style: SpanStyle,
    /// 링크 대상 (링크 안이면 `Some`)
    pub link: Option<String>,
}

pub type Inlines = Vec<Span>;

/// 표 열 정렬.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// 작업 목록 체크 상태 (`- [ ]` → `Some(false)`)
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Heading {
        level: u8,
        text: Inlines,
        /// 앵커 슬러그 (중복이면 `-1` 등 접미사 포함)
        anchor: String,
    },
    Paragraph(Inlines),
    List {
        /// 순서 목록의 시작 번호 (비순서 목록은 `None`)
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    Code {
        /// 펜스 코드 블록의 언어 (들여쓰기 코드 블록은 빈 문자열)
        lang: String,
        text: String,
    },
    Table {
        align: Vec<Align>,
        head: Vec<Inlines>,
        rows: Vec<Vec<Inlines>>,
    },
    Rule,
    /// 원시 HTML 블록 (원문)
    Html(String),
    /// YAML Front Matter 본문 (`---` 구분선 제외)
    FrontMatter(String),
    /// 각주 정의
    Footnote {
        label: String,
        blocks: Vec<Block>,
    },
}

impl BlockKind {
    /// 블록 종류 이름 (로그·테스트용).
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
            BlockKind::FrontMatter(_) => "Front Matter",
            BlockKind::Footnote { .. } => "각주",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// 소스 줄 범위 (0부터, 끝 포함)
    pub first_line: usize,
    pub last_line: usize,
}

/// 문서 안의 제목 앵커 (문서 순서, 중첩 블록 포함).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub level: u8,
    pub slug: String,
    /// 제목의 소스 줄
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    /// 최상위 블록 (소스 순서)
    pub blocks: Vec<Block>,
    pub anchors: Vec<Anchor>,
    /// 소스 줄 수 (끝 개행 뒤의 빈 줄 포함)
    pub line_count: usize,
}

impl Document {
    /// 앵커 슬러그(`#` 없이)에 해당하는 제목의 소스 줄.
    pub fn anchor_line(&self, slug: &str) -> Option<usize> {
        self.anchors.iter().find(|a| a.slug == slug).map(|a| a.line)
    }
}

/// 인라인 스팬의 표시 텍스트 (이미지·HTML·각주 참조 제외). 제목 슬러그 계산에 쓴다.
pub fn plain_text(inlines: &[Span]) -> String {
    inlines
        .iter()
        .filter(|s| matches!(s.kind, SpanKind::Text | SpanKind::Code))
        .map(|s| s.text.as_str())
        .collect()
}

/// GitHub 방식 슬러그: 소문자로 바꾸고, 문자·숫자(한글 포함)·`_`·`-`만 남기고, 공백은 `-`로 바꾼다.
pub fn slugify(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// 중복 슬러그에 `-1`, `-2` … 를 붙인다 (github-slugger와 같은 규칙).
#[derive(Default)]
struct Slugger {
    occurrences: HashMap<String, usize>,
}

impl Slugger {
    fn slug(&mut self, text: &str) -> String {
        let original = slugify(text);
        let mut slug = original.clone();
        while self.occurrences.contains_key(&slug) {
            let count = self.occurrences.entry(original.clone()).or_insert(0);
            *count += 1;
            slug = format!("{original}-{count}");
        }
        self.occurrences.insert(slug.clone(), 0);
        slug
    }
}

/// Markdown 텍스트를 블록 모델로 파싱한다.
pub fn parse(text: &str) -> Document {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let events: Vec<(Event<'_>, Range<usize>)> =
        Parser::new_ext(text, options).into_offset_iter().collect();

    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let mut parser = BlockParser {
        text,
        events: &events,
        pos: 0,
        line_starts: &line_starts,
        slugger: Slugger::default(),
        anchors: Vec::new(),
    };
    let blocks = parser.blocks(None);
    Document {
        blocks,
        anchors: parser.anchors,
        line_count: line_starts.len(),
    }
}

/// 문서 버전 기반 파싱 캐시. 탭(문서)마다 하나씩 둔다.
#[derive(Default)]
pub struct MarkdownCache {
    /// 마지막으로 파싱한 (문서 버전, 텍스트 길이). 다시 불러오기로 버전이 초기화되는 경우를 길이로 보완한다.
    key: Option<(u64, usize)>,
    document: Arc<Document>,
    parse_count: u64,
}

impl MarkdownCache {
    /// `revision`이 마지막 파싱 때와 같으면 캐시된 결과를, 다르면 다시 파싱한 결과를 돌려준다.
    pub fn get(&mut self, revision: u64, text: &str) -> Arc<Document> {
        let key = (revision, text.len());
        if self.key != Some(key) {
            self.document = Arc::new(parse(text));
            self.key = Some(key);
            self.parse_count += 1;
        }
        Arc::clone(&self.document)
    }

    /// 지금까지 실제로 파싱한 횟수.
    pub fn parse_count(&self) -> u64 {
        self.parse_count
    }

    /// 다음 [`get`](Self::get)에서 반드시 다시 파싱하게 한다.
    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

type Events<'a> = [(Event<'a>, Range<usize>)];

struct BlockParser<'e, 'a> {
    text: &'e str,
    events: &'e Events<'a>,
    pos: usize,
    line_starts: &'e [usize],
    slugger: Slugger,
    anchors: Vec<Anchor>,
}

impl BlockParser<'_, '_> {
    fn line_of(&self, byte: usize) -> usize {
        self.line_starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    }

    /// 파서 범위 끝의 빈 줄(목록·각주 뒤 등)은 블록 줄 범위에서 뺀다.
    fn block(&self, kind: BlockKind, range: &Range<usize>) -> Block {
        let content = self.text[range.clone()].trim_end();
        let last = (range.start + content.len())
            .saturating_sub(1)
            .max(range.start);
        Block {
            kind,
            first_line: self.line_of(range.start),
            last_line: self.line_of(last),
        }
    }

    /// `until`(끝 태그)를 만날 때까지 블록을 읽는다. `None`이면 문서 끝까지.
    fn blocks(&mut self, until: Option<TagEnd>) -> Vec<Block> {
        let mut blocks = Vec::new();
        while let Some((event, range)) = self.events.get(self.pos) {
            let range = range.clone();
            match event {
                Event::End(end) if Some(*end) == until => {
                    self.pos += 1;
                    return blocks;
                }
                Event::Start(Tag::Paragraph) => {
                    self.pos += 1;
                    let text = self.inlines(Some(TagEnd::Paragraph));
                    blocks.push(self.block(BlockKind::Paragraph(text), &range));
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    let level = *level;
                    self.pos += 1;
                    let text = self.inlines(Some(TagEnd::Heading(level)));
                    let anchor = self.slugger.slug(&plain_text(&text));
                    let block = self.block(
                        BlockKind::Heading {
                            level: level as u8,
                            text,
                            anchor: anchor.clone(),
                        },
                        &range,
                    );
                    self.anchors.push(Anchor {
                        level: level as u8,
                        slug: anchor,
                        line: block.first_line,
                    });
                    blocks.push(block);
                }
                Event::Start(tag @ Tag::BlockQuote(_)) => {
                    let end = tag.to_end();
                    self.pos += 1;
                    let children = self.blocks(Some(end));
                    blocks.push(self.block(BlockKind::Quote(children), &range));
                }
                Event::Start(Tag::FootnoteDefinition(label)) => {
                    let label = label.to_string();
                    self.pos += 1;
                    let children = self.blocks(Some(TagEnd::FootnoteDefinition));
                    let kind = BlockKind::Footnote {
                        label,
                        blocks: children,
                    };
                    blocks.push(self.block(kind, &range));
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    self.pos += 1;
                    let mut code = self.raw_text(TagEnd::CodeBlock);
                    if code.ends_with('\n') {
                        code.pop();
                    }
                    blocks.push(self.block(BlockKind::Code { lang, text: code }, &range));
                }
                Event::Start(Tag::MetadataBlock(kind)) => {
                    let end = TagEnd::MetadataBlock(*kind);
                    self.pos += 1;
                    let yaml = self.raw_text(end);
                    let kind = BlockKind::FrontMatter(yaml.trim_end().to_owned());
                    blocks.push(self.block(kind, &range));
                }
                Event::Start(Tag::HtmlBlock) => {
                    self.pos += 1;
                    let html = self.raw_text(TagEnd::HtmlBlock);
                    let kind = BlockKind::Html(html.trim_end().to_owned());
                    blocks.push(self.block(kind, &range));
                }
                Event::Start(Tag::List(start)) => {
                    let start = *start;
                    self.pos += 1;
                    let items = self.list_items();
                    blocks.push(self.block(BlockKind::List { start, items }, &range));
                }
                Event::Start(Tag::Table(align)) => {
                    let align = align.iter().map(convert_align).collect();
                    self.pos += 1;
                    let (head, rows) = self.table();
                    blocks.push(self.block(BlockKind::Table { align, head, rows }, &range));
                }
                Event::Rule => {
                    self.pos += 1;
                    blocks.push(self.block(BlockKind::Rule, &range));
                }
                // 느슨하지 않은(tight) 목록 항목은 문단 태그 없이 인라인이 바로 온다.
                e if is_inline_start(e) => {
                    let first = self.pos;
                    let text = self.inlines(None);
                    let end = self.events[first..self.pos]
                        .iter()
                        .map(|(_, r)| r.end)
                        .max()
                        .unwrap_or(range.end);
                    blocks.push(self.block(BlockKind::Paragraph(text), &(range.start..end)));
                }
                _ => self.pos += 1,
            }
        }
        blocks
    }

    /// `end` 태그까지의 텍스트·HTML 이벤트를 이어 붙인다 (코드 블록, HTML 블록, Front Matter).
    fn raw_text(&mut self, end: TagEnd) -> String {
        let mut text = String::new();
        while let Some((event, _)) = self.events.get(self.pos) {
            self.pos += 1;
            match event {
                Event::Text(t) | Event::Html(t) => text.push_str(t),
                Event::End(e) if *e == end => break,
                _ => {}
            }
        }
        text
    }

    fn list_items(&mut self) -> Vec<ListItem> {
        let mut items = Vec::new();
        while let Some((event, _)) = self.events.get(self.pos) {
            match event {
                Event::Start(Tag::Item) => {
                    self.pos += 1;
                    let mut task = None;
                    if let Some((Event::TaskListMarker(checked), _)) = self.events.get(self.pos) {
                        task = Some(*checked);
                        self.pos += 1;
                    }
                    let blocks = self.blocks(Some(TagEnd::Item));
                    items.push(ListItem { task, blocks });
                }
                Event::End(TagEnd::List(_)) => {
                    self.pos += 1;
                    break;
                }
                _ => self.pos += 1,
            }
        }
        items
    }

    fn table(&mut self) -> (Vec<Inlines>, Vec<Vec<Inlines>>) {
        let mut head = Vec::new();
        let mut rows: Vec<Vec<Inlines>> = Vec::new();
        let mut in_head = false;
        while let Some((event, _)) = self.events.get(self.pos) {
            self.pos += 1;
            match event {
                Event::Start(Tag::TableHead) => in_head = true,
                Event::End(TagEnd::TableHead) => in_head = false,
                Event::Start(Tag::TableRow) => rows.push(Vec::new()),
                Event::Start(Tag::TableCell) => {
                    let cell = self.inlines(Some(TagEnd::TableCell));
                    if in_head {
                        head.push(cell);
                    } else if let Some(row) = rows.last_mut() {
                        row.push(cell);
                    }
                }
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        (head, rows)
    }

    /// 인라인 이벤트를 스팬으로. `until`이 `None`이면 인라인이 아닌 이벤트를 만날 때 멈춘다.
    fn inlines(&mut self, until: Option<TagEnd>) -> Inlines {
        let mut spans = Inlines::new();
        let mut style = SpanStyle::default();
        let mut link: Option<String> = None;
        while let Some((event, _)) = self.events.get(self.pos) {
            match event {
                Event::End(end) if Some(*end) == until => {
                    self.pos += 1;
                    break;
                }
                Event::Text(t) => push(&mut spans, t, SpanKind::Text, style, &link),
                Event::Code(t) => push(&mut spans, t, SpanKind::Code, style, &link),
                Event::SoftBreak => push(&mut spans, " ", SpanKind::Text, style, &link),
                Event::HardBreak => push(&mut spans, "\n", SpanKind::Text, style, &link),
                Event::InlineHtml(t) => push(&mut spans, t, SpanKind::Html, style, &link),
                Event::FootnoteReference(label) => {
                    push(&mut spans, label, SpanKind::FootnoteRef, style, &link);
                }
                Event::Start(Tag::Emphasis) => style.emph = true,
                Event::End(TagEnd::Emphasis) => style.emph = false,
                Event::Start(Tag::Strong) => style.strong = true,
                Event::End(TagEnd::Strong) => style.strong = false,
                Event::Start(Tag::Strikethrough) => style.strike = true,
                Event::End(TagEnd::Strikethrough) => style.strike = false,
                Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()),
                Event::End(TagEnd::Link) => link = None,
                Event::Start(Tag::Image {
                    dest_url, title, ..
                }) => {
                    let kind = SpanKind::Image {
                        url: dest_url.to_string(),
                        title: title.to_string(),
                    };
                    self.pos += 1;
                    let alt = self.image_alt();
                    spans.push(Span {
                        text: alt,
                        kind,
                        style,
                        link: link.clone(),
                    });
                    continue;
                }
                _ if until.is_none() => break,
                _ => {}
            }
            self.pos += 1;
        }
        spans
    }

    /// 이미지 끝 태그까지의 대체 텍스트.
    fn image_alt(&mut self) -> String {
        let mut alt = String::new();
        let mut depth = 0usize;
        while let Some((event, _)) = self.events.get(self.pos) {
            self.pos += 1;
            match event {
                Event::Start(Tag::Image { .. }) => depth += 1,
                Event::End(TagEnd::Image) if depth == 0 => break,
                Event::End(TagEnd::Image) => depth -= 1,
                Event::Text(t) | Event::Code(t) => alt.push_str(t),
                Event::SoftBreak | Event::HardBreak => alt.push(' '),
                _ => {}
            }
        }
        alt
    }
}

/// 앞 스팬과 종류·강조·링크가 같은 텍스트·코드는 이어 붙인다.
fn push(spans: &mut Inlines, text: &str, kind: SpanKind, style: SpanStyle, link: &Option<String>) {
    let mergeable = matches!(kind, SpanKind::Text | SpanKind::Code);
    if mergeable
        && let Some(last) = spans.last_mut()
        && last.kind == kind
        && last.style == style
        && last.link == *link
    {
        last.text.push_str(text);
        return;
    }
    spans.push(Span {
        text: text.to_owned(),
        kind,
        style,
        link: link.clone(),
    });
}

fn convert_align(align: &Alignment) -> Align {
    match align {
        Alignment::None => Align::None,
        Alignment::Left => Align::Left,
        Alignment::Center => Align::Center,
        Alignment::Right => Align::Right,
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
            | Event::FootnoteReference(_)
            | Event::Start(
                Tag::Emphasis
                    | Tag::Strong
                    | Tag::Strikethrough
                    | Tag::Link { .. }
                    | Tag::Image { .. }
            )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_follows_github_rules() {
        assert_eq!(slugify("3.8 Markdown 소스 보기"), "38-markdown-소스-보기");
        assert_eq!(slugify("FAQ: 자주 묻는 질문?"), "faq-자주-묻는-질문");
        assert_eq!(slugify("제목 (괄호) & 기호!"), "제목-괄호--기호");
        assert_eq!(slugify("snake_case-이름"), "snake_case-이름");
    }

    #[test]
    fn slugger_appends_suffix_for_duplicates() {
        let mut s = Slugger::default();
        assert_eq!(s.slug("제목"), "제목");
        assert_eq!(s.slug("제목"), "제목-1");
        assert_eq!(s.slug("제목"), "제목-2");
        // 접미사가 붙은 슬러그와 같은 제목이 나오면 다시 접미사를 붙인다.
        assert_eq!(s.slug("제목-1"), "제목-1-1");
    }

    #[test]
    fn tight_list_item_spans_its_lines() {
        let doc = parse("- 첫 줄\n  이어지는 줄\n- 둘째\n");
        let BlockKind::List { items, .. } = &doc.blocks[0].kind else {
            panic!("목록이 아님");
        };
        let para = &items[0].blocks[0];
        assert_eq!((para.first_line, para.last_line), (0, 1));
        assert_eq!(
            plain_text(match &para.kind {
                BlockKind::Paragraph(t) => t,
                _ => panic!("문단이 아님"),
            }),
            "첫 줄 이어지는 줄"
        );
    }

    #[test]
    fn link_and_image_spans_keep_targets() {
        let doc = parse("[**굵은 링크**](a.md#제목) ![대체 *텍스트*](img.png \"제목\")\n");
        let BlockKind::Paragraph(spans) = &doc.blocks[0].kind else {
            panic!("문단이 아님");
        };
        assert_eq!(spans[0].text, "굵은 링크");
        assert!(spans[0].style.strong);
        assert_eq!(spans[0].link.as_deref(), Some("a.md#제목"));
        assert_eq!(spans[1].link, None);
        assert_eq!(spans[2].text, "대체 텍스트");
        assert_eq!(
            spans[2].kind,
            SpanKind::Image {
                url: "img.png".into(),
                title: "제목".into()
            }
        );
    }

    #[test]
    fn empty_document_has_one_line_and_no_blocks() {
        let doc = parse("");
        assert!(doc.blocks.is_empty());
        assert_eq!(doc.line_count, 1);
    }
}
