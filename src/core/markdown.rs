//! Markdown 파싱 및 블록 모델 (PLAN.md §2.5, §3.8, ADR 0002, P3-01).
//!
//! - `pulldown-cmark`(GFM: 표, 취소선, 작업 목록, 각주, YAML Front Matter) 이벤트를 **블록 모델**로 바꾼다.
//!   블록 = 종류 + 소스 줄 범위 + 인라인 스팬. 미리보기 렌더러(`ui::markdown_view`)와 스크롤 동기화(P3-08)는
//!   이 모델만 보고 파서 타입에 의존하지 않는다.
//! - 줄 번호는 0부터이며 문서 텍스트(`LF` 정규화) 기준이다. 블록의 `last_line`은 끝을 포함한다.
//! - 제목마다 GitHub 방식 앵커 슬러그([`slugify`])를 붙인다. 한글은 그대로 두고, 같은 슬러그가 다시 나오면
//!   `-1`, `-2` … 접미사를 붙인다 (인용·목록 안의 제목도 문서 순서대로 센다).
//! - [`MarkdownCache`]는 문서 버전(`revision`)이 바뀔 때만 다시 파싱한다 (§3.8-2 매 프레임 재파싱 금지).
//!   [`BACKGROUND_PARSE_BYTES`]를 넘는 문서는 작업 스레드에서 파싱하고, 끝날 때까지 이전 결과를 쓴다 (P3-05).
//!
//! - 소스 모드 구문 색상([`syntax_spans`])도 같은 파서의 위치 정보로 계산한다 (P3-10, `syntect` 없이).
//!
//! 원시 HTML은 실행하지 않고 원문 그대로 보관하며(§3.8-4), Mermaid·수식은 일반 코드 블록·텍스트로 남는다
//! (미지원 라벨은 렌더러가 붙인다).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

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
    /// 각주 이름 (본문에서 처음 참조한 순서 — 화면 번호는 순서 + 1)
    pub footnotes: Vec<String>,
    /// 각주를 처음 참조한 소스 줄 (`footnotes`와 같은 순서)
    pub footnote_ref_lines: Vec<usize>,
    /// 소스 줄 수 (끝 개행 뒤의 빈 줄 포함)
    pub line_count: usize,
}

impl Document {
    /// 앵커 슬러그(`#` 없이)에 해당하는 제목의 소스 줄.
    pub fn anchor_line(&self, slug: &str) -> Option<usize> {
        self.anchors.iter().find(|a| a.slug == slug).map(|a| a.line)
    }

    /// 각주 화면 번호 (1부터, 처음 참조한 순서). 참조가 없는 각주는 `None`.
    pub fn footnote_number(&self, label: &str) -> Option<usize> {
        self.footnotes
            .iter()
            .position(|l| l == label)
            .map(|i| i + 1)
    }

    /// 각주를 처음 참조한 소스 줄.
    pub fn footnote_ref_line(&self, label: &str) -> Option<usize> {
        let i = self.footnotes.iter().position(|l| l == label)?;
        self.footnote_ref_lines.get(i).copied()
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
        footnotes: Vec::new(),
        footnote_ref_lines: Vec::new(),
    };
    let blocks = parser.blocks(None);
    Document {
        blocks,
        anchors: parser.anchors,
        footnotes: parser.footnotes,
        footnote_ref_lines: parser.footnote_ref_lines,
        line_count: line_starts.len(),
    }
}

/// 이보다 큰 문서는 작업 스레드에서 파싱한다 (PLAN.md §3.8-2 "잠정 1MB 초과").
pub const BACKGROUND_PARSE_BYTES: usize = 1 << 20;

/// 문서 버전 기반 파싱 캐시. 탭(문서)마다 하나씩 둔다.
#[derive(Debug, Default)]
pub struct MarkdownCache {
    /// 마지막으로 파싱한 (문서 버전, 텍스트 길이). 다시 불러오기로 버전이 초기화되는 경우를 길이로 보완한다.
    key: Option<(u64, usize)>,
    document: Arc<Document>,
    parse_count: u64,
    /// 작업 스레드에서 파싱 중인 (버전 키, 결과 받는 쪽)
    job: Option<((u64, usize), Receiver<Document>)>,
}

impl MarkdownCache {
    /// `revision`이 마지막 파싱 때와 같으면 캐시된 결과를, 다르면 다시 파싱한 결과를 돌려준다 (항상 이 스레드에서).
    pub fn get(&mut self, revision: u64, text: &str) -> Arc<Document> {
        let key = (revision, text.len());
        if self.key != Some(key) {
            self.document = Arc::new(parse(text));
            self.key = Some(key);
            self.parse_count += 1;
        }
        Arc::clone(&self.document)
    }

    /// [`get`](Self::get)과 같되, [`BACKGROUND_PARSE_BYTES`]를 넘는 문서는 작업 스레드에서 파싱한다.
    /// 파싱이 끝날 때까지는 이전 결과(처음이면 빈 문서)를 돌려주고, 끝나면 `done`을 부른다 (화면 다시 그리기 요청).
    /// 파싱 중에 문서가 또 바뀌면 지금 작업이 끝난 뒤 최신 버전으로 다시 파싱한다.
    pub fn get_or_spawn(
        &mut self,
        revision: u64,
        text: &str,
        done: impl FnOnce() + Send + 'static,
    ) -> Arc<Document> {
        if let Some((job_key, receiver)) = &self.job
            && let Ok(document) = receiver.try_recv()
        {
            self.document = Arc::new(document);
            self.key = Some(*job_key);
            self.parse_count += 1;
            self.job = None;
        }
        let key = (revision, text.len());
        if self.key == Some(key) || text.len() <= BACKGROUND_PARSE_BYTES {
            return self.get(revision, text);
        }
        if self.job.is_none() {
            let (sender, receiver) = channel();
            let owned = text.to_owned();
            log::info!("큰 Markdown 문서 백그라운드 파싱 시작: {} B", owned.len());
            std::thread::spawn(move || {
                let start = Instant::now();
                let document = parse(&owned);
                log::info!(
                    "백그라운드 파싱 끝: {} B, 최상위 블록 {}개, {:.1}ms ({:?})",
                    owned.len(),
                    document.blocks.len(),
                    start.elapsed().as_secs_f64() * 1000.0,
                    std::thread::current().id()
                );
                // 탭이 닫혀 받는 쪽이 없으면 버린다
                if sender.send(document).is_ok() {
                    done();
                }
            });
            self.job = Some((key, receiver));
        }
        Arc::clone(&self.document)
    }

    /// 마지막 결과가 이 버전의 것인지 (다시 파싱할 필요가 없는지).
    pub fn is_current(&self, revision: u64, text: &str) -> bool {
        self.key == Some((revision, text.len()))
    }

    /// 작업 스레드에서 파싱 중인지.
    pub fn is_parsing(&self) -> bool {
        self.job.is_some()
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

/// 소스 모드 구문 색상 종류 (PLAN.md §3.8-6, P3-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    /// 제목 줄 전체 (`#` 포함)
    Heading,
    /// 기울임
    Emphasis,
    /// 굵게
    Strong,
    /// 취소선
    Strike,
    /// 인라인 코드·코드 블록 (펜스 포함)
    Code,
    /// 링크·이미지 (`[글자](주소)` 전체)
    Link,
    /// 목록 기호 (`-`, `*`, `1.`, `[ ]`)
    ListMarker,
    /// 인용 기호 `>`
    QuoteMarker,
    /// Front Matter·원시 HTML·각주 참조
    Meta,
}

/// 구문 색상 구간: 바이트 범위 (오름차순, 겹치지 않음).
pub type SyntaxSpans = Vec<(Range<usize>, SyntaxKind)>;

/// 소스 텍스트의 구문 색상 구간을 계산한다. 안쪽 요소가 바깥 요소보다 우선한다 (예: 제목 안의 인라인 코드).
pub fn syntax_spans(text: &str) -> SyntaxSpans {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    // 바이트마다 종류를 칠한다 (나중에 칠한 안쪽 요소가 이긴다) — 0 = 없음
    let mut paint = vec![0u8; text.len()];
    let mut fill = |range: Range<usize>, kind: SyntaxKind| {
        let end = range.end.min(text.len());
        if range.start < end {
            paint[range.start..end].fill(kind as u8 + 1);
        }
    };
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => fill(range, SyntaxKind::Heading),
            Event::Start(Tag::Emphasis) => fill(range, SyntaxKind::Emphasis),
            Event::Start(Tag::Strong) => fill(range, SyntaxKind::Strong),
            Event::Start(Tag::Strikethrough) => fill(range, SyntaxKind::Strike),
            Event::Start(Tag::CodeBlock(_)) | Event::Code(_) => fill(range, SyntaxKind::Code),
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => fill(range, SyntaxKind::Link),
            Event::Start(Tag::MetadataBlock(_) | Tag::HtmlBlock)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_) => fill(range, SyntaxKind::Meta),
            Event::Start(Tag::Item) => {
                // 항목 맨 앞의 기호만 (`-`, `*`, `+`, `1.`, `1)`)
                let rest = &text[range.clone()];
                let len = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
                fill(range.start..range.start + len, SyntaxKind::ListMarker);
            }
            Event::TaskListMarker(_) => {
                // `[ ]`·`[x]` — 범위가 표시만 가리키므로 앞뒤 대괄호까지 찾는다
                let rest = &text[range.start..];
                let len = rest.find(']').map_or(rest.len(), |i| i + 1);
                fill(range.start..range.start + len, SyntaxKind::ListMarker);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                // 인용 범위 안 줄마다 앞쪽 `>`들
                let mut line_start = range.start;
                for line in text[range.clone()].split_inclusive('\n') {
                    let mut i = 0;
                    for (j, c) in line.char_indices() {
                        match c {
                            '>' => i = j + 1,
                            ' ' | '\t' => {}
                            _ => break,
                        }
                    }
                    if i > 0 {
                        fill(line_start..line_start + i, SyntaxKind::QuoteMarker);
                    }
                    line_start += line.len();
                }
            }
            _ => {}
        }
    }
    let kinds = [
        SyntaxKind::Heading,
        SyntaxKind::Emphasis,
        SyntaxKind::Strong,
        SyntaxKind::Strike,
        SyntaxKind::Code,
        SyntaxKind::Link,
        SyntaxKind::ListMarker,
        SyntaxKind::QuoteMarker,
        SyntaxKind::Meta,
    ];
    let mut spans = SyntaxSpans::new();
    let mut i = 0;
    while i < paint.len() {
        let value = paint[i];
        let start = i;
        while i < paint.len() && paint[i] == value {
            i += 1;
        }
        if value != 0 {
            spans.push((start..i, kinds[value as usize - 1]));
        }
    }
    spans
}

/// 한 군데 편집된 두 텍스트 사이의 위치 대응 (앞·뒤 공통 부분 기준).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditMap {
    /// 앞쪽 공통 부분 길이
    prefix: usize,
    /// 뒤쪽 공통 부분이 시작하는 위치 (옛 텍스트, 새 텍스트)
    old_tail: usize,
    new_tail: usize,
}

impl EditMap {
    /// `old` → `new`. 경계는 두 텍스트 모두의 글자 경계로 맞춘다.
    pub fn new(old: &str, new: &str) -> Self {
        let (o, n) = (old.as_bytes(), new.as_bytes());
        let mut prefix = o.iter().zip(n).take_while(|(a, b)| a == b).count();
        while !new.is_char_boundary(prefix) || !old.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max_suffix = o.len().min(n.len()) - prefix;
        let mut suffix = o
            .iter()
            .rev()
            .zip(n.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        while !new.is_char_boundary(n.len() - suffix) || !old.is_char_boundary(o.len() - suffix) {
            suffix -= 1;
        }
        Self {
            prefix,
            old_tail: o.len() - suffix,
            new_tail: n.len() - suffix,
        }
    }

    /// 옛 텍스트 위치 → 새 텍스트 위치. 편집으로 바뀐 곳 안은 새 편집 범위 끝으로.
    pub fn map(&self, pos: usize) -> usize {
        if pos <= self.prefix {
            pos
        } else if pos >= self.old_tail {
            pos - self.old_tail + self.new_tail
        } else {
            self.new_tail
        }
    }
}

/// `old` 텍스트로 계산한 구간을 한 군데 편집된 `new` 텍스트에 맞게 옮긴다 (다시 계산하기 전까지 색이 흔들리지 않게).
///
/// 앞쪽 공통 부분의 구간은 그대로, 뒤쪽 공통 부분의 구간은 길이 차이만큼 옮기고, 편집한 곳에 걸친 구간은
/// 편집 범위에 맞게 줄이거나 늘린다 ([`EditMap`]).
pub fn shift_spans(spans: &[(Range<usize>, SyntaxKind)], old: &str, new: &str) -> SyntaxSpans {
    if old == new {
        return spans.to_vec();
    }
    let map = EditMap::new(old, new);
    spans
        .iter()
        .filter_map(|(range, kind)| {
            let (start, end) = (map.map(range.start), map.map(range.end));
            (start < end).then_some((start..end, *kind))
        })
        .collect()
}

type Events<'a> = [(Event<'a>, Range<usize>)];

struct BlockParser<'e, 'a> {
    text: &'e str,
    events: &'e Events<'a>,
    pos: usize,
    line_starts: &'e [usize],
    slugger: Slugger,
    anchors: Vec<Anchor>,
    footnotes: Vec<String>,
    footnote_ref_lines: Vec<usize>,
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
                    if !self.footnotes.iter().any(|l| l == label.as_ref()) {
                        self.footnotes.push(label.to_string());
                        let line = self.line_of(self.events[self.pos].1.start);
                        self.footnote_ref_lines.push(line);
                    }
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

    fn kinds_of(text: &str) -> Vec<(&str, SyntaxKind)> {
        syntax_spans(text)
            .into_iter()
            .map(|(r, k)| (&text[r], k))
            .collect()
    }

    #[test]
    fn syntax_spans_mark_markdown_parts() {
        use SyntaxKind::*;
        let text = "# 제목 `코드`

**굵게** *기울임* ~~취소~~ [링크](a.md)

- 항목
1. 번호
- [x] 끝

> 인용
> > 중첩

```rs
fn a() {}
```
";
        let spans = kinds_of(text);
        let has = |part: &str, kind| spans.contains(&(part, kind));
        assert!(has("# 제목 ", Heading), "{spans:?}");
        assert!(has("`코드`", Code), "제목 안의 코드가 이긴다");
        assert!(has("**굵게**", Strong));
        assert!(has("*기울임*", Emphasis));
        assert!(has("~~취소~~", Strike));
        assert!(has("[링크](a.md)", Link));
        assert!(has("-", ListMarker));
        assert!(has("1.", ListMarker));
        assert!(has("[x]", ListMarker));
        assert!(has(">", QuoteMarker));
        assert!(has("> >", QuoteMarker));
        let code = spans
            .iter()
            .find(|(part, kind)| *kind == Code && part.starts_with("```rs"))
            .map(|(part, _)| *part);
        assert_eq!(
            code.map(str::trim_end),
            Some("```rs\nfn a() {}\n```"),
            "{spans:?}"
        );
        // 구간은 겹치지 않고 오름차순
        let raw = syntax_spans(text);
        assert!(raw.windows(2).all(|w| w[0].0.end <= w[1].0.start));
    }

    #[test]
    fn shift_spans_follows_one_edit() {
        let old = "앞 **굵게** 뒤 `코드`
";
        let spans = syntax_spans(old);
        // 앞쪽에 글자를 넣으면 뒤 구간이 그만큼 밀린다
        let new = "앞앞 **굵게** 뒤 `코드`
";
        let shifted = shift_spans(&spans, old, new);
        assert_eq!(shifted, syntax_spans(new));
        // 굵게 안에서 지우면 그 구간이 줄어든다
        let new = "앞 **굵** 뒤 `코드`
";
        let shifted = shift_spans(&spans, old, new);
        assert_eq!(&new[shifted[0].0.clone()], "**굵**");
        assert_eq!(&new[shifted[1].0.clone()], "`코드`");
        // 한글 글자 일부만 같은 경우(앞 바이트 공유)에도 글자 경계를 지킨다
        let old = "가 `a`";
        let new = "각 `a`";
        let shifted = shift_spans(&syntax_spans(old), old, new);
        assert_eq!(&new[shifted[0].0.clone()], "`a`");
    }

    #[test]
    fn empty_document_has_one_line_and_no_blocks() {
        let doc = parse("");
        assert!(doc.blocks.is_empty());
        assert_eq!(doc.line_count, 1);
    }
}
