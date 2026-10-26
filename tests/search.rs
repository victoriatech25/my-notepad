//! P4-01 검색 엔진·P4-02 치환 엔진 통합 테스트 (PLAN.md §3.2-2).
//!
//! 검증 항목 (WORKPLAN.md P4-01):
//! - 옵션 조합(정규식·대소문자·단어 단위·Wrap Around)별 매치
//! - 한글·이모지·결합 문자 포함 문서의 바이트↔문자 오프셋 정확성
//! - 잘못된 정규식 → 오류 메시지
//! - 10MB 파일 매치 카운트 ≤ 200ms (측정용, 릴리스 빌드)
//!
//! 검증 항목 (WORKPLAN.md P4-02):
//! - 캡처 그룹(`$1`·`${name}`)·이스케이프(`\n`·`\t`)·빈 매치(`^`·`$`) 치환
//! - 줄바꿈 정규화(`LF` 내부 표현)와의 상호작용

use std::ops::Range;
use std::time::Instant;

use my_notepad::core::line_ending::{LineEnding, apply_line_ending, normalize_to_lf};
use my_notepad::core::search::{
    ReplaceAll, SearchError, SearchOptions, SearchQuery, byte_ranges_to_char_ranges,
    char_range_to_byte_range, next_match_index, prev_match_index,
};

fn opts(regex: bool, match_case: bool, whole_word: bool) -> SearchOptions {
    SearchOptions {
        regex,
        match_case,
        whole_word,
        ..Default::default()
    }
}

fn query(pattern: &str, options: SearchOptions) -> SearchQuery {
    SearchQuery::new(pattern, &options).expect("올바른 검색어")
}

/// 매치된 원문 조각들
fn found<'t>(text: &'t str, pattern: &str, options: SearchOptions) -> Vec<&'t str> {
    query(pattern, options)
        .find_all(text)
        .into_iter()
        .map(|r| &text[r])
        .collect()
}

/// 문자 인덱스 범위로 자른 조각 (egui 커서 기준으로 보이는 텍스트)
fn char_slice(text: &str, range: Range<usize>) -> String {
    text.chars()
        .skip(range.start)
        .take(range.end - range.start)
        .collect()
}

#[test]
fn default_options() {
    let o = SearchOptions::default();
    assert!(!o.regex && !o.match_case && !o.whole_word);
    assert!(o.wrap_around, "Wrap Around는 기본 켬 (PLAN.md §3.2-2)");
}

#[test]
fn plain_text_option_combinations() {
    let text = "Error error ERRORS error_code error. (error)";
    // 대소문자 무시 + 부분 일치 (기본)
    assert_eq!(found(text, "error", opts(false, false, false)).len(), 6);
    // 대소문자 구분 + 부분 일치
    assert_eq!(found(text, "error", opts(false, true, false)).len(), 4);
    // 대소문자 무시 + 단어 단위: `ERRORS`·`error_code` 제외
    assert_eq!(
        found(text, "error", opts(false, false, true)),
        ["Error", "error", "error", "error"]
    );
    // 대소문자 구분 + 단어 단위
    assert_eq!(found(text, "Error", opts(false, true, true)), ["Error"]);
}

#[test]
fn plain_mode_treats_regex_metacharacters_literally() {
    let text = "a.b axb (x) [y] $1 \\d";
    assert_eq!(found(text, "a.b", opts(false, true, false)), ["a.b"]);
    assert_eq!(found(text, "(x)", opts(false, true, false)), ["(x)"]);
    assert_eq!(found(text, "[y]", opts(false, true, false)), ["[y]"]);
    assert_eq!(found(text, "$1", opts(false, true, false)), ["$1"]);
    assert_eq!(found(text, "\\d", opts(false, true, false)), ["\\d"]);
    // 정규식 모드에서는 메타 문자로 동작
    assert_eq!(found(text, "a.b", opts(true, true, false)), ["a.b", "axb"]);
}

#[test]
fn regex_option_combinations() {
    let text = "id=12 ID=345 x99 7_8 id=6";
    assert_eq!(
        found(text, r"id=\d+", opts(true, true, false)),
        ["id=12", "id=6"]
    );
    assert_eq!(
        found(text, r"id=\d+", opts(true, false, false)),
        ["id=12", "ID=345", "id=6"]
    );
    // 단어 단위: `x99`의 `99`, `7_8`의 숫자는 단어 중간
    assert_eq!(
        found(text, r"\d+", opts(true, true, true)),
        ["12", "345", "6"]
    );
    assert_eq!(found(text, r"\d+", opts(true, true, false)).len(), 6);
    // 정규식 안의 인라인 플래그는 옵션보다 우선
    assert_eq!(found(text, r"(?i)ID", opts(true, true, false)).len(), 3);
}

#[test]
fn whole_word_retries_inside_rejected_word() {
    // 경계 실패 뒤 다음 글자부터 다시 찾는다 (겹치는 위치도 놓치지 않음)
    assert_eq!(found("aaa aa", "aa", opts(false, true, true)), ["aa"]);
    assert_eq!(found("xab ab", "ab", opts(false, true, true)), ["ab"]);
    assert_eq!(found("aaaa", "aa", opts(false, true, false)), ["aa", "aa"]);
    // 끝 글자가 단어 글자가 아니면 그쪽 경계는 보지 않는다 (스마트 하이라이트와 같은 규칙)
    assert_eq!(found("a->b ->", "->", opts(false, true, true)).len(), 2);
    assert_eq!(
        found("foo() foo()bar xfoo()", "foo()", opts(false, true, true)),
        ["foo()", "foo()"]
    );
}

#[test]
fn korean_whole_word_and_case_folding() {
    let text = "검색 검색어 검색을 다시 검색";
    assert_eq!(
        found(text, "검색", opts(false, false, true)).len(),
        2,
        "조사가 붙은 `검색어`·`검색을`은 단어 단위에서 제외"
    );
    assert_eq!(found(text, "검색", opts(false, false, false)).len(), 4);

    // 대소문자 무시는 유니코드 대소문자 접기: 켈빈 기호 `K`(3바이트)도 `k`로 찾는다
    let text = "İx Kelvin kelvin";
    let q = query("kelvin", opts(false, false, true));
    let ranges = q.find_all(text);
    assert_eq!(ranges.len(), 2);
    assert_eq!(&text[ranges[0].clone()], "Kelvin");
    assert_eq!(&text[ranges[1].clone()], "kelvin");
}

#[test]
fn multiline_anchors_and_empty_matches() {
    let text = "a1\nb2\n\nc3";
    // `^`·`$`는 줄 단위
    assert_eq!(
        found(text, r"^\w", opts(true, true, false)),
        ["a", "b", "c"]
    );
    assert_eq!(
        found(text, r"\d$", opts(true, true, false)),
        ["1", "2", "3"]
    );
    // 빈 매치: 줄 시작마다 하나 (빈 줄 포함), 무한 반복 없음
    let starts: Vec<usize> = query("^", opts(true, true, false))
        .find_all(text)
        .into_iter()
        .map(|r| r.start)
        .collect();
    assert_eq!(starts, [0, 3, 6, 7]);
    // 비어 있지 않은 매치 바로 뒤의 빈 매치는 내지 않는다
    assert_eq!(
        query("a*", opts(true, true, false)).find_all("aab"),
        [0..2, 3..3]
    );
    // 빈 매치는 글자 단위로 진행 (한글 글자 중간에서 멈추지 않음)
    let text = "가나";
    assert_eq!(
        query("", opts(true, true, false)).count(text),
        0,
        "빈 검색어는 매치 없음"
    );
    let empties = query("x*", opts(true, true, false)).find_all(text);
    assert_eq!(empties, [0..0, 3..3, 6..6]);
}

#[test]
fn offsets_with_korean_emoji_and_combining_marks() {
    // 👍🏽 = 2글자(8바이트), 👨‍👩‍👧 = 5글자(18바이트, ZWJ 연결), é = e + U+0301 결합 악센트
    let text = "👍🏽 좋아요\n👨‍👩‍👧 가족 좋아요\ncafe\u{301} cafe 좋아요!";
    for (pattern, options) in [
        ("좋아요", opts(false, false, false)),
        ("👨‍👩‍👧", opts(false, true, false)),
        ("cafe", opts(false, false, false)),
        (r"\w+", opts(true, true, false)),
        (r".", opts(true, true, false)),
        (r"(?m)^.", opts(true, true, false)),
    ] {
        let q = query(pattern, options);
        let bytes = q.find_all(text);
        assert!(!bytes.is_empty(), "{pattern}");
        let chars = byte_ranges_to_char_ranges(text, &bytes);
        assert_eq!(chars.len(), bytes.len());
        for (b, c) in bytes.iter().zip(&chars) {
            // 문자 인덱스로 잘라도 같은 텍스트, 다시 바이트로 바꾸면 같은 범위
            assert_eq!(char_slice(text, c.clone()), &text[b.clone()], "{pattern}");
            assert_eq!(char_range_to_byte_range(text, c.clone()), *b, "{pattern}");
        }
    }

    // 위치를 직접 확인: 줄마다 `좋아요`의 문자 인덱스
    let bytes = query("좋아요", SearchOptions::default()).find_all(text);
    let chars = byte_ranges_to_char_ranges(text, &bytes);
    // "👍🏽 " = 3글자 → 3..6 · "\n👨‍👩‍👧 가족 " = 1+5+1+2+1 = 10글자 → 16..19
    // · "\ncafe\u{301} cafe " = 1+5+1+4+1 = 12글자 → 31..34
    assert_eq!(chars, [3..6, 16..19, 31..34]);
    assert_eq!(bytes[0], 9..18);
}

#[test]
fn whole_word_treats_combining_mark_as_word_char() {
    // 결합 악센트가 뒤따르는 `cafe`는 단어 중간 (정규식 `\w`와 같은 판정)
    let text = "cafe\u{301} cafe";
    let r = query("cafe", opts(false, false, true)).find_all(text);
    assert_eq!(r, vec![7..11]);
    assert_eq!(byte_ranges_to_char_ranges(text, &r), vec![6..10]);
    assert_eq!(found(text, "cafe", opts(false, false, false)).len(), 2);
}

#[test]
fn offset_conversion_handles_unsorted_and_out_of_range() {
    let text = "가a나";
    // 순서가 뒤바뀐 목록도 정확히 변환
    assert_eq!(
        byte_ranges_to_char_ranges(text, &[4..7, 0..3, 3..4]),
        [2..3, 0..1, 1..2]
    );
    // 글자 중간·텍스트 밖 위치는 앞쪽 글자 경계로 보정 (편집 직후 오래된 결과)
    assert_eq!(
        byte_ranges_to_char_ranges(text, &[1..5, 6..99]),
        [0..2, 2..3]
    );
    assert_eq!(char_range_to_byte_range(text, 1..10), 3..7);
    assert_eq!(char_range_to_byte_range(text, 5..6), 7..7);
    assert_eq!(char_range_to_byte_range("", 0..0), 0..0);
}

#[test]
fn invalid_regex_returns_error_message() {
    for pattern in ["(", "[a-", "a{2,1}", r"\"] {
        let err = SearchQuery::new(pattern, &opts(true, true, false)).unwrap_err();
        let SearchError::Syntax(detail) = &err else {
            panic!("{pattern}: 문법 오류여야 함 — {err:?}");
        };
        assert!(!detail.is_empty());
        assert!(err.to_string().starts_with("정규식 오류"), "{err}");
    }
    // 지원하지 않는 전후방 탐색·역참조도 오류로 알린다
    for pattern in ["a(?=b)", "(?<!a)b", r"(a)\1"] {
        assert!(
            matches!(
                SearchQuery::new(pattern, &opts(true, true, false)),
                Err(SearchError::Syntax(_))
            ),
            "{pattern}"
        );
    }
    // 크기 한도 초과
    let err = SearchQuery::new(r"\w{1000}{1000}", &opts(true, true, false)).unwrap_err();
    assert_eq!(err, SearchError::TooBig);
    assert!(!err.to_string().is_empty());
    // 일반 문자열 모드에서는 같은 글자도 그대로 찾는다
    assert_eq!(found("f(x", "(", opts(false, true, false)), ["("]);
}

#[test]
fn empty_pattern_is_not_an_error() {
    let q = SearchQuery::new("", &SearchOptions::default()).unwrap();
    assert!(q.is_empty());
    assert_eq!(q.count("abc"), 0);
    assert!(q.find_all("abc").is_empty());
    assert!(!query("a", SearchOptions::default()).is_empty());
}

#[test]
fn next_and_prev_match_with_wrap_around() {
    let text = "ab ab ab";
    let q = query("ab", SearchOptions::default());
    assert!(q.wrap_around());
    let m = q.find_all(text);
    assert_eq!(m, [0..2, 3..5, 6..8]);

    // 커서 위치(포함) 이후 첫 매치
    assert_eq!(next_match_index(&m, 0, true), Some(0));
    assert_eq!(next_match_index(&m, 1, true), Some(1));
    assert_eq!(next_match_index(&m, 3, true), Some(1));
    // 현재 매치 시작 + 1 → 다음 매치, 끝에서는 처음으로 이어짐
    assert_eq!(next_match_index(&m, 6 + 1, true), Some(0));
    assert_eq!(next_match_index(&m, 7, false), None);

    // 커서 앞 마지막 매치, 처음에서는 끝으로 이어짐
    assert_eq!(prev_match_index(&m, 8, true), Some(2));
    assert_eq!(prev_match_index(&m, 3, true), Some(0));
    assert_eq!(prev_match_index(&m, 0, true), Some(2));
    assert_eq!(prev_match_index(&m, 0, false), None);

    // 매치가 없으면 Wrap Around와 상관없이 없음
    assert_eq!(next_match_index(&[], 0, true), None);
    assert_eq!(prev_match_index(&[], 0, true), None);

    // 빈 매치에서도 제자리에 멈추지 않는다
    let m = query("^", opts(true, true, false)).find_all("a\nb");
    assert_eq!(m, [0..0, 2..2]);
    // 첫 빈 매치(0) 시작 + 1
    assert_eq!(next_match_index(&m, 1, true), Some(1));
}

#[test]
fn matches_iterator_is_lazy() {
    let text = "x ".repeat(1000);
    let q = query("x", SearchOptions::default());
    let first: Vec<_> = q.matches(&text).take(3).collect();
    assert_eq!(first, [0..1, 2..3, 4..5]);
    assert_eq!(q.count(&text), 1000);
}

/// 10MB 문서 (로그 형식 한글·영문 혼합 줄)
fn text_10mb() -> String {
    let mut text = String::with_capacity(10 << 20);
    let mut i = 0u32;
    while text.len() < 10 << 20 {
        let level = ["INFO", "WARN", "ERROR"][(i % 7 % 3) as usize];
        text.push_str(&format!(
            "2026-10-26 12:{:02}:{:02} [{level}] 요청 처리 완료 id={i} user=홍길동 path=/api/v1/items/{} error={}\n",
            i / 60 % 60,
            i % 60,
            i % 997,
            if i.is_multiple_of(13) { "timeout" } else { "none" },
        ));
        i += 1;
    }
    text
}

/// 10MB 매치 카운트 시간 측정 — `cargo test --release --test search -- --ignored --nocapture`
#[test]
#[ignore = "측정용: 릴리스 빌드로 실행"]
fn count_time_10mb() {
    let text = text_10mb();
    let cases = [
        (
            "일반 `error=timeout`",
            "error=timeout",
            opts(false, true, false),
        ),
        (
            "일반 대소문자 무시 `error`",
            "error",
            opts(false, false, false),
        ),
        ("일반 단어 단위 `홍길동`", "홍길동", opts(false, true, true)),
        ("정규식 `id=\\d+`", r"id=\d+", opts(true, true, false)),
        (
            "정규식 `\\[(WARN|ERROR)\\]`",
            r"\[(WARN|ERROR)\]",
            opts(true, true, false),
        ),
        (
            "정규식 단어 단위 `\\d{3}`",
            r"\d{3}",
            opts(true, true, true),
        ),
        ("정규식 `^2026`", "^2026", opts(true, true, false)),
    ];
    let runs = 5;
    let mut worst: f64 = 0.0;
    for (label, pattern, options) in cases {
        let q = query(pattern, options);
        let mut times = Vec::with_capacity(runs);
        let mut count = 0;
        for _ in 0..runs {
            let start = Instant::now();
            count = q.count(&text);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let avg = times.iter().sum::<f64>() / runs as f64;
        let max = times.iter().cloned().fold(0.0, f64::max);
        worst = worst.max(max);
        println!("{label}: 매치 {count}개, 평균 {avg:.1}ms, 최대 {max:.1}ms ({runs}회)");
    }
    println!("10MB = {} B, 최대 {worst:.1}ms", text.len());
    assert!(
        worst <= 200.0,
        "10MB 매치 카운트 ≤ 200ms — 최대 {worst:.1}ms"
    );
}

// ── P4-02 치환 엔진 ──

/// 모두 바꾸기 결과 `(텍스트, 개수)`
fn replace(
    text: &str,
    pattern: &str,
    replacement: &str,
    options: SearchOptions,
) -> (String, usize) {
    let ReplaceAll { text, count } = query(pattern, options).replace_all(text, replacement);
    (text, count)
}

#[test]
fn replace_capture_groups() {
    let re = opts(true, true, false);
    assert_eq!(
        replace("10-20, 3-4", r"(\d+)-(\d+)", "$2-$1", re),
        ("20-10, 4-3".to_owned(), 2)
    );
    // 이름 있는 그룹 (`(?P<name>…)`·`(?<name>…)` 둘 다)
    assert_eq!(
        replace(
            "2026-10, 1999-01",
            r"(?P<y>\d{4})-(?<m>\d{2})",
            "${m}/${y}",
            re
        ),
        ("10/2026, 01/1999".to_owned(), 2)
    );
    // `$0`은 매치 전체, `$$`는 `$` 글자
    assert_eq!(
        replace("a1 b22", r"\d+", "$$$0", re),
        ("a$1 b$22".to_owned(), 2)
    );
    // `$1a`는 그룹 이름 `1a`(없음 → 빈 문자열), 뒤에 글자를 붙이려면 `${1}a`
    assert_eq!(replace("b", "(b)", "$1a", re), (String::new(), 1));
    assert_eq!(replace("b", "(b)", "${1}a", re), ("ba".to_owned(), 1));
    // 매치되지 않은 선택 그룹은 빈 문자열
    assert_eq!(replace("ac", "a(b)?c", "[$1]", re), ("[]".to_owned(), 1));
    // 한글 그룹
    assert_eq!(
        replace("홍길동 님, 김철수 님", r"(\S+) 님", "$1 씨", re),
        ("홍길동 씨, 김철수 씨".to_owned(), 2)
    );
}

#[test]
fn replace_escapes_in_regex_mode() {
    let re = opts(true, true, false);
    assert_eq!(
        replace("a, b, c", ", ", r"\n", re),
        ("a\nb\nc".to_owned(), 2)
    );
    assert_eq!(replace("a b", " ", r"\t", re), ("a\tb".to_owned(), 1));
    assert_eq!(replace("a b", " ", r"\\", re), ("a\\b".to_owned(), 1));
    // 이스케이프와 그룹을 함께
    assert_eq!(
        replace("k=v", r"(\w)=(\w)", r"$1\t$2\n", re),
        ("k\tv\n".to_owned(), 1)
    );
    // `\\n`은 백슬래시 + n, 그 밖의 `\x`와 끝의 `\`는 그대로
    assert_eq!(replace("a", "a", r"\\n", re), ("\\n".to_owned(), 1));
    assert_eq!(replace("a", "a", r"\d\", re), ("\\d\\".to_owned(), 1));
}

#[test]
fn replace_is_literal_in_plain_mode() {
    let plain = opts(false, true, false);
    // 일반 모드에서는 `$1`·`\n`을 해석하지 않는다
    assert_eq!(
        replace("price: 10", "10", r"$1\n${x}", plain),
        (r"price: $1\n${x}".to_owned(), 1)
    );
    assert_eq!(
        replace("a.b a.b", "a.b", "(x)", plain),
        ("(x) (x)".to_owned(), 2)
    );
}

#[test]
fn replace_follows_search_options() {
    // 단어 단위: `category`·`cats`는 그대로
    assert_eq!(
        replace(
            "cat category cats cat.",
            "cat",
            "dog",
            opts(false, true, true)
        ),
        ("dog category cats dog.".to_owned(), 2)
    );
    // 대소문자 무시
    assert_eq!(
        replace(
            "Error error ERROR",
            "error",
            "ok",
            opts(false, false, false)
        ),
        ("ok ok ok".to_owned(), 3)
    );
    // 개수는 찾기 카운트와 같다
    let text = "검색 검색어 검색을 다시 검색";
    let q = query("검색", opts(false, true, true));
    assert_eq!(q.replace_all(text, "찾기").count, q.count(text));
    assert_eq!(
        q.replace_all(text, "찾기").text,
        "찾기 검색어 검색을 다시 찾기"
    );
}

#[test]
fn replace_empty_matches() {
    let re = opts(true, true, false);
    // `^`: 줄마다 앞에 붙인다 — 끝 개행 뒤의 빈 줄도 한 줄이다 (에디터에서 커서가 갈 수 있는 줄)
    assert_eq!(
        replace("a\nb\n", "^", "> ", re),
        ("> a\n> b\n> ".to_owned(), 3)
    );
    assert_eq!(replace("a\nb", "^", "> ", re), ("> a\n> b".to_owned(), 2));
    // `$`: 줄마다 끝에 붙인다
    assert_eq!(replace("a\nb", "$", ";", re), ("a;\nb;".to_owned(), 2));
    // 빈 줄 지우기 (`^\n`)
    assert_eq!(
        replace("a\n\n\nb\n", r"^\n", "", re),
        ("a\nb\n".to_owned(), 2)
    );
    // 글자 사이 빈 매치: 글자 단위로 진행하고 한글 글자 중간에 끼우지 않는다
    assert_eq!(replace("가나", "x*", "-", re), ("-가-나-".to_owned(), 3));
    // 비어 있지 않은 매치 바로 뒤의 빈 매치는 바꾸지 않는다 (찾기와 같은 규칙)
    assert_eq!(replace("baac", "a*", "-", re), ("-b-c-".to_owned(), 3));
}

#[test]
fn replace_interacts_with_lf_normalization() {
    // CRLF 파일은 열 때 `LF`로 정규화된다 → 정규식 `\n`으로 줄을 합치고, `\r\n`은 매치되지 않는다
    let text = normalize_to_lf("one\r\ntwo\r\nthree\r\n");
    let re = opts(true, true, false);
    assert_eq!(
        replace(&text, r"\n(\w)", " $1", re),
        ("one two three\n".to_owned(), 2)
    );
    assert_eq!(replace(&text, r"\r\n", " ", re).1, 0);
    assert_eq!(replace(&text, r"\r", " ", re).1, 0);

    // 바꿀 문자열 안의 CRLF·CR(붙여넣기 등)도 `LF`로 넣는다 — 일반·정규식 모드 모두
    let (plain, _) = replace("a|b", "|", "\r\n", opts(false, true, false));
    assert_eq!(plain, "a\nb");
    let (regex, _) = replace("a|b", r"\|", "x\ry", re);
    assert_eq!(regex, "ax\nyb");
    assert!(!plain.contains('\r') && !regex.contains('\r'));

    // 줄을 나눈 결과를 CRLF 문서로 저장하면 새 줄도 CRLF가 된다
    let (split, _) = replace(&text, "two", r"two\nhalf", re);
    assert_eq!(
        apply_line_ending(&split, LineEnding::CrLf),
        "one\r\ntwo\r\nhalf\r\nthree\r\n"
    );
}

#[test]
fn replace_without_matches_keeps_text() {
    assert_eq!(
        replace("abc", "zz", "x", SearchOptions::default()),
        ("abc".to_owned(), 0)
    );
    let empty = SearchQuery::new("", &SearchOptions::default()).unwrap();
    assert_eq!(
        empty.replace_all("abc", "x"),
        ReplaceAll {
            text: "abc".to_owned(),
            count: 0
        }
    );
}

#[test]
fn replacement_for_single_match() {
    let text = "ab ab xab";
    let q = query("ab", opts(false, true, true));
    assert_eq!(q.replacement_for(text, &(3..5), "X").as_deref(), Some("X"));
    // 매치가 아닌 범위·단어 중간·텍스트 밖·글자 중간은 `None` (오래된 매치를 바꾸지 않게)
    assert_eq!(q.replacement_for(text, &(1..3), "X"), None);
    assert_eq!(
        q.replacement_for(text, &(7..9), "X"),
        None,
        "단어 중간 `xab`"
    );
    assert_eq!(q.replacement_for(text, &(8..20), "X"), None);
    assert_eq!(
        query("가", SearchOptions::default()).replacement_for("가", &(1..3), "X"),
        None
    );

    // 정규식 그룹·이스케이프
    let q = query(r"(\w)(\w)", opts(true, true, false));
    assert_eq!(
        q.replacement_for("ab cd", &(3..5), r"$2$1\t").as_deref(),
        Some("dc\t")
    );
    // `^`처럼 앞 문맥이 필요한 매치도 같은 위치에서 판정
    let q = query(r"^\w", opts(true, true, false));
    assert_eq!(
        q.replacement_for("a\nb", &(2..3), "X").as_deref(),
        Some("X")
    );
    assert_eq!(q.replacement_for("ab", &(1..2), "X"), None);
}
