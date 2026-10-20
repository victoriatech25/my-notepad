//! 프로토타입용 간이 매치 계산 (실제 검색 엔진은 Phase 4).

use std::ops::Range;

#[derive(Default, Clone)]
pub struct FindState {
    pub open: bool,
    pub replace_open: bool,
    pub query: String,
    pub replacement: String,
    pub regex: bool,
    pub match_case: bool,
    pub whole_word: bool,
    pub current: usize,
    pub focus_query: bool,
}

impl FindState {
    /// 정규식 모드에서 괄호 짝이 안 맞으면 오류로 보여준다 (프로토타입용 흉내).
    pub fn regex_error(&self) -> Option<&'static str> {
        if !self.regex {
            return None;
        }
        let mut depth_paren = 0i32;
        let mut depth_bracket = 0i32;
        for c in self.query.chars() {
            match c {
                '(' => depth_paren += 1,
                ')' => depth_paren -= 1,
                '[' => depth_bracket += 1,
                ']' => depth_bracket -= 1,
                _ => {}
            }
            if depth_paren < 0 || depth_bracket < 0 {
                return Some("정규식 오류: 여는 괄호 없이 닫는 괄호가 있습니다");
            }
        }
        if depth_paren > 0 {
            Some("정규식 오류: 닫는 괄호 ')'가 없습니다")
        } else if depth_bracket > 0 {
            Some("정규식 오류: 닫는 대괄호 ']'가 없습니다")
        } else {
            None
        }
    }

    /// 검색어의 매치 구간 (바이트 범위). 정규식 모드도 일반 문자열로 취급한다.
    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        if !self.open || self.query.is_empty() || self.regex_error().is_some() {
            return Vec::new();
        }
        find_all(text, &self.query, self.match_case, self.whole_word)
    }
}

/// 선택 영역이 스마트 하이라이트 조건(한 줄, 공백 없음, 2글자 이상)을 만족하면 그 단어를 돌려준다.
pub fn smart_word(selected: &str) -> Option<&str> {
    let word = selected.trim();
    if word.chars().count() >= 2 && !word.chars().any(char::is_whitespace) {
        Some(word)
    } else {
        None
    }
}

pub fn find_all(text: &str, needle: &str, match_case: bool, whole_word: bool) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    let (hay, pat) = if match_case {
        (text.to_owned(), needle.to_owned())
    } else {
        (text.to_lowercase(), needle.to_lowercase())
    };
    // 소문자 변환으로 바이트 길이가 바뀌는 문자가 있으면 위치가 어긋나므로 원문 기준으로 다시 맞춘다.
    if hay.len() != text.len() {
        return Vec::new();
    }
    hay.match_indices(&pat)
        .map(|(start, m)| start..start + m.len())
        .filter(|r| !whole_word || is_whole_word(text, r))
        .collect()
}

fn is_whole_word(text: &str, range: &Range<usize>) -> bool {
    let before = text[..range.start].chars().next_back();
    let after = text[range.end..].chars().next();
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    !is_word(before) && !is_word(after)
}

pub fn byte_to_char(text: &str, byte_index: usize) -> usize {
    text[..byte_index].chars().count()
}

/// 문자 인덱스 → (줄, 열), 둘 다 1부터.
pub fn line_col(text: &str, char_index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for c in text.chars().take(char_index) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
