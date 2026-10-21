//! 텍스트 버퍼 추상화 (`TextBuffer` 트레이트, `StringBuffer` 구현체, 줄 시작 위치 색인 `LineIndex`
//! — PLAN.md §2.2, §4.1, P1-07, P1-14).
//!
//! 설계 원칙:
//! - 에디터 위젯(방식 A `TextEdit`)과 향후 대용량 자체 위젯(방식 B `ropey`) 사이의 전환 비용을 줄이기 위해
//!   [`TextBuffer`] 트레이트로 텍스트 저장소를 추상화한다.
//! - 내부 텍스트는 항상 `LF`(`\n`)로 정규화된 상태를 유지한다.
//! - [`LineIndex`]는 각 논리 줄의 시작 바이트 오프셋을 보관하여, 상태 표시줄(`Ln/Col`), 줄 이동(`Go to Line`),
//!   대용량 읽기 전용 뷰어(`viewer.rs`), 검색·하이라이트·Markdown 블록 줄 매핑의 공통 기반이 된다.

use std::ops::Range;

/// `LF`(`\n`)로 정규화된 텍스트의 논리 줄 시작 바이트 오프셋 색인.
///
/// 항상 최소 1개의 줄(첫 줄 시작 오프셋 `0`)을 가진다.
/// 예:
/// - `""` → `[0]` (1줄)
/// - `"가\n나"` → `[0, 4]` (2줄)
/// - `"가\n나\n"` → `[0, 4, 8]` (3줄: 마지막 `\n` 뒤의 빈 줄 포함)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl Default for LineIndex {
    fn default() -> Self {
        Self {
            line_starts: vec![0],
        }
    }
}

impl LineIndex {
    /// 주어진 텍스트를 스캔하여 줄 시작 바이트 색인을 생성한다.
    pub fn new(text: &str) -> Self {
        let mut index = Self::default();
        index.rebuild(text);
        index
    }

    /// 기존 할당을 재사용하여 줄 시작 바이트 색인을 다시 계산한다.
    pub fn rebuild(&mut self, text: &str) {
        self.line_starts.clear();
        self.line_starts.push(0);
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                self.line_starts.push(i + 1);
            }
        }
    }

    /// 전체 논리 줄 수 (최소 1).
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// 각 줄의 시작 바이트 오프셋 슬라이스 (`0`-기반 줄 인덱스).
    pub fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    /// `0`-기반 `line`의 시작 바이트 오프셋을 반환한다.
    pub fn line_start_byte(&self, line: usize) -> Option<usize> {
        self.line_starts.get(line).copied()
    }

    /// `0`-기반 `line`의 바이트 범위(줄 끝의 `\n` 및 혹시 남아 있는 `\r` 제외)를 반환한다.
    pub fn line_byte_range(&self, line: usize, text: &str) -> Option<Range<usize>> {
        let start = *self.line_starts.get(line)?;
        let raw_end = self
            .line_starts
            .get(line + 1)
            .map(|&next_start| next_start.saturating_sub(1))
            .unwrap_or(text.len());
        let end = if raw_end > start && text.as_bytes().get(raw_end - 1) == Some(&b'\r') {
            raw_end - 1
        } else {
            raw_end
        };
        Some(start..end)
    }

    /// `0`-기반 `line`의 문자열 슬라이스(줄바꿈 제외)를 반환한다.
    pub fn line_str<'a>(&self, line: usize, text: &'a str) -> Option<&'a str> {
        let range = self.line_byte_range(line, text)?;
        text.get(range)
    }

    /// 주어진 바이트 오프셋이 속한 `0`-기반 줄 인덱스를 이진 탐색으로 찾는다.
    pub fn line_of_byte(&self, byte_offset: usize) -> usize {
        self.line_starts
            .partition_point(|&start| start <= byte_offset)
            .saturating_sub(1)
    }

    /// 바이트 오프셋을 상태 표시줄 표기용 **`1`-기반 `(줄 번호 Ln, 문자 단위 열 번호 Col)`**로 변환한다.
    ///
    /// `byte_offset`이 문자 경계 중간이거나 텍스트 길이를 넘더라도 안전하게 보정한다.
    pub fn line_col_of_byte(&self, text: &str, byte_offset: usize) -> (usize, usize) {
        let clamped = clamp_to_char_boundary(text, byte_offset.min(text.len()));
        let line_idx = self.line_of_byte(clamped);
        let line_start = self.line_starts[line_idx];
        let col_chars = text[line_start..clamped].chars().count();
        (line_idx + 1, col_chars + 1)
    }

    /// `0`-기반 문자 인덱스(`egui` 커서 인덱스)를 **`1`-기반 `(줄 번호 Ln, 문자 단위 열 번호 Col)`**로 변환한다.
    pub fn line_col_of_char(&self, text: &str, char_offset: usize) -> (usize, usize) {
        let byte_offset = char_to_byte_offset(text, char_offset);
        self.line_col_of_byte(text, byte_offset)
    }

    /// **`1`-기반 `(줄 번호 Ln, 문자 단위 열 번호 Col)`**을 UTF-8 바이트 오프셋으로 변환한다.
    ///
    /// 줄 번호나 열 번호가 범위를 벗어나면 해당 줄/문서의 유효 범위로 클램프한다 (`Go to Line` 등에서 사용).
    pub fn byte_of_line_col(&self, text: &str, line_1based: usize, col_1based: usize) -> usize {
        let line_idx = line_1based
            .saturating_sub(1)
            .min(self.line_count().saturating_sub(1));
        let range = self
            .line_byte_range(line_idx, text)
            .unwrap_or(text.len()..text.len());
        let line_slice = &text[range.clone()];
        let target_chars = col_1based.saturating_sub(1);

        match line_slice.char_indices().nth(target_chars) {
            Some((rel_byte, _)) => range.start + rel_byte,
            None => range.end,
        }
    }

    /// **`1`-기반 `(줄 번호 Ln, 문자 단위 열 번호 Col)`**을 `0`-기반 문자 인덱스(`egui` 커서용)로 변환한다.
    pub fn char_of_line_col(&self, text: &str, line_1based: usize, col_1based: usize) -> usize {
        let byte_offset = self.byte_of_line_col(text, line_1based, col_1based);
        byte_to_char_offset(text, byte_offset)
    }
}

/// `0`-기반 문자 인덱스를 UTF-8 바이트 오프셋으로 변환한다.
pub fn char_to_byte_offset(text: &str, char_offset: usize) -> usize {
    match text.char_indices().nth(char_offset) {
        Some((byte_idx, _)) => byte_idx,
        None => text.len(),
    }
}

/// UTF-8 바이트 오프셋을 `0`-기반 문자 인덱스로 변환한다.
pub fn byte_to_char_offset(text: &str, byte_offset: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, byte_offset.min(text.len()));
    text[..clamped].chars().count()
}

fn clamp_to_char_boundary(text: &str, mut byte_offset: usize) -> usize {
    byte_offset = byte_offset.min(text.len());
    while byte_offset > 0 && !text.is_char_boundary(byte_offset) {
        byte_offset -= 1;
    }
    byte_offset
}

/// 문서 텍스트 저장소 추상화 트레이트 (PLAN.md §2.2, §4.1).
pub trait TextBuffer {
    /// 전체 텍스트 슬라이스를 반환한다.
    fn as_str(&self) -> &str;

    /// 바이트 길이를 반환한다.
    fn len_bytes(&self) -> usize {
        self.as_str().len()
    }

    /// 유니코드 스칼라(`char`) 개수를 반환한다.
    fn len_chars(&self) -> usize {
        self.as_str().chars().count()
    }

    /// 버퍼가 비어 있는지 여부.
    fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    /// 줄 시작 위치 색인을 반환한다.
    fn line_index(&self) -> &LineIndex;

    /// 논리 줄 수를 반환한다 (최소 1).
    fn line_count(&self) -> usize {
        self.line_index().line_count()
    }

    /// `0`-기반 `line`의 텍스트 슬라이스(줄바꿈 제외)를 반환한다.
    fn line_str(&self, line: usize) -> Option<&str> {
        self.line_index().line_str(line, self.as_str())
    }

    /// 전체 텍스트를 교체하고 줄 색인을 갱신한다.
    fn set_text(&mut self, text: String);

    /// 지정한 바이트 범위를 `replacement`로 치환하고 줄 색인을 갱신한다.
    fn replace_range(&mut self, byte_range: Range<usize>, replacement: &str);
}

/// 방식 A(`String` 기반) 텍스트 버퍼 구현체.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringBuffer {
    text: String,
    line_index: LineIndex,
}

impl StringBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let line_index = LineIndex::new(&text);
        Self { text, line_index }
    }

    /// 내부 `String`에 대한 불변 참조.
    pub fn as_string(&self) -> &String {
        &self.text
    }

    /// 클로저에 `&mut String`을 빌려주고, 텍스트가 바뀌었으면 [`LineIndex`]를 자동 재구축한다.
    ///
    /// 클로저 실행 전후 텍스트가 실제로 변경되었는지(`bool`)를 반환한다.
    pub fn edit_string<F>(&mut self, f: F) -> bool
    where
        F: FnOnce(&mut String),
    {
        let before = self.text.clone();
        f(&mut self.text);
        if self.text != before {
            self.line_index.rebuild(&self.text);
            true
        } else {
            false
        }
    }
}

impl TextBuffer for StringBuffer {
    fn as_str(&self) -> &str {
        &self.text
    }

    fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    fn set_text(&mut self, text: String) {
        self.line_index.rebuild(&text);
        self.text = text;
    }

    fn replace_range(&mut self, byte_range: Range<usize>, replacement: &str) {
        self.text.replace_range(byte_range, replacement);
        self.line_index.rebuild(&self.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_handles_korean_multibyte_and_trailing_newline() {
        // "가나다\nABC\n한글"
        // 줄 0 ("가나다"): 바이트 0..9, 뒤따르는 '\n'은 9
        // 줄 1 ("ABC"):   바이트 10..13, 뒤따르는 '\n'은 13
        // 줄 2 ("한글"):  바이트 14..20
        let text = "가나다\nABC\n한글";
        let idx = LineIndex::new(text);

        assert_eq!(idx.line_count(), 3);
        assert_eq!(idx.line_starts(), &[0, 10, 14]);
        assert_eq!(idx.line_str(0, text), Some("가나다"));
        assert_eq!(idx.line_str(1, text), Some("ABC"));
        assert_eq!(idx.line_str(2, text), Some("한글"));
        assert_eq!(idx.line_str(3, text), None);

        // 바이트 오프셋 -> (1-기반 Ln, 1-기반 문자 Col)
        assert_eq!(idx.line_col_of_byte(text, 0), (1, 1)); // '가'
        assert_eq!(idx.line_col_of_byte(text, 3), (1, 2)); // '나'
        assert_eq!(idx.line_col_of_byte(text, 9), (1, 4)); // 줄 1 끝('\n' 위치)
        assert_eq!(idx.line_col_of_byte(text, 10), (2, 1)); // 'A'
        assert_eq!(idx.line_col_of_byte(text, 14), (3, 1)); // '한'
        assert_eq!(idx.line_col_of_byte(text, 17), (3, 2)); // '글'
        assert_eq!(idx.line_col_of_byte(text, 20), (3, 3)); // EOF

        // 문자 오프셋 -> (Ln, Col) 및 역변환 일치
        assert_eq!(idx.line_col_of_char(text, 0), (1, 1));
        assert_eq!(idx.line_col_of_char(text, 2), (1, 3)); // '다'
        assert_eq!(idx.line_col_of_char(text, 4), (2, 1)); // 'A'
        assert_eq!(idx.line_col_of_char(text, 8), (3, 1)); // '한'
        assert_eq!(idx.line_col_of_char(text, 9), (3, 2)); // '글'

        assert_eq!(idx.byte_of_line_col(text, 1, 2), 3);
        assert_eq!(idx.byte_of_line_col(text, 3, 2), 17);
        assert_eq!(idx.char_of_line_col(text, 3, 2), 9);

        // 끝 개행이 있는 경우 마지막 빈 줄까지 포함
        let with_trailing = "첫줄\n둘째줄\n";
        let idx_trailing = LineIndex::new(with_trailing);
        assert_eq!(idx_trailing.line_count(), 3);
        assert_eq!(idx_trailing.line_str(2, with_trailing), Some(""));
    }

    #[test]
    fn string_buffer_updates_line_index_on_mutation() {
        let mut buf = StringBuffer::new("줄1\n줄2");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.len_chars(), 5);

        // "줄1\n"은 3 + 1 + 1 = 5바이트
        buf.replace_range(5..5, "가운데\n");
        assert_eq!(buf.as_str(), "줄1\n가운데\n줄2");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.line_str(0), Some("줄1"));
        assert_eq!(buf.line_str(1), Some("가운데"));
        assert_eq!(buf.line_str(2), Some("줄2"));
    }
}
