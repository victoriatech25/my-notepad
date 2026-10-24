//! 선택 단어 일괄 하이라이트(Smart Highlighting) 조건 판정 및 매치 계산 (PLAN.md §3.2-1, P2-03).
//!
//! - 조건([`target_word`]): 선택을 앞뒤 공백 제거한 뒤 2글자 이상이고 안에 공백(줄바꿈 포함)이 없을 것.
//!   여러 줄에 걸친 선택은 안에 줄바꿈이 있으므로 자연히 제외된다.
//! - 매치([`find_matches`]): 기본값은 **단어 단위 일치 + 대소문자 구분** (Notepad++ 기본 동작).
//!   단어 글자는 문자·숫자(한글 포함)와 `_`이다. 한글은 조사가 붙으므로 단어 단위로 `검색`을 찾으면
//!   `검색어`는 매치되지 않는다 → 부분 일치 옵션(`whole_word = false`)으로 포함할 수 있다.
//! - 대소문자 무시는 글자마다 소문자로 바꿔 비교하되 위치는 원문 바이트 기준으로 돌려준다
//!   (소문자로 바꾸면 바이트 길이가 달라지는 글자가 있어도 위치가 어긋나지 않는다).
//! - 상한: 매치가 [`HighlightOptions::max_matches`]를 넘으면 계산을 멈추고 빈 결과 + `exceeded`를 돌려준다
//!   (하이라이트 중단, 상태 표시줄 안내는 UI가 맡는다 — P2-04).
//!
//! 위치는 문서 텍스트(`LF` 정규화)의 바이트 범위이며, 서로 겹치지 않고 앞에서부터 순서대로다.

use std::ops::Range;

/// 기본 최대 매치 수.
pub const DEFAULT_MAX_MATCHES: usize = 10_000;

/// 매칭 옵션.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightOptions {
    /// 단어 단위 일치 (끄면 부분 일치)
    pub whole_word: bool,
    /// 대소문자 구분
    pub match_case: bool,
    /// 최대 매치 수 (넘으면 하이라이트 중단)
    pub max_matches: usize,
}

impl Default for HighlightOptions {
    fn default() -> Self {
        Self {
            whole_word: true,
            match_case: true,
            max_matches: DEFAULT_MAX_MATCHES,
        }
    }
}

/// 매치 계산 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatchSet {
    /// 매치 위치 (바이트 범위, 오름차순·겹침 없음). 상한을 넘으면 비어 있다.
    pub ranges: Vec<Range<usize>>,
    /// 매치 수가 상한을 넘어 계산을 멈췄는지 여부.
    pub exceeded: bool,
}

/// 선택 텍스트가 하이라이트 조건을 만족하면 대상 단어(앞뒤 공백 제거)를 돌려준다.
pub fn target_word(selected: &str) -> Option<&str> {
    let word = selected.trim();
    (word.chars().nth(1).is_some() && !word.chars().any(char::is_whitespace)).then_some(word)
}

/// `text`에서 `word`의 매치 위치를 찾는다.
pub fn find_matches(text: &str, word: &str, options: &HighlightOptions) -> MatchSet {
    if word.is_empty() {
        return MatchSet::default();
    }
    let lower: Vec<char> = if options.match_case {
        Vec::new()
    } else {
        word.chars().flat_map(char::to_lowercase).collect()
    };
    // 단어 단위 일치에서 경계 검사는 단어의 끝 글자가 단어 글자일 때만 한다 (`\b`와 같은 규칙).
    let check_start = options.whole_word && word.chars().next().is_some_and(is_word_char);
    let check_end = options.whole_word && word.chars().next_back().is_some_and(is_word_char);

    let mut ranges = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        let found = if options.match_case {
            text[pos..]
                .find(word)
                .map(|i| (pos + i, pos + i + word.len()))
        } else {
            find_ignore_case(text, pos, &lower)
        };
        let Some((start, end)) = found else {
            break;
        };
        let bounded = (!check_start
            || !text[..start].chars().next_back().is_some_and(is_word_char))
            && (!check_end || !text[end..].chars().next().is_some_and(is_word_char));
        if bounded {
            if ranges.len() == options.max_matches {
                return MatchSet {
                    ranges: Vec::new(),
                    exceeded: true,
                };
            }
            ranges.push(start..end);
            pos = end;
        } else {
            // 경계가 맞지 않으면 다음 글자부터 다시 찾는다 (`aaa`에서 단어 `aa` 등).
            pos = start + text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    MatchSet {
        ranges,
        exceeded: false,
    }
}

/// 단어 글자: 문자·숫자(한글 포함)와 `_`.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `from`부터 대소문자를 무시하고 찾은 첫 매치의 `(시작, 끝)` 바이트 위치.
fn find_ignore_case(text: &str, from: usize, lower: &[char]) -> Option<(usize, usize)> {
    let first = *lower.first()?;
    text[from..].char_indices().find_map(|(i, c)| {
        let start = from + i;
        // 첫 글자부터 다르면 건너뛴다 (대부분의 위치는 여기서 끝남).
        if c.to_lowercase().next() != Some(first) {
            return None;
        }
        match_len_ignore_case(&text[start..], lower).map(|len| (start, start + len))
    })
}

/// `rest` 앞부분이 소문자 글자열 `lower`와 같으면 원문에서 차지하는 바이트 길이를 돌려준다.
/// 매치 끝은 원문 글자 경계와 맞아야 한다.
fn match_len_ignore_case(rest: &str, lower: &[char]) -> Option<usize> {
    let mut matched = 0;
    for (offset, c) in rest.char_indices() {
        for l in c.to_lowercase() {
            if lower.get(matched) != Some(&l) {
                return None;
            }
            matched += 1;
        }
        if matched == lower.len() {
            return Some(offset + c.len_utf8());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(whole_word: bool, match_case: bool) -> HighlightOptions {
        HighlightOptions {
            whole_word,
            match_case,
            ..Default::default()
        }
    }

    /// 매치된 원문 조각들
    fn found<'t>(text: &'t str, word: &str, options: &HighlightOptions) -> Vec<&'t str> {
        find_matches(text, word, options)
            .ranges
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn target_word_conditions() {
        // 2글자 이상 → 적용 (앞뒤 공백은 제거)
        assert_eq!(target_word("ab"), Some("ab"));
        assert_eq!(target_word("검색"), Some("검색"));
        assert_eq!(target_word("  error\t"), Some("error"));
        assert_eq!(target_word("word\n"), Some("word"));
        // 1글자·빈 선택 → 미적용
        assert_eq!(target_word("a"), None);
        assert_eq!(target_word("한"), None);
        assert_eq!(target_word(" x "), None);
        assert_eq!(target_word(""), None);
        assert_eq!(target_word("   "), None);
        // 공백 포함·여러 줄 → 미적용
        assert_eq!(target_word("hello world"), None);
        assert_eq!(target_word("한글 단어"), None);
        assert_eq!(target_word("first\nsecond"), None);
        assert_eq!(target_word("a\u{3000}b"), None); // 전각 공백
    }

    #[test]
    fn default_options_are_whole_word_and_case_sensitive() {
        let o = HighlightOptions::default();
        assert!(o.whole_word);
        assert!(o.match_case);
        assert_eq!(o.max_matches, DEFAULT_MAX_MATCHES);
    }

    #[test]
    fn whole_word_vs_partial() {
        let text = "error errors error_code xerror error.";
        assert_eq!(found(text, "error", &opts(true, true)), ["error", "error"]);
        let partial = find_matches(text, "error", &opts(false, true));
        assert_eq!(partial.ranges.len(), 5);
        assert_eq!(partial.ranges[0], 0..5);
    }

    #[test]
    fn korean_whole_word_excludes_particles_partial_includes() {
        let text = "검색 검색어 검색을 다시 검색";
        let whole = find_matches(text, "검색", &opts(true, true));
        assert_eq!(
            whole.ranges.len(),
            2,
            "`검색어`·`검색을`은 단어 단위에서 제외"
        );
        assert_eq!(&text[whole.ranges[1].clone()], "검색");
        assert_eq!(whole.ranges[1].start, text.rfind("검색").unwrap());
        assert_eq!(
            find_matches(text, "검색", &opts(false, true)).ranges.len(),
            4,
            "부분 일치는 `검색어`·`검색을`도 포함"
        );
    }

    #[test]
    fn case_options() {
        let text = "Value value VALUE vAlUe values";
        assert_eq!(found(text, "value", &opts(true, true)), ["value"]);
        assert_eq!(
            found(text, "value", &opts(true, false)),
            ["Value", "value", "VALUE", "vAlUe"]
        );
        assert_eq!(found(text, "VALUE", &opts(false, false)).len(), 5);
    }

    #[test]
    fn ignore_case_keeps_original_byte_positions() {
        // 'İ'(2바이트)는 소문자로 바꾸면 "i̇"(3바이트)가 되어 길이가 달라진다.
        // 'K'(켈빈 기호, 3바이트)의 소문자는 'k'(1바이트).
        let text = "İx Kelvin kelvin ǅungla";
        let r = find_matches(text, "kelvin", &opts(true, false));
        assert_eq!(r.ranges.len(), 2);
        assert_eq!(&text[r.ranges[0].clone()], "Kelvin");
        assert_eq!(&text[r.ranges[1].clone()], "kelvin");
        // 앞쪽 글자의 길이 변화와 상관없이 뒤쪽 매치 위치가 원문 기준
        assert_eq!(found(text, "ǆungla", &opts(true, false)), ["ǅungla"]);
    }

    #[test]
    fn matches_do_not_overlap_and_boundaries_retry() {
        assert_eq!(found("aaaa", "aa", &opts(false, true)), ["aa", "aa"]);
        // 단어 단위: `aaa` 안의 `aa`는 어느 위치든 경계에 걸려 실패, 뒤의 `aa`만 매치
        assert_eq!(found("aaa aa", "aa", &opts(true, true)), ["aa"]);
        // 경계 실패 뒤 같은 단어 안의 다음 위치에서 다시 찾는다
        assert_eq!(found("xab ab", "ab", &opts(true, false)), ["ab"]);
    }

    #[test]
    fn non_word_edges_need_no_boundary() {
        // 단어 끝 글자가 단어 글자가 아니면 그쪽 경계는 검사하지 않는다 (`\b`와 같은 규칙)
        let text = "a->b x->y ->";
        assert_eq!(found(text, "->", &opts(true, true)).len(), 3);
        assert_eq!(
            found("call foo() foo()bar", "foo()", &opts(true, true)),
            ["foo()", "foo()"]
        );
    }

    #[test]
    fn multiline_text_and_byte_ranges() {
        let text = "첫 줄 한글\n둘째 줄 한글\n한글";
        let r = find_matches(text, "한글", &HighlightOptions::default());
        assert_eq!(r.ranges.len(), 3);
        for range in &r.ranges {
            assert_eq!(&text[range.clone()], "한글");
        }
        assert!(!r.exceeded);
    }

    #[test]
    fn exceeding_max_matches_stops_with_flag() {
        let text = "ab ".repeat(30);
        let capped = HighlightOptions {
            max_matches: 30,
            ..Default::default()
        };
        let r = find_matches(&text, "ab", &capped);
        assert_eq!(r.ranges.len(), 30, "상한과 같으면 그대로");
        assert!(!r.exceeded);

        let capped = HighlightOptions {
            max_matches: 29,
            ..Default::default()
        };
        let r = find_matches(&text, "ab", &capped);
        assert!(r.exceeded, "상한 초과 → 중단 플래그");
        assert!(r.ranges.is_empty(), "중단 시 하이라이트 없음");
    }

    #[test]
    fn default_cap_on_large_text() {
        let text = "id ".repeat(DEFAULT_MAX_MATCHES + 1);
        let r = find_matches(&text, "id", &HighlightOptions::default());
        assert!(r.exceeded);
        let text = "id ".repeat(DEFAULT_MAX_MATCHES);
        assert_eq!(
            find_matches(&text, "id", &HighlightOptions::default())
                .ranges
                .len(),
            DEFAULT_MAX_MATCHES
        );
    }

    #[test]
    fn empty_inputs() {
        assert_eq!(
            find_matches("abc", "", &HighlightOptions::default()),
            MatchSet::default()
        );
        assert_eq!(
            find_matches("", "ab", &HighlightOptions::default()),
            MatchSet::default()
        );
    }
}
