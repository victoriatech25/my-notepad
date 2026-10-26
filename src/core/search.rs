//! 찾기 바의 검색 엔진: 일반 문자열·정규식 검색, 매치 이동, 바이트↔문자 오프셋 변환 (PLAN.md §3.2-2, P4-01).
//!
//! - 일반 문자열도 [`regex::escape`]로 정규식으로 바꿔 같은 경로로 찾는다 (옵션 처리를 한 곳에 모은다).
//! - 정규식 문법은 Rust `regex`를 따른다 — **전후방 탐색(lookaround)과 역참조는 지원하지 않는다**.
//!   `^`·`$`는 줄 단위(multi-line)로 동작한다. 잘못된 패턴은 [`SearchError`]로 알린다.
//! - 대소문자 무시는 `regex`의 유니코드 대소문자 접기를 쓴다. 위치는 항상 원문 바이트 기준이다.
//! - 단어 단위 일치: 매치의 첫 글자가 단어 글자면 앞에, 끝 글자가 단어 글자면 뒤에 단어 경계가 있어야 한다
//!   (스마트 하이라이트와 같은 판정 방식). 단어 글자는 정규식 `\w`와 같아 결합 문자도 포함한다 —
//!   `e` + 결합 악센트 앞의 `cafe`는 단어 중간으로 본다. 경계가 맞지 않으면 다음 글자부터 다시 찾는다.
//! - 위치는 문서 텍스트(`LF` 정규화)의 바이트 범위이며 오름차순·겹침 없음. egui 커서는 문자 인덱스를 쓰므로
//!   UI는 [`byte_ranges_to_char_ranges`]·[`char_range_to_byte_range`]로 변환한다
//!   (낱개 오프셋은 [`crate::core::buffer`]의 `byte_to_char_offset`·`char_to_byte_offset`).
//! - 치환(P4-02): **정규식 모드**에서만 바꿀 문자열의 `$1`·`${name}`(캡처 그룹, `$$`는 `$` 글자)과
//!   `\n`·`\t`·`\\` 이스케이프를 해석한다. 그 밖의 `\x`는 그대로 둔다. `$1a`는 `regex` 규칙대로 그룹 이름 `1a`로
//!   읽히므로 뒤에 글자를 붙이려면 `${1}a`로 쓴다. **일반 모드**의 바꿀 문자열은 글자 그대로 넣는다 (VS Code·Notepad++
//!   기본 모드와 같음). 어느 모드든 바꿀 문자열의 `CRLF`·`CR`은 `LF`로 바꾼다 (문서 내부 표현, 저장할 때 문서 줄바꿈으로).
//!   모두 바꾸기는 [`SearchQuery::matches`]와 같은 매치(단어 단위·빈 매치 규칙 포함)를 바꾼다.
//! - 백그라운드 검색(P4-03): [`SearchJob`]이 작업 스레드에서 문서 사본을 찾고 결과를 채널로 넘긴다.
//!   작업을 버리면(새 검색어로 바꾸면) 취소 표시를 세워 스레드가 곧 멈추고 결과도 보내지 않는다.
//!   어떤 조건으로 찾은 결과인지 확인하고 오래된 결과를 버리는 일은 작업을 가진 쪽(UI)이 맡는다.

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use regex::{Regex, RegexBuilder};

use crate::core::line_ending::normalize_to_lf;

/// 찾기 옵션 (찾기 바의 `.*`·`Aa`·`\b` 토글과 Wrap Around).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    /// 정규식 사용 (`.*`)
    pub regex: bool,
    /// 대소문자 구분 (`Aa`)
    pub match_case: bool,
    /// 단어 단위 일치 (`\b`)
    pub whole_word: bool,
    /// 끝에서 처음으로 이어서 찾기 (기본 켬)
    pub wrap_around: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            regex: false,
            match_case: false,
            whole_word: false,
            wrap_around: true,
        }
    }
}

/// 검색어를 검색 조건으로 만들 수 없는 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    /// 정규식 문법 오류 (`regex`가 알려 준 위치·내용)
    Syntax(String),
    /// 컴파일된 정규식이 크기 한도를 넘음 (큰 반복 횟수 등)
    TooBig,
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(detail) => write!(f, "정규식 오류:\n{detail}"),
            Self::TooBig => f.write_str("정규식이 너무 큽니다. 반복 횟수나 길이를 줄여 주세요."),
        }
    }
}

impl std::error::Error for SearchError {}

impl From<regex::Error> for SearchError {
    fn from(err: regex::Error) -> Self {
        match err {
            regex::Error::CompiledTooBig(_) => Self::TooBig,
            other => Self::Syntax(other.to_string()),
        }
    }
}

/// 검색어와 옵션으로 만든 검색 조건. 같은 조건으로 여러 번 찾을 때 다시 컴파일하지 않도록 보관한다.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// 빈 검색어면 `None` (매치 없음, 오류도 아님)
    regex: Option<Regex>,
    /// 정규식 모드 (바꿀 문자열의 그룹·이스케이프 해석 여부)
    regex_mode: bool,
    whole_word: bool,
    wrap_around: bool,
}

/// 모두 바꾸기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceAll {
    /// 바꾼 뒤 전체 텍스트 (`LF` 정규화)
    pub text: String,
    /// 바꾼 매치 수
    pub count: usize,
}

impl SearchQuery {
    /// 검색어를 컴파일한다. 빈 검색어는 오류가 아니라 매치가 없는 조건이 된다.
    pub fn new(pattern: &str, options: &SearchOptions) -> Result<Self, SearchError> {
        let regex = if pattern.is_empty() {
            None
        } else {
            let source = if options.regex {
                Cow::Borrowed(pattern)
            } else {
                Cow::Owned(regex::escape(pattern))
            };
            Some(
                RegexBuilder::new(&source)
                    .case_insensitive(!options.match_case)
                    .multi_line(true)
                    .build()?,
            )
        };
        Ok(Self {
            regex,
            regex_mode: options.regex,
            whole_word: options.whole_word,
            wrap_around: options.wrap_around,
        })
    }

    /// 빈 검색어인지 여부.
    pub fn is_empty(&self) -> bool {
        self.regex.is_none()
    }

    /// Wrap Around 옵션.
    pub fn wrap_around(&self) -> bool {
        self.wrap_around
    }

    /// 매치 위치를 앞에서부터 차례로 돌려준다 (필요한 만큼만 `take`할 수 있다).
    pub fn matches<'a>(&'a self, text: &'a str) -> impl Iterator<Item = Range<usize>> + 'a {
        Matches {
            regex: self.regex.as_ref(),
            text,
            whole_word: self.whole_word,
            pos: 0,
            last_end: None,
        }
    }

    /// 모든 매치 위치.
    pub fn find_all(&self, text: &str) -> Vec<Range<usize>> {
        self.matches(text).collect()
    }

    /// 매치 수 (위치 목록을 만들지 않는다).
    pub fn count(&self, text: &str) -> usize {
        self.matches(text).count()
    }

    /// 모든 매치를 `replacement`로 바꾼 텍스트와 바꾼 개수. 매치가 없으면 원문 그대로, 개수 0.
    pub fn replace_all(&self, text: &str, replacement: &str) -> ReplaceAll {
        let Some(regex) = &self.regex else {
            return ReplaceAll {
                text: text.to_owned(),
                count: 0,
            };
        };
        let template = self.template(replacement);
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut count = 0;
        for range in self.matches(text) {
            out.push_str(&text[last..range.start]);
            expand_into(regex, text, &range, &template, &mut out);
            last = range.end;
            count += 1;
        }
        out.push_str(&text[last..]);
        ReplaceAll { text: out, count }
    }

    /// 바이트 범위 `range`의 매치를 바꿀 문자열 (`바꾸기` 한 번).
    ///
    /// 지금 텍스트에서 그 범위가 이 조건의 매치가 아니면 `None` — 편집 뒤 오래된 매치를 바꾸지 않게 한다.
    pub fn replacement_for(
        &self,
        text: &str,
        range: &Range<usize>,
        replacement: &str,
    ) -> Option<String> {
        let regex = self.regex.as_ref()?;
        if range.end > text.len() || !text.is_char_boundary(range.start) {
            return None;
        }
        let found = regex.find_at(text, range.start)?.range();
        if found != *range || (self.whole_word && !has_word_bounds(text, range)) {
            return None;
        }
        let mut out = String::new();
        expand_into(regex, text, range, &self.template(replacement), &mut out);
        Some(out)
    }

    /// 바꿀 문자열을 이 조건에 맞게 준비한다 (줄바꿈 `LF` 정규화, 정규식 모드면 이스케이프 해석).
    fn template(&self, raw: &str) -> Template {
        let raw = normalize_to_lf(raw);
        if self.regex_mode {
            Template::Expand(unescape(&raw))
        } else {
            Template::Literal(raw)
        }
    }
}

/// 준비된 바꿀 문자열.
enum Template {
    /// 글자 그대로 넣는다 (일반 모드)
    Literal(String),
    /// `$1`·`${name}`을 캡처 그룹으로 펼친다 (정규식 모드, 이스케이프는 이미 해석됨)
    Expand(String),
}

/// `range` 매치의 바꿀 문자열을 `out`에 붙인다.
fn expand_into(
    regex: &Regex,
    text: &str,
    range: &Range<usize>,
    template: &Template,
    out: &mut String,
) {
    match template {
        Template::Literal(s) => out.push_str(s),
        Template::Expand(t) => {
            // 같은 위치에서 다시 찾으면 같은 매치가 나온다 (`find_at`처럼 앞 글자 문맥도 본다).
            match regex.captures_at(text, range.start) {
                Some(caps) if caps.get(0).is_some_and(|m| m.range() == *range) => {
                    caps.expand(t, out);
                }
                // 일어나지 않아야 하지만, 그룹 없이 글자 그대로 넣는다.
                _ => out.push_str(t),
            }
        }
    }
}

/// 정규식 모드 바꿀 문자열의 이스케이프 `\n`·`\t`·`\\`를 해석한다. 그 밖의 `\x`는 그대로 둔다.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// [`SearchQuery::matches`]의 반복자.
struct Matches<'a> {
    regex: Option<&'a Regex>,
    text: &'a str,
    whole_word: bool,
    /// 다음 탐색 시작 바이트 위치 (텍스트 길이를 넘으면 끝)
    pos: usize,
    /// 직전 매치의 끝 위치
    last_end: Option<usize>,
}

impl Iterator for Matches<'_> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Range<usize>> {
        let regex = self.regex?;
        while self.pos <= self.text.len() {
            // `find_at`은 시작 위치 앞 글자도 보므로 `^`·`\b`가 문서 전체 기준으로 판정된다.
            let range = regex.find_at(self.text, self.pos)?.range();
            // 직전 매치 끝에 붙은 빈 매치는 건너뛴다 (`a*`가 `aaa` 뒤에서 빈 매치를 한 번 더 내지 않게).
            let touches_last = range.is_empty() && self.last_end == Some(range.start);
            if touches_last || (self.whole_word && !has_word_bounds(self.text, &range)) {
                self.pos = next_char_end(self.text, range.start);
                continue;
            }
            self.pos = if range.is_empty() {
                next_char_end(self.text, range.end)
            } else {
                range.end
            };
            self.last_end = Some(range.end);
            return Some(range);
        }
        None
    }
}

/// `pos`의 다음 글자 끝 위치. 텍스트 끝이면 길이 + 1 (탐색 종료 표시).
fn next_char_end(text: &str, pos: usize) -> usize {
    text[pos..]
        .chars()
        .next()
        .map_or(text.len() + 1, |c| pos + c.len_utf8())
}

/// 단어 단위 일치 경계 판정: 매치 양 끝 중 단어 글자인 쪽만 바깥 글자가 단어 글자가 아니어야 한다.
fn has_word_bounds(text: &str, range: &Range<usize>) -> bool {
    let matched = &text[range.clone()];
    let start_ok = !matched.chars().next().is_some_and(is_word_char)
        || !text[..range.start]
            .chars()
            .next_back()
            .is_some_and(is_word_char);
    let end_ok = !matched.chars().next_back().is_some_and(is_word_char)
        || !text[range.end..].chars().next().is_some_and(is_word_char);
    start_ok && end_ok
}

/// 단어 글자: 정규식 `\w`와 같다 (문자·숫자·결합 문자·`_` 등, 한글 포함).
fn is_word_char(c: char) -> bool {
    regex_syntax::is_word_character(c)
}

/// 바이트 위치 `from`(포함) 이후 첫 매치의 번호. 없으면 `wrap_around`일 때 첫 매치.
///
/// `matches`는 오름차순 매치 목록이다. 현재 매치에서 다음 매치로 넘어가려면 `from`에
/// 현재 매치 시작 + 1을 넘긴다 (빈 매치에서도 제자리에 멈추지 않는다).
pub fn next_match_index(matches: &[Range<usize>], from: usize, wrap_around: bool) -> Option<usize> {
    let i = matches.partition_point(|m| m.start < from);
    if i < matches.len() {
        Some(i)
    } else if wrap_around && !matches.is_empty() {
        Some(0)
    } else {
        None
    }
}

/// 바이트 위치 `before`보다 앞에서 시작하는 마지막 매치의 번호. 없으면 `wrap_around`일 때 마지막 매치.
pub fn prev_match_index(
    matches: &[Range<usize>],
    before: usize,
    wrap_around: bool,
) -> Option<usize> {
    let i = matches.partition_point(|m| m.start < before);
    if i > 0 {
        Some(i - 1)
    } else if wrap_around {
        matches.len().checked_sub(1)
    } else {
        None
    }
}

/// 선택 범위와 똑같은 매치(현재 매치)의 번호. 선택이 매치가 아니면 `None` (카운트는 `? / N`).
pub fn current_match_index(matches: &[Range<usize>], selection: &Range<usize>) -> Option<usize> {
    let i = matches.partition_point(|m| m.start < selection.start);
    (matches.get(i) == Some(selection)).then_some(i)
}

/// 취소 표시를 확인하는 매치 간격 (매치마다 원자 변수를 읽지 않게).
const CANCEL_CHECK_EVERY: usize = 1024;

/// [`SearchJob`] 결과 위치의 단위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeUnit {
    /// 바이트 범위 (읽기 전용 뷰어)
    Bytes,
    /// 문자 인덱스 범위 (편집 위젯 커서 단위)
    Chars,
}

/// 백그라운드 검색 상태.
#[derive(Debug, PartialEq, Eq)]
pub enum JobPoll {
    /// 아직 찾는 중
    Running,
    /// 다 찾았다 (매치 범위, 요청한 단위)
    Done(Vec<Range<usize>>),
    /// 스레드가 결과 없이 끝났다 (취소·패닉)
    Lost,
}

/// 작업 스레드에서 도는 검색 하나. 버리면 취소된다.
#[derive(Debug)]
pub struct SearchJob {
    cancel: Arc<AtomicBool>,
    rx: Receiver<Vec<Range<usize>>>,
}

impl SearchJob {
    /// `text` 사본에서 `query`로 모든 매치를 찾는 스레드를 띄운다. 다 찾으면 결과를 보낸 뒤 `notify`를 부른다
    /// (UI 다시 그리기 요청). 결과는 `unit` 단위로 바꿔 보낸다.
    pub fn spawn(
        query: SearchQuery,
        text: String,
        unit: RangeUnit,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let flag = Arc::clone(&cancel);
        std::thread::Builder::new()
            .name("search".to_owned())
            .spawn(move || {
                let Some(ranges) = find_all_cancellable(&query, &text, &flag) else {
                    return;
                };
                let ranges = match unit {
                    RangeUnit::Bytes => ranges,
                    RangeUnit::Chars => byte_ranges_to_char_ranges(&text, &ranges),
                };
                if !flag.load(Ordering::Relaxed) && tx.send(ranges).is_ok() {
                    notify();
                }
            })
            .expect("검색 스레드 생성");
        Self { cancel, rx }
    }

    /// 결과가 왔는지 확인한다 (기다리지 않음).
    pub fn poll(&self) -> JobPoll {
        match self.rx.try_recv() {
            Ok(ranges) => JobPoll::Done(ranges),
            Err(TryRecvError::Empty) => JobPoll::Running,
            Err(TryRecvError::Disconnected) => JobPoll::Lost,
        }
    }

    /// 결과가 올 때까지 기다린다 (테스트·측정용). 취소됐으면 `None`.
    pub fn wait(self) -> Option<Vec<Range<usize>>> {
        self.rx.recv().ok()
    }
}

impl Drop for SearchJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 모든 매치를 찾되 취소 표시가 서면 `None`.
fn find_all_cancellable(
    query: &SearchQuery,
    text: &str,
    cancel: &AtomicBool,
) -> Option<Vec<Range<usize>>> {
    let mut ranges = Vec::new();
    for range in query.matches(text) {
        if ranges.len() % CANCEL_CHECK_EVERY == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        ranges.push(range);
    }
    (!cancel.load(Ordering::Relaxed)).then_some(ranges)
}

/// 바이트 범위 목록을 문자 인덱스 범위로 한 번에 변환한다.
///
/// 오름차순 목록이면 텍스트를 한 번만 훑는다 (매치마다 처음부터 세지 않는다). 범위가 텍스트를 넘거나
/// 글자 중간이면 앞쪽 글자 경계로 보정한다 (편집 직후의 오래된 결과에서도 패닉하지 않게).
pub fn byte_ranges_to_char_ranges(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut byte = 0;
    let mut chars = 0;
    let mut to_char = |b: usize| {
        let b = text.floor_char_boundary(b);
        if b >= byte {
            chars += text[byte..b].chars().count();
        } else {
            chars -= text[b..byte].chars().count();
        }
        byte = b;
        chars
    };
    ranges
        .iter()
        .map(|r| {
            let start = to_char(r.start);
            start..to_char(r.end)
        })
        .collect()
}

/// 문자 인덱스 범위를 바이트 범위로 변환한다 (egui 선택 영역 → 검색 시작 위치·선택 텍스트).
/// 텍스트 끝을 넘는 인덱스는 텍스트 끝으로 보정한다.
pub fn char_range_to_byte_range(text: &str, range: Range<usize>) -> Range<usize> {
    let byte_at = |from: usize, n: usize| {
        text[from..]
            .char_indices()
            .nth(n)
            .map_or(text.len(), |(i, _)| from + i)
    };
    let start = byte_at(0, range.start);
    start..byte_at(start, range.end.saturating_sub(range.start))
}
