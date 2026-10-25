//! P3-01 Markdown 블록 모델 통합 테스트 (PLAN.md §3.8, ADR 0002).
//!
//! 검증 항목 (WORKPLAN.md P3-01):
//! - `gfm_all.md`의 블록 종류·순서·소스 줄 범위 스냅샷
//! - 제목 슬러그: GitHub 방식(한글 포함), 중복 제목 `-1` 접미사
//! - 같은 버전 재요청 시 재파싱하지 않음(캐시 적중)
//! - 1MB 문서 파싱 시간 기록 (`--ignored`, 릴리스 빌드로 측정)

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use my_notepad::core::markdown::{
    Align, Block, BlockKind, Document, MarkdownCache, SpanKind, parse, plain_text,
};

fn fixture(name: &str) -> String {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "markdown",
        name,
    ]
    .iter()
    .collect();
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} 읽기 실패: {e}", path.display()))
}

/// 블록 트리를 `종류 첫줄-끝줄 요약` 줄로 펼친다 (줄 번호는 1부터, 편집기 표시와 같게).
fn outline(blocks: &[Block], depth: usize, out: &mut String) {
    for block in blocks {
        let indent = "  ".repeat(depth);
        let lines = format!("{}-{}", block.first_line + 1, block.last_line + 1);
        let summary = match &block.kind {
            BlockKind::Heading {
                level,
                text,
                anchor,
            } => format!("H{level} {} #{anchor}", plain_text(text)),
            BlockKind::Paragraph(text) => {
                // 텍스트가 아닌 스팬은 `[종류:내용]`으로, 강제 줄바꿈은 `⏎`로 보인다.
                let kinds: Vec<String> = text
                    .iter()
                    .filter_map(|s| match s.kind {
                        SpanKind::Text => None,
                        SpanKind::Code => Some("code".to_owned()),
                        SpanKind::Html => Some(format!("html:{}", s.text)),
                        SpanKind::Image { .. } => Some(format!("image:{}", s.text)),
                        SpanKind::FootnoteRef => Some(format!("fn:{}", s.text)),
                    })
                    .collect();
                let text = plain_text(text).replace('\n', "⏎");
                format!("{text} [{}]", kinds.join(","))
                    .trim_start()
                    .to_owned()
            }
            BlockKind::List { start, items } => {
                let tasks: Vec<String> = items
                    .iter()
                    .map(|i| match i.task {
                        Some(true) => "x".into(),
                        Some(false) => " ".into(),
                        None => "-".into(),
                    })
                    .collect();
                format!("start={start:?} items={} [{}]", items.len(), tasks.join(""))
            }
            BlockKind::Code { lang, text } => format!("lang={lang} lines={}", text.lines().count()),
            BlockKind::Table { align, head, rows } => {
                let align: String = align
                    .iter()
                    .map(|a| match a {
                        Align::None => '-',
                        Align::Left => 'L',
                        Align::Center => 'C',
                        Align::Right => 'R',
                    })
                    .collect();
                format!("cols={} rows={} align={align}", head.len(), rows.len())
            }
            BlockKind::Html(html) => html.lines().next().unwrap_or_default().to_owned(),
            BlockKind::FrontMatter(yaml) => yaml.lines().next().unwrap_or_default().to_owned(),
            BlockKind::Footnote { label, .. } => format!("[^{label}]"),
            BlockKind::Quote(_) | BlockKind::Rule => String::new(),
        };
        let line = format!("{indent}{} {lines} {summary}", block.kind.name());
        writeln!(out, "{}", line.trim_end()).unwrap();
        match &block.kind {
            BlockKind::Quote(children)
            | BlockKind::Footnote {
                blocks: children, ..
            } => outline(children, depth + 1, out),
            BlockKind::List { items, .. } => {
                for item in items {
                    outline(&item.blocks, depth + 1, out);
                }
            }
            _ => {}
        }
    }
}

fn outline_of(doc: &Document) -> String {
    let mut out = String::new();
    outline(&doc.blocks, 0, &mut out);
    out
}

#[test]
fn gfm_all_block_snapshot() {
    let doc = parse(&fixture("gfm_all.md"));
    let expected = "\
Front Matter 1-4 title: GFM 전체 요소
제목 6-6 H1 제목 1 #제목-1
제목 8-8 H2 제목 2 #제목-2
제목 10-10 H3 제목 3 #제목-3
제목 12-12 H4 제목 4 #제목-4
제목 14-14 H5 제목 5 #제목-5
제목 16-16 H6 제목 6 #제목-6
문단 18-19 문단에 굵게, 기울임, 취소선, 인라인 코드가 섞여 있다. 같은 문단의 둘째 줄이다. [code]
문단 21-21 굵은 기울임과 링크와 https://auto.example.com 자동 링크. []
문단 23-24 줄 끝 역슬래시로⏎강제 줄바꿈. []
인용 26-28
  문단 26-26 인용 첫 줄 []
  인용 28-28
    문단 28-28 중첩 인용 []
수평선 30-30
목록 32-35 start=None items=2 [--]
  문단 32-32 비순서 항목 1 []
  문단 33-33 비순서 항목 2 []
  목록 34-35 start=None items=1 [-]
    문단 34-34 중첩 항목 []
    목록 35-35 start=None items=1 [-]
      문단 35-35 3단계 항목 []
목록 37-38 start=Some(3) items=2 [--]
  문단 37-37 셋부터 시작 []
  문단 38-38 넷 []
목록 40-41 start=None items=2 [ x]
  문단 40-40 할 일 []
  문단 41-41 끝난 일 []
코드 43-47 lang=rust lines=3
코드 49-49 lang= lines=1
표 51-54 cols=4 rows=2 align=LCR-
문단 56-56 [image:로컬 이미지]
문단 58-58 각주 참조와 이름 각주. [fn:1,fn:note]
각주 60-60 [^1]
  문단 60-60 첫 번째 각주. []
각주 62-62 [^note]
  문단 62-62 이름 있는 각주. []
HTML 64-66 <div align=\"center\">
문단 68-68 인라인 Ctrl HTML과 줄바꿈태그. [html:<kbd>,html:</kbd>,html:<br>]
코드 70-73 lang=mermaid lines=2
제목 75-75 H2 제목 2 #제목-2-1
";
    assert_eq!(outline_of(&doc), expected);
    assert_eq!(doc.line_count, 76);
}

#[test]
fn gfm_all_inline_and_table_details() {
    let doc = parse(&fixture("gfm_all.md"));
    let BlockKind::Paragraph(spans) = &doc.blocks[7].kind else {
        panic!("문단이 아님");
    };
    let strong = spans.iter().find(|s| s.text == "굵게").unwrap();
    assert!(strong.style.strong && !strong.style.emph);
    assert!(
        spans
            .iter()
            .find(|s| s.text == "기울임")
            .unwrap()
            .style
            .emph
    );
    assert!(
        spans
            .iter()
            .find(|s| s.text == "취소선")
            .unwrap()
            .style
            .strike
    );
    assert_eq!(
        spans
            .iter()
            .find(|s| s.kind == SpanKind::Code)
            .unwrap()
            .text,
        "인라인 코드"
    );

    let BlockKind::Paragraph(spans) = &doc.blocks[8].kind else {
        panic!("문단이 아님");
    };
    let both = &spans[0];
    assert_eq!(both.text, "굵은 기울임");
    assert!(both.style.strong && both.style.emph);
    let link = spans.iter().find(|s| s.text == "링크").unwrap();
    assert_eq!(link.link.as_deref(), Some("https://example.com"));
    let auto = spans
        .iter()
        .find(|s| s.text == "https://auto.example.com")
        .unwrap();
    assert_eq!(auto.link.as_deref(), Some("https://auto.example.com"));

    let table = doc
        .blocks
        .iter()
        .find_map(|b| match &b.kind {
            BlockKind::Table { head, rows, .. } => Some((head, rows)),
            _ => None,
        })
        .unwrap();
    assert_eq!(plain_text(&table.0[1]), "가운데");
    assert!(table.1[0][1][0].style.strong);
    assert_eq!(table.1[0][2][0].kind, SpanKind::Code);
    assert_eq!(plain_text(&table.1[1][0]), "한글");

    let BlockKind::FrontMatter(yaml) = &doc.blocks[0].kind else {
        panic!("Front Matter가 아님");
    };
    assert_eq!(yaml, "title: GFM 전체 요소\ntags: [markdown, test]");

    // 각주 번호는 본문에서 처음 참조한 순서
    assert_eq!(doc.footnotes, ["1", "note"]);
    assert_eq!(doc.footnote_number("note"), Some(2));
    assert_eq!(doc.footnote_number("없음"), None);
}

#[test]
fn heading_slugs_match_github() {
    let doc = parse(&fixture("korean.md"));
    let slugs: Vec<&str> = doc.anchors.iter().map(|a| a.slug.as_str()).collect();
    assert_eq!(
        slugs,
        [
            "한글-markdown-문서",
            "38-markdown-소스-보기--미리보기-핵심-기능",
            "목록과-강조",
            "표",
            "faq-자주-묻는-질문",
            "제목-괄호--기호",
            "목록과-강조-1",
        ]
    );
    assert_eq!(doc.anchor_line("목록과-강조-1"), Some(31));
    assert_eq!(doc.anchor_line("없는-제목"), None);

    // PLAN.md 자체의 3.8절 제목
    let plan = parse("## 3.8 Markdown 소스 보기\n");
    assert_eq!(plan.anchors[0].slug, "38-markdown-소스-보기");
}

#[test]
fn cache_reparses_only_when_revision_changes() {
    let mut cache = MarkdownCache::default();
    let text = fixture("gfm_all.md");
    let first = cache.get(1, &text);
    let again = cache.get(1, &text);
    assert_eq!(cache.parse_count(), 1);
    assert!(std::sync::Arc::ptr_eq(&first, &again));

    let edited = format!("{text}\n추가 문단\n");
    let next = cache.get(2, &edited);
    assert_eq!(cache.parse_count(), 2);
    assert_eq!(next.blocks.len(), first.blocks.len() + 1);

    // 다시 불러오기로 버전이 초기화되어도 내용 길이가 다르면 다시 파싱한다.
    cache.get(2, &text);
    assert_eq!(cache.parse_count(), 3);

    cache.invalidate();
    cache.get(2, &text);
    assert_eq!(cache.parse_count(), 4);
}

/// 1MB 문서 파싱 시간 측정 — `cargo test --release --test markdown -- --ignored --nocapture`
#[test]
#[ignore = "측정용: 릴리스 빌드로 실행"]
fn parse_time_1mb() {
    let unit = format!("{}\n{}\n", fixture("gfm_all.md"), fixture("korean.md"));
    // Front Matter는 문서 맨 앞에만 유효하므로 반복분은 그대로 둬도 일반 블록으로 파싱된다.
    let mut text = String::with_capacity(1 << 20);
    while text.len() < 1 << 20 {
        text.push_str(&unit);
    }
    let runs = 10;
    let mut times = Vec::with_capacity(runs);
    let mut blocks = 0;
    for _ in 0..runs {
        let start = Instant::now();
        let doc = parse(&text);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        blocks = doc.blocks.len();
    }
    let avg = times.iter().sum::<f64>() / runs as f64;
    let max = times.iter().cloned().fold(0.0, f64::max);
    println!(
        "1MB Markdown 파싱: {} B, 최상위 블록 {blocks}개, 평균 {avg:.1}ms, 최대 {max:.1}ms ({runs}회)",
        text.len()
    );
}
