//! 문서 모델 (`Document` — 경로, 버퍼, 인코딩, 줄바꿈, 수정/손상/읽기전용 플래그, 수정 카운터,
//! 탭별 독립 Undo/Redo 이력, 디스크 메타데이터 — PLAN.md §3.1, §3.3, §3.4, §4.1, P1-07).
//!
//! 설계 원칙:
//! - 각 탭은 독립된 [`Document`] 인스턴스를 소유하며, 커서 위치·스크롤 오프셋·Undo/Redo 이력을 탭별로 유지한다.
//! - 문서 내용이 변경될 때마다 단조 증가하는 `revision`(수정 카운터)을 갱신하여,
//!   하이라이트·검색·Markdown 파싱 캐시가 매 프레임 재계산 없이 변경 여부를 즉시 판별하게 한다 (PLAN.md §4.1).
//! - Undo/Redo 이력은 문서 전체가 아니라 **바뀐 구간(지운 문자열·넣은 문자열)만** 기록하고,
//!   단계 수와 바이트 합계 상한을 함께 둔다 (5MB 편집 문서에서도 이력 메모리가 문서 크기에 비례해 늘지 않게).
//! - 줄바꿈 없는 연속 입력·삭제·IME 조합은 하나의 Undo 단계로 묶는다. 공백으로 시작하는 입력, 줄바꿈,
//!   [`Document::break_undo_group`], 저장·인코딩/줄바꿈 변경·Undo/Redo에서 묶음이 끊긴다.
//! - `apply_batch_replacements` 등 원자적 편집 메서드는 전체 변경을 단일 Undo 항목으로 기록하여
//!   향후 `Replace All`(전체 바꾸기)을 한 번의 `Undo`로 되돌릴 수 있게 한다 (PLAN.md §3.2).

use std::collections::VecDeque;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::buffer::{LineIndex, StringBuffer, TextBuffer, byte_to_char_offset};
use crate::core::encoding::{
    DecodeOutcome, DecodedDocument, TextEncoding, UnmappableChar, UnmappableError,
    decode_bytes_with_default, encode_text_lossy_with_bom, encode_text_with_bom,
    find_unmappable_chars,
};
use crate::core::line_ending::{
    LineEnding, analyze_and_normalize_with_default, apply_line_ending, normalize_to_lf,
};

/// 탭별 Undo/Redo 스택의 기본 최대 보관 단계 수.
pub const DEFAULT_MAX_UNDO_HISTORY: usize = 200;

/// 탭별 Undo 스택이 보관하는 변경 구간 문자열의 기본 최대 바이트 합계 (16 MiB).
pub const DEFAULT_MAX_UNDO_BYTES: usize = 16 * 1024 * 1024;

/// 외부 변경 감지를 위해 파일 열기·저장 시점에 기록해 두는 디스크 메타데이터 (PLAN.md §3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskMetadata {
    pub modified_time: Option<SystemTime>,
    pub byte_len: u64,
}

impl DiskMetadata {
    /// 파일 시스템 경로에서 현재 메타데이터를 읽어 온다.
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        Ok(Self {
            modified_time: meta.modified().ok(),
            byte_len: meta.len(),
        })
    }
}

/// 바이너리 파일 바이트로 `Document::from_bytes`를 호출했을 때 반환되는 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryDocumentError;

/// Undo/Redo 스택에 저장되는 단일 변경 구간.
///
/// 편집 후 텍스트의 `start..start + inserted.len()` 구간이 편집 전에는 `removed`였다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoryEntry {
    /// 변경 구간의 시작 바이트 오프셋.
    start: usize,
    /// 편집 전 구간 문자열 (Undo 시 복원).
    removed: String,
    /// 편집 후 구간 문자열 (Redo 시 복원).
    inserted: String,
    /// 편집 전 커서/선택 범위.
    cursor_before: Range<usize>,
    /// 편집 후 커서/선택 범위.
    cursor_after: Range<usize>,
}

impl HistoryEntry {
    fn bytes(&self) -> usize {
        self.removed.len() + self.inserted.len()
    }

    /// 연속 입력 묶음 대상인지 여부 (줄바꿈이 없는 입력·삭제·치환).
    fn is_typing(&self) -> bool {
        !self.removed.contains('\n') && !self.inserted.contains('\n')
    }

    /// 바로 이어지는 입력 `next`를 이 항목에 합친다. 합칠 수 없으면 `false`.
    fn merge_typing(&mut self, next: &Self) -> bool {
        // 글자 뒤에 공백으로 시작하는 입력이 오면 단어 경계로 보고 새 단계를 시작한다.
        if next.removed.is_empty()
            && next.inserted.starts_with(char::is_whitespace)
            && self.inserted.ends_with(|c: char| !c.is_whitespace())
        {
            return false;
        }

        let end = self.start + self.inserted.len();
        if next.start + next.removed.len() == end && next.start >= self.start {
            // 이어서 입력하거나, 방금 넣은 끝부분을 고침 (IME 조합 갱신, Backspace)
            self.inserted.truncate(next.start - self.start);
            self.inserted.push_str(&next.inserted);
        } else if next.start + next.removed.len() == end {
            // Backspace가 묶음 시작보다 앞까지 지움: next.removed = (앞부분) + self.inserted
            let before = &next.removed[..self.start - next.start];
            self.removed.insert_str(0, before);
            self.start = next.start;
            self.inserted.clone_from(&next.inserted);
        } else if next.inserted.is_empty() && self.inserted.is_empty() && next.start == self.start {
            // Delete 키 연속 삭제
            self.removed.push_str(&next.removed);
        } else {
            return false;
        }
        self.cursor_after.clone_from(&next.cursor_after);
        true
    }
}

/// 마지막 저장 시점(또는 최초 로드 시점)의 문서 기준 상태.
///
/// Undo/Redo나 인코딩·줄바꿈 변경 후 문서가 저장 시점 상태와 일치하는지 판정하는 데 사용한다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SavedBaseline {
    text: String,
    encoding: TextEncoding,
    has_bom: bool,
    line_ending: LineEnding,
    /// 기준 시점에 저장하면 줄바꿈 바이트가 바뀌는 상태였는지 (`Mixed` 또는 단독 `CR`).
    line_ending_converts_on_save: bool,
}

/// 단일 탭에 대응하는 문서 상태 모델.
#[derive(Debug, Clone)]
pub struct Document {
    /// 디스크 상의 파일 경로 (`None`이면 아직 저장되지 않은 새 문서 `Untitled-N`).
    path: Option<PathBuf>,
    /// 새 문서 번호 (`Untitled-1`의 `1`). 파일로 저장되면 `None`이 된다.
    untitled_number: Option<usize>,
    /// 내부 `LF` 정규화 텍스트 버퍼 + 줄 시작 색인.
    buffer: StringBuffer,
    /// 문서 인코딩.
    encoding: TextEncoding,
    /// 원본 파일 BOM 존재 여부 및 저장 시 BOM 기록 여부.
    has_bom: bool,
    /// 인코딩이 휴리스틱 또는 `chardetng`로 추정되었는지 여부.
    encoding_estimated: bool,
    /// `chardetng` 추정 결과가 지원 목록 밖이어서 `EUC-KR`로 대체된 경우 원래 추정 인코딩 이름.
    encoding_fallback_from: Option<&'static str>,
    /// 디코딩 중 잘못된 바이트가 `U+FFFD`로 대체되었는지 여부 ("디코딩 손상").
    damaged: bool,
    /// 저장 시 사용할 줄바꿈 방식 (`CRLF` 또는 `LF`).
    line_ending: LineEnding,
    /// 파일 로드 시 줄바꿈 방식이 둘 이상 섞여 있었는지 여부 (`Mixed`).
    mixed_line_endings: bool,
    /// 파일 로드 시 단독 `CR`(구형 Mac)이 있었는지 여부 (저장 시 `line_ending`으로 변환됨).
    legacy_cr_line_endings: bool,
    /// 줄바꿈 통일 안내(`Mixed`·단독 `CR`)를 첫 저장 때 이미 표시했는지 여부.
    line_ending_conversion_notified: bool,
    /// 읽기 전용 여부 (파일 권한 또는 대용량/바이너리 읽기 전용 열람).
    readonly: bool,
    /// 마지막 저장(또는 최초 생성/로드) 이후 수정되었는지 여부 (`●` 마커).
    modified: bool,
    /// 편집·Undo/Redo·인코딩/줄바꿈 변경 때마다 1씩 증가하는 수정 카운터 (캐시 무효화 키).
    revision: u64,
    /// 탭별 커서/선택 문자 인덱스 범위 (`0`-기반 문자 오프셋).
    cursor_char_range: Range<usize>,
    /// 탭별 세로 스크롤 위치.
    scroll_offset_y: f32,
    /// 마지막으로 디스크에서 읽거나 저장했을 때의 파일 메타데이터.
    disk_metadata: Option<DiskMetadata>,
    /// 마지막 저장(또는 로드/생성) 기준 상태.
    saved_baseline: Option<SavedBaseline>,
    /// 탭별 독립 Undo 스택 (앞쪽이 오래된 단계).
    undo_stack: VecDeque<HistoryEntry>,
    /// Undo 스택 항목들의 바이트 합계.
    undo_bytes: usize,
    /// 탭별 독립 Redo 스택.
    redo_stack: Vec<HistoryEntry>,
    /// Undo 스택 최대 깊이.
    max_undo_history: usize,
    /// Undo 스택 최대 바이트 합계.
    max_undo_bytes: usize,
    /// 마지막 Undo 단계에 다음 연속 입력을 합칠 수 있는지 여부.
    typing_group_open: bool,
}

impl Document {
    /// OS 기본 인코딩(`UTF-8`)과 기본 줄바꿈(Windows `CRLF`, macOS `LF`)으로 새 빈 문서(`Untitled-N`)를 만든다.
    pub fn new_untitled(untitled_number: usize) -> Self {
        Self::new_untitled_with_defaults(
            untitled_number,
            TextEncoding::Utf8,
            LineEnding::default_for_os(),
        )
    }

    /// 지정한 기본 인코딩과 기본 줄바꿈으로 새 빈 문서(`Untitled-N`)를 만든다.
    pub fn new_untitled_with_defaults(
        untitled_number: usize,
        default_encoding: TextEncoding,
        default_line_ending: LineEnding,
    ) -> Self {
        let has_bom = !default_encoding.bom_bytes().is_empty();
        let baseline = SavedBaseline {
            text: String::new(),
            encoding: default_encoding,
            has_bom,
            line_ending: default_line_ending,
            line_ending_converts_on_save: false,
        };

        Self {
            path: None,
            untitled_number: Some(untitled_number),
            buffer: StringBuffer::new(String::new()),
            encoding: default_encoding,
            has_bom,
            encoding_estimated: false,
            encoding_fallback_from: None,
            damaged: false,
            line_ending: default_line_ending,
            mixed_line_endings: false,
            legacy_cr_line_endings: false,
            line_ending_conversion_notified: false,
            readonly: false,
            modified: false,
            revision: 0,
            cursor_char_range: 0..0,
            scroll_offset_y: 0.0,
            disk_metadata: None,
            saved_baseline: Some(baseline),
            undo_stack: VecDeque::new(),
            undo_bytes: 0,
            redo_stack: Vec::new(),
            max_undo_history: DEFAULT_MAX_UNDO_HISTORY,
            max_undo_bytes: DEFAULT_MAX_UNDO_BYTES,
            typing_group_open: false,
        }
    }

    /// 이미 디코딩된 [`DecodedDocument`]로부터 줄바꿈을 감지·`LF` 정규화하여 [`Document`]를 생성한다.
    pub fn from_decoded(
        path: Option<PathBuf>,
        decoded: DecodedDocument,
        default_line_ending: LineEnding,
        readonly: bool,
        disk_metadata: Option<DiskMetadata>,
    ) -> Self {
        let norm = analyze_and_normalize_with_default(&decoded.text, default_line_ending);
        let baseline = SavedBaseline {
            text: norm.text.clone(),
            encoding: decoded.encoding,
            has_bom: decoded.has_bom,
            line_ending: norm.detection.line_ending,
            line_ending_converts_on_save: norm.detection.converts_on_save(),
        };

        Self {
            path,
            untitled_number: None,
            buffer: StringBuffer::new(norm.text),
            encoding: decoded.encoding,
            has_bom: decoded.has_bom,
            encoding_estimated: decoded.estimated,
            encoding_fallback_from: decoded.fallback_from,
            damaged: decoded.damaged,
            line_ending: norm.detection.line_ending,
            mixed_line_endings: norm.detection.mixed,
            legacy_cr_line_endings: norm.detection.has_legacy_cr,
            line_ending_conversion_notified: false,
            readonly,
            modified: false,
            revision: 0,
            cursor_char_range: 0..0,
            scroll_offset_y: 0.0,
            disk_metadata,
            saved_baseline: Some(baseline),
            undo_stack: VecDeque::new(),
            undo_bytes: 0,
            redo_stack: Vec::new(),
            max_undo_history: DEFAULT_MAX_UNDO_HISTORY,
            max_undo_bytes: DEFAULT_MAX_UNDO_BYTES,
            typing_group_open: false,
        }
    }

    /// 원시 바이트 슬라이스에서 인코딩과 줄바꿈을 자동 감지하여 [`Document`]를 생성한다.
    ///
    /// 바이너리 파일로 판정되면 `Err(BinaryDocumentError)`를 반환한다.
    pub fn from_bytes(
        path: Option<PathBuf>,
        bytes: &[u8],
        readonly: bool,
        disk_metadata: Option<DiskMetadata>,
    ) -> Result<Self, BinaryDocumentError> {
        Self::from_bytes_with_defaults(
            path,
            bytes,
            TextEncoding::Utf8,
            LineEnding::default_for_os(),
            readonly,
            disk_metadata,
        )
    }

    /// 기본 인코딩과 기본 줄바꿈을 지정하여 원시 바이트 슬라이스로부터 [`Document`]를 생성한다.
    pub fn from_bytes_with_defaults(
        path: Option<PathBuf>,
        bytes: &[u8],
        default_encoding: TextEncoding,
        default_line_ending: LineEnding,
        readonly: bool,
        disk_metadata: Option<DiskMetadata>,
    ) -> Result<Self, BinaryDocumentError> {
        match decode_bytes_with_default(bytes, default_encoding) {
            DecodeOutcome::Binary => Err(BinaryDocumentError),
            DecodeOutcome::Text(decoded) => Ok(Self::from_decoded(
                path,
                decoded,
                default_line_ending,
                readonly,
                disk_metadata,
            )),
        }
    }

    // ── 조회자 (Getters) ──

    /// 파일 경로 (`None`이면 미저장 새 문서).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 탭 제목에 표시할 문서 이름 (파일 이름 또는 `Untitled-N`).
    pub fn title(&self) -> String {
        if let Some(path) = &self.path
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            return name.to_owned();
        }
        format!("Untitled-{}", self.untitled_number.unwrap_or(1))
    }

    /// 내부 `LF` 정규화 텍스트 슬라이스.
    pub fn text(&self) -> &str {
        self.buffer.as_str()
    }

    /// 내부 [`StringBuffer`] 참조.
    pub fn buffer(&self) -> &StringBuffer {
        &self.buffer
    }

    /// 줄 시작 위치 색인([`LineIndex`]) 참조.
    pub fn line_index(&self) -> &LineIndex {
        self.buffer.line_index()
    }

    /// 전체 논리 줄 수 (최소 1).
    pub fn line_count(&self) -> usize {
        self.buffer.line_count()
    }

    /// 현재 문서 인코딩.
    pub fn encoding(&self) -> TextEncoding {
        self.encoding
    }

    /// BOM 유무.
    pub fn has_bom(&self) -> bool {
        self.has_bom
    }

    /// 인코딩이 추정되었는지 여부 (`추정됨` 배지용).
    pub fn is_encoding_estimated(&self) -> bool {
        self.encoding_estimated
    }

    /// 지원 목록 밖 추정 인코딩에서 대체된 경우 원래 추정 인코딩 이름.
    pub fn encoding_fallback_from(&self) -> Option<&'static str> {
        self.encoding_fallback_from
    }

    /// 디코딩 손상 상태 여부 (`U+FFFD` 포함).
    pub fn is_damaged(&self) -> bool {
        self.damaged
    }

    /// 현재 저장용 줄바꿈 방식.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// 줄바꿈 혼합(`Mixed`) 여부.
    pub fn has_mixed_line_endings(&self) -> bool {
        self.mixed_line_endings
    }

    /// 단독 `CR`(구형 Mac) 줄바꿈이 남아 있는지 여부.
    pub fn has_legacy_cr_line_endings(&self) -> bool {
        self.legacy_cr_line_endings
    }

    /// 지금 저장하면 원본 줄바꿈 바이트가 바뀌는지 여부 (`Mixed` 또는 단독 `CR`, PLAN.md §3.4).
    pub fn line_ending_converts_on_save(&self) -> bool {
        self.mixed_line_endings || self.legacy_cr_line_endings
    }

    /// 상태 표시줄에 표시할 줄바꿈 라벨 (`"CRLF"`, `"LF"`, `"Mixed (CRLF)"`, `"Mixed (LF)"`).
    pub fn line_ending_status_label(&self) -> &'static str {
        self.line_ending.status_label(self.mixed_line_endings)
    }

    /// 첫 저장 시 줄바꿈 통일 안내(`Mixed` 또는 단독 `CR` 변환)가 필요한지 여부.
    pub fn needs_line_ending_conversion_notice(&self) -> bool {
        self.line_ending_converts_on_save() && !self.line_ending_conversion_notified
    }

    /// 줄바꿈 통일 안내를 표시 완료로 기록한다.
    pub fn mark_line_ending_conversion_notified(&mut self) {
        self.line_ending_conversion_notified = true;
    }

    /// 읽기 전용 여부.
    pub fn is_readonly(&self) -> bool {
        self.readonly
    }

    /// 읽기 전용 플래그를 변경한다 (예: 5MB 초과 읽기 전용 뷰어에서 사용자가 "편집 모드로 열기"를 승인한 경우).
    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }

    /// 수정 여부 (`●` 마커).
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// 단조 증가 수정 카운터 (하이라이트·검색·Markdown 캐시 무효화 키).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 탭별 커서/선택 문자 인덱스 범위.
    pub fn cursor_char_range(&self) -> Range<usize> {
        self.cursor_char_range.clone()
    }

    /// 탭별 커서/선택 문자 인덱스 범위를 갱신한다.
    pub fn set_cursor_char_range(&mut self, range: Range<usize>) {
        self.cursor_char_range = range;
    }

    /// 탭별 세로 스크롤 위치.
    pub fn scroll_offset_y(&self) -> f32 {
        self.scroll_offset_y
    }

    /// 탭별 세로 스크롤 위치를 갱신한다.
    pub fn set_scroll_offset_y(&mut self, offset_y: f32) {
        self.scroll_offset_y = offset_y;
    }

    /// 디스크 메타데이터 조회.
    pub fn disk_metadata(&self) -> Option<DiskMetadata> {
        self.disk_metadata
    }

    /// 파일 확장자가 `.md` 또는 `.markdown`인지 여부.
    pub fn is_markdown(&self) -> bool {
        self.path
            .as_deref()
            .and_then(Path::extension)
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
            })
    }

    // ── 편집 및 탭별 Undo/Redo ──

    /// 실행 취소(`Undo`) 가능 여부.
    pub fn can_undo(&self) -> bool {
        !self.readonly && !self.undo_stack.is_empty()
    }

    /// 다시 실행(`Redo`) 가능 여부.
    pub fn can_redo(&self) -> bool {
        !self.readonly && !self.redo_stack.is_empty()
    }

    /// 현재 Undo 스택에 쌓인 단계 수.
    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }

    /// 현재 Redo 스택에 쌓인 단계 수.
    pub fn redo_len(&self) -> usize {
        self.redo_stack.len()
    }

    /// Undo 스택이 보관 중인 변경 구간 문자열의 바이트 합계.
    pub fn undo_bytes(&self) -> usize {
        self.undo_bytes
    }

    /// Undo 스택의 최대 단계 수와 최대 바이트 합계를 바꾸고, 넘치는 오래된 단계는 즉시 버린다.
    ///
    /// 바이트 상한을 넘더라도 가장 최근 1단계는 남긴다 (큰 일괄 편집도 한 번은 되돌릴 수 있게).
    pub fn set_undo_limits(&mut self, max_steps: usize, max_bytes: usize) {
        self.max_undo_history = max_steps;
        self.max_undo_bytes = max_bytes;
        self.enforce_undo_limits();
    }

    /// 연속 입력 묶음을 끊어 다음 입력이 새 Undo 단계가 되게 한다.
    ///
    /// 커서 이동·포커스 이탈·입력 공백 시간 등 UI가 판단하는 경계에서 호출한다.
    pub fn break_undo_group(&mut self) {
        self.typing_group_open = false;
    }

    /// 전체 텍스트를 새 문자열로 교체하고(내부 `LF` 정규화 적용), 변경이 있으면 Undo 이력·수정 플래그·수정 카운터를 갱신한다.
    ///
    /// 읽기 전용(`readonly == true`)이거나 정규화 후 텍스트가 기존과 동일하면 아무것도 하지 않고 `false`를 반환한다.
    pub fn set_text(&mut self, new_text: impl Into<String>) -> bool {
        let cursor = self.cursor_char_range.clone();
        self.apply_edit_with_cursor(new_text, cursor)
    }

    /// 편집 후 커서 위치까지 함께 지정하여 텍스트를 변경한다 (에디터 위젯의 입력 경로).
    ///
    /// 이전 텍스트와 비교해 바뀐 구간만 Undo 이력에 기록하고, 줄바꿈 없는 연속 입력·삭제·IME 조합은
    /// 하나의 Undo 단계로 묶는다.
    pub fn apply_edit_with_cursor(
        &mut self,
        new_text: impl Into<String>,
        cursor_after: Range<usize>,
    ) -> bool {
        if self.readonly {
            return false;
        }

        let raw: String = new_text.into();
        let normalized = if raw.as_bytes().contains(&b'\r') {
            normalize_to_lf(&raw)
        } else {
            raw
        };

        let Some((start, old_end, new_end)) = diff_range(self.buffer.as_str(), &normalized) else {
            self.cursor_char_range = cursor_after;
            return false;
        };

        let inserted = &normalized[start..new_end];
        self.commit_edit(start..old_end, inserted, cursor_after, true);
        true
    }

    /// 지정한 바이트 범위를 `replacement`로 치환한다 (단일 Undo 단계로 기록).
    ///
    /// 범위가 텍스트 길이를 넘거나 문자 경계가 아니면 아무것도 하지 않고 `false`를 반환한다.
    pub fn replace_range(&mut self, byte_range: Range<usize>, replacement: &str) -> bool {
        if self.readonly || !is_valid_byte_range(self.buffer.as_str(), &byte_range) {
            return false;
        }
        let norm_rep = normalize_to_lf(replacement);
        if self.buffer.as_str()[byte_range.clone()] == *norm_rep {
            return false;
        }

        // 치환 후 커서는 넣은 문자열 끝. 문자 인덱스 = 범위 앞 문자 수 + 넣은 문자 수.
        let cursor_char =
            byte_to_char_offset(self.buffer.as_str(), byte_range.start) + norm_rep.chars().count();
        self.commit_edit(byte_range, &norm_rep, cursor_char..cursor_char, false);
        true
    }

    /// 여러 바이트 구간 치환(예: `Replace All` 전체 바꾸기)을 **단 한 번의 Undo 단계**로 일괄 적용한다 (PLAN.md §3.2).
    ///
    /// `replacements`는 오름차순·비중첩 바이트 범위 목록이어야 한다. 이전 범위와 겹치거나 텍스트 길이를 넘거나
    /// 문자 경계가 아닌 범위는 건너뛰며, 실제 치환된 개수를 반환한다.
    pub fn apply_batch_replacements(
        &mut self,
        replacements: &[(Range<usize>, &str)],
        cursor_after: Option<Range<usize>>,
    ) -> usize {
        if self.readonly || replacements.is_empty() {
            return 0;
        }

        let src = self.buffer.as_str();
        let mut out = String::with_capacity(src.len());
        let mut last_end = 0usize;
        let mut applied = 0usize;
        // 바뀐 전체 구간 (첫 치환 시작 ~ 마지막 치환 끝)
        let mut span: Option<Range<usize>> = None;

        for (range, rep) in replacements {
            if range.start < last_end || !is_valid_byte_range(src, range) {
                continue;
            }
            out.push_str(&src[last_end..range.start]);
            if rep.as_bytes().contains(&b'\r') {
                out.push_str(&normalize_to_lf(rep));
            } else {
                out.push_str(rep);
            }
            last_end = range.end;
            applied += 1;
            span = Some(span.map_or(range.start, |s| s.start)..range.end);
        }

        let Some(span) = span else {
            return 0;
        };
        // out = src[..span.start] + (치환된 구간) — 이 시점에 out 끝은 src[span.end]에 대응한다.
        let inserted = out[span.start..].to_owned();
        if src[span.clone()] == *inserted {
            return 0;
        }

        let cursor = cursor_after.unwrap_or_else(|| self.cursor_char_range.clone());
        self.commit_edit(span, &inserted, cursor, false);
        applied
    }

    /// 마지막 편집을 실행 취소(`Undo`)하고 복원 여부를 반환한다.
    pub fn undo(&mut self) -> bool {
        if self.readonly {
            return false;
        }
        let Some(entry) = self.undo_stack.pop_back() else {
            return false;
        };
        self.undo_bytes -= entry.bytes();
        self.typing_group_open = false;

        let end = entry.start + entry.inserted.len();
        self.buffer.replace_range(entry.start..end, &entry.removed);
        self.cursor_char_range = entry.cursor_before.clone();
        self.redo_stack.push(entry);
        self.bump_revision_and_sync_modified();
        true
    }

    /// 취소했던 편집을 다시 실행(`Redo`)하고 복원 여부를 반환한다.
    pub fn redo(&mut self) -> bool {
        if self.readonly {
            return false;
        }
        let Some(entry) = self.redo_stack.pop() else {
            return false;
        };
        self.typing_group_open = false;

        let end = entry.start + entry.removed.len();
        self.buffer.replace_range(entry.start..end, &entry.inserted);
        self.cursor_char_range = entry.cursor_after.clone();
        self.push_undo_entry(entry);
        self.bump_revision_and_sync_modified();
        true
    }

    /// Undo/Redo 이력을 모두 비운다.
    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.undo_bytes = 0;
        self.redo_stack.clear();
        self.typing_group_open = false;
    }

    // ── 인코딩 및 줄바꿈 전환 ──

    /// 문서 내용은 그대로 두고 저장할 인코딩만 변경한다 (`Command::ConvertToEncoding`, PLAN.md §3.3-5).
    ///
    /// 표준 BOM 정책(`UTF-8` / `EUC-KR`은 BOM 없음, `UTF-8 with BOM` / `UTF-16 LE/BE`는 BOM 있음)으로 맞추고
    /// 문서를 수정됨(`modified = true`) 상태로 전환한다.
    pub fn convert_to_encoding(&mut self, target_encoding: TextEncoding) -> bool {
        if self.readonly {
            return false;
        }
        let target_bom = !target_encoding.bom_bytes().is_empty();
        if self.encoding == target_encoding && self.has_bom == target_bom {
            return false;
        }

        self.encoding = target_encoding;
        self.has_bom = target_bom;
        self.encoding_estimated = false;
        self.encoding_fallback_from = None;
        self.typing_group_open = false;
        self.bump_revision_and_sync_modified();
        true
    }

    /// 저장할 줄바꿈 방식을 변경한다 (`Command::SetLineEnding`, PLAN.md §3.4).
    ///
    /// 줄바꿈이 바뀌거나, 같은 방식이라도 저장 시 변환이 남아 있으면(`Mixed`·단독 `CR`) 사용자가 통일을 고른 것으로 보고
    /// 두 플래그를 해제한 뒤 수정됨 상태로 전환한다.
    pub fn set_line_ending(&mut self, target_line_ending: LineEnding) -> bool {
        if self.readonly {
            return false;
        }
        if self.line_ending == target_line_ending && !self.line_ending_converts_on_save() {
            return false;
        }

        self.line_ending = target_line_ending;
        self.mixed_line_endings = false;
        self.legacy_cr_line_endings = false;
        self.typing_group_open = false;
        self.bump_revision_and_sync_modified();
        true
    }

    /// 현재 인코딩으로 표현할 수 없는 문자를 모두 `?`로 바꾸고 바꾼 개수를 돌려준다 (단일 Undo 단계).
    ///
    /// 표현 불가 문자 경고에서 "`?`로 바꿔 저장"을 고른 경우, 디스크에 쓴 내용과 화면이 같도록 문서도 바꾼다.
    pub fn replace_unmappable(&mut self) -> usize {
        let ranges: Vec<(Range<usize>, &str)> =
            find_unmappable_chars(self.buffer.as_str(), self.encoding, usize::MAX)
                .into_iter()
                .map(|c| (c.byte_offset..c.byte_offset + c.ch.len_utf8(), "?"))
                .collect();
        self.apply_batch_replacements(&ranges, None)
    }

    // ── 저장 직렬화 및 저장 완료 처리 ──

    /// 현재 문서의 줄바꿈 방식(`line_ending`)과 인코딩(`encoding`, `has_bom`)을 적용해
    /// 디스크에 기록할 바이트를 생성한다.
    ///
    /// 표현 불가 문자가 있으면 `Err(UnmappableError)`를 반환한다.
    pub fn encode_for_save(&self) -> Result<Vec<u8>, UnmappableError> {
        let with_eol = apply_line_ending(self.buffer.as_str(), self.line_ending);
        encode_text_with_bom(&with_eol, self.encoding, self.has_bom)
    }

    /// 표현 불가 문자를 `?`(`0x3F`)로 대체하여 디스크에 기록할 바이트를 생성한다.
    pub fn encode_for_save_lossy(&self) -> (Vec<u8>, Vec<UnmappableChar>) {
        let with_eol = apply_line_ending(self.buffer.as_str(), self.line_ending);
        encode_text_lossy_with_bom(&with_eol, self.encoding, self.has_bom)
    }

    /// 디스크 저장 성공 후 호출하여 수정 플래그(`modified`), 줄바꿈 변환 플래그, 손상 플래그를 초기화하고
    /// 경로·메타데이터 및 저장 기준점(`saved_baseline`)을 갱신한다.
    ///
    /// 연속 입력 묶음도 끊어서, Undo가 저장 시점을 건너뛰지 않게 한다.
    pub fn mark_saved(&mut self, new_path: Option<PathBuf>, disk_metadata: Option<DiskMetadata>) {
        if let Some(p) = new_path {
            self.path = Some(p);
            self.untitled_number = None;
        }
        if disk_metadata.is_some() {
            self.disk_metadata = disk_metadata;
        }
        self.mixed_line_endings = false;
        self.legacy_cr_line_endings = false;
        self.damaged = false;
        self.encoding_estimated = false;
        self.encoding_fallback_from = None;
        self.typing_group_open = false;
        self.saved_baseline = Some(SavedBaseline {
            text: self.buffer.as_string().clone(),
            encoding: self.encoding,
            has_bom: self.has_bom,
            line_ending: self.line_ending,
            line_ending_converts_on_save: false,
        });
        self.modified = false;
    }

    // ── 내부 도우미 ──

    /// 버퍼의 `range`를 `inserted`로 바꾸고 Undo 이력·수정 카운터를 갱신한다.
    fn commit_edit(
        &mut self,
        range: Range<usize>,
        inserted: &str,
        cursor_after: Range<usize>,
        typing: bool,
    ) {
        let entry = HistoryEntry {
            start: range.start,
            removed: self.buffer.as_str()[range.clone()].to_owned(),
            inserted: inserted.to_owned(),
            cursor_before: self.cursor_char_range.clone(),
            cursor_after: cursor_after.clone(),
        };
        self.buffer.replace_range(range, inserted);
        self.cursor_char_range = cursor_after;
        self.redo_stack.clear();

        let typing = typing && entry.is_typing();
        let merged = typing
            && self.typing_group_open
            && self.undo_stack.back_mut().is_some_and(|last| {
                let before = last.bytes();
                let merged = last.merge_typing(&entry);
                if merged {
                    self.undo_bytes = self.undo_bytes - before + last.bytes();
                }
                merged
            });
        if merged {
            self.enforce_undo_limits();
        } else {
            self.push_undo_entry(entry);
        }
        self.typing_group_open = typing;
        self.bump_revision_and_sync_modified();
    }

    fn push_undo_entry(&mut self, entry: HistoryEntry) {
        self.undo_bytes += entry.bytes();
        self.undo_stack.push_back(entry);
        self.enforce_undo_limits();
    }

    fn enforce_undo_limits(&mut self) {
        while self.undo_stack.len() > self.max_undo_history
            || (self.undo_bytes > self.max_undo_bytes && self.undo_stack.len() > 1)
        {
            let Some(oldest) = self.undo_stack.pop_front() else {
                break;
            };
            self.undo_bytes -= oldest.bytes();
        }
    }

    fn bump_revision_and_sync_modified(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.modified = match &self.saved_baseline {
            Some(base) => {
                self.buffer.as_str() != base.text
                    || self.encoding != base.encoding
                    || self.has_bom != base.has_bom
                    || self.line_ending != base.line_ending
                    || self.line_ending_converts_on_save() != base.line_ending_converts_on_save
            }
            None => true,
        };
    }
}

/// `range`가 `text` 안에 있고 양 끝이 문자 경계인지 여부.
fn is_valid_byte_range(text: &str, range: &Range<usize>) -> bool {
    range.start <= range.end
        && range.end <= text.len()
        && text.is_char_boundary(range.start)
        && text.is_char_boundary(range.end)
}

/// 두 문자열의 공통 앞부분·뒷부분을 제외한 바뀐 구간 `(start, old_end, new_end)`를 구한다 (같으면 `None`).
///
/// 경계는 문자 경계로 맞춘다. 같은 바이트가 이어지므로 `old`에서 문자 경계인 위치는 `new`에서도 문자 경계다.
fn diff_range(old: &str, new: &str) -> Option<(usize, usize, usize)> {
    if old == new {
        return None;
    }
    let (ob, nb) = (old.as_bytes(), new.as_bytes());

    let mut prefix = ob.iter().zip(nb).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(prefix) {
        prefix -= 1;
    }

    let max_suffix = ob.len().min(nb.len()) - prefix;
    let mut suffix = ob
        .iter()
        .rev()
        .zip(nb.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(ob.len() - suffix) {
        suffix -= 1;
    }

    Some((prefix, ob.len() - suffix, nb.len() - suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_updates_modified_flag_and_revision_and_save_resets_modified() {
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::CrLf);
        assert_eq!(doc.title(), "Untitled-1");
        assert!(!doc.is_modified());
        assert_eq!(doc.revision(), 0);

        // 1. 첫 편집 -> modified = true, revision 증가
        assert!(doc.set_text("첫 번째 줄\r\n두 번째 줄"));
        assert_eq!(doc.text(), "첫 번째 줄\n두 번째 줄");
        assert_eq!(doc.line_count(), 2);
        assert!(doc.is_modified());
        assert_eq!(doc.revision(), 1);

        // 같은 내용으로 다시 set_text 하면 아무 변화 없음
        assert!(!doc.set_text("첫 번째 줄\n두 번째 줄"));
        assert_eq!(doc.revision(), 1);

        // 2. 저장 완료 처리 -> modified = false 초기화, 경로 반영
        doc.mark_saved(
            Some(PathBuf::from("C:\\docs\\memo.txt")),
            Some(DiskMetadata {
                modified_time: None,
                byte_len: 30,
            }),
        );
        assert!(!doc.is_modified());
        assert_eq!(doc.title(), "memo.txt");

        // 저장용 바이트 직렬화 시 CRLF가 적용되는지 확인
        let saved_bytes = doc.encode_for_save().unwrap();
        assert_eq!(
            std::str::from_utf8(&saved_bytes).unwrap(),
            "첫 번째 줄\r\n두 번째 줄"
        );
    }

    #[test]
    fn undo_and_redo_restore_text_cursor_and_modified_state() {
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::Lf);

        // 단계 1 입력
        assert!(doc.apply_edit_with_cursor("안녕", 2..2));
        assert!(doc.is_modified());
        assert_eq!(doc.revision(), 1);

        // 이 시점에 저장했다고 가정
        doc.mark_saved(Some(PathBuf::from("greeting.txt")), None);
        assert!(!doc.is_modified());

        // 단계 2 입력
        assert!(doc.apply_edit_with_cursor("안녕하세요", 5..5));
        assert!(doc.is_modified());
        assert_eq!(doc.revision(), 2);

        // 단계 3 입력
        assert!(doc.apply_edit_with_cursor("안녕하세요 반갑습니다", 11..11));
        assert!(doc.is_modified());
        assert_eq!(doc.revision(), 3);

        // Undo 1회 -> "안녕하세요" (여전히 저장 시점 이후이므로 modified = true)
        assert!(doc.undo());
        assert_eq!(doc.text(), "안녕하세요");
        assert_eq!(doc.cursor_char_range(), 5..5);
        assert!(doc.is_modified());
        assert_eq!(doc.revision(), 4);

        // Undo 2회 -> "안녕" (저장 시점 텍스트와 일치하므로 modified = false로 복원!)
        assert!(doc.undo());
        assert_eq!(doc.text(), "안녕");
        assert_eq!(doc.cursor_char_range(), 2..2);
        assert!(!doc.is_modified());
        assert_eq!(doc.revision(), 5);

        // Undo 3회 -> "" (저장 시점 이전이므로 다시 modified = true!)
        assert!(doc.undo());
        assert_eq!(doc.text(), "");
        assert_eq!(doc.cursor_char_range(), 0..0);
        assert!(doc.is_modified());
        assert_eq!(doc.revision(), 6);
        assert!(!doc.can_undo());

        // Redo 1회 -> 다시 저장 시점("안녕")으로 돌아와 modified = false
        assert!(doc.redo());
        assert_eq!(doc.text(), "안녕");
        assert!(!doc.is_modified());

        // Redo 2회 -> "안녕하세요"
        assert!(doc.redo());
        assert_eq!(doc.text(), "안녕하세요");
        assert!(doc.is_modified());
    }

    #[test]
    fn batch_replacements_revert_in_single_undo() {
        let mut doc = Document::from_bytes(
            Some(PathBuf::from("test.txt")),
            b"apple banana apple cherry apple\n",
            false,
            None,
        )
        .unwrap();
        assert!(!doc.is_modified());

        // "apple" 3곳을 "포도"로 일괄 치환 (Replace All 시뮬레이션)
        let count = doc.apply_batch_replacements(
            &[(0..5, "포도"), (13..18, "포도"), (26..31, "포도")],
            Some(0..2),
        );
        assert_eq!(count, 3);
        assert_eq!(doc.text(), "포도 banana 포도 cherry 포도\n");
        assert!(doc.is_modified());
        assert_eq!(doc.undo_len(), 1);

        // 단 한 번의 Undo로 3곳 모두 원래대로 복원되어야 함 (PLAN.md §3.2)
        assert!(doc.undo());
        assert_eq!(doc.text(), "apple banana apple cherry apple\n");
        assert!(!doc.is_modified());
    }

    #[test]
    fn converting_encoding_and_line_ending_updates_modified_and_roundtrips() {
        let mut doc = Document::from_bytes(
            Some(PathBuf::from("mixed.txt")),
            "줄1\r\n줄2\n줄3\r\n".as_bytes(),
            false,
            None,
        )
        .unwrap();

        assert_eq!(doc.encoding(), TextEncoding::Utf8);
        assert_eq!(doc.line_ending(), LineEnding::CrLf);
        assert!(doc.has_mixed_line_endings());
        assert_eq!(doc.line_ending_status_label(), "Mixed (CRLF)");
        assert!(!doc.is_modified());

        // 줄바꿈을 LF로 전환 -> Mixed 해제, modified = true
        assert!(doc.set_line_ending(LineEnding::Lf));
        assert!(!doc.has_mixed_line_endings());
        assert_eq!(doc.line_ending_status_label(), "LF");
        assert!(doc.is_modified());

        // 인코딩을 EUC-KR로 변환 저장 설정
        assert!(doc.convert_to_encoding(TextEncoding::EucKr));
        assert_eq!(doc.encoding(), TextEncoding::EucKr);
        assert!(!doc.has_bom());
        assert!(doc.is_modified());
    }

    /// egui 입력창처럼 한 글자씩 전체 텍스트를 넘겨 입력한다.
    fn type_chars(doc: &mut Document, s: &str) {
        for ch in s.chars() {
            let mut text = doc.text().to_owned();
            let cursor = doc.cursor_char_range().end;
            let byte = crate::core::buffer::char_to_byte_offset(&text, cursor);
            text.insert(byte, ch);
            assert!(doc.apply_edit_with_cursor(text, cursor + 1..cursor + 1));
        }
    }

    #[test]
    fn undo_history_records_changed_span_not_whole_text() {
        let base = "한글 줄 abc\n".repeat(20_000); // 약 300KB (디버그 빌드 테스트 시간 고려)
        let mut doc = Document::from_bytes(None, base.as_bytes(), false, None).unwrap();
        let end = doc.text().chars().count();
        doc.set_cursor_char_range(end..end);

        type_chars(&mut doc, &"가".repeat(200));
        // 200자 연속 입력은 한 단계로 묶이고, 이력은 넣은 글자 바이트만 보관한다.
        assert_eq!(doc.undo_len(), 1);
        assert_eq!(doc.undo_bytes(), 200 * '가'.len_utf8());

        assert!(doc.undo());
        assert_eq!(doc.text(), base);
        assert!(!doc.is_modified());
        assert_eq!(doc.cursor_char_range(), end..end);
        assert!(doc.redo());
        assert!(doc.text().ends_with(&"가".repeat(200)));
    }

    #[test]
    fn undo_history_respects_step_and_byte_limits() {
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::Lf);
        doc.set_undo_limits(3, 1000);

        // 범위 치환은 묶지 않으므로 매번 새 단계
        for i in 0..5 {
            let len = doc.text().len();
            assert!(doc.replace_range(len..len, &format!("{i}")));
        }
        assert_eq!(doc.undo_len(), 3);

        // 600바이트 치환 2번 → 바이트 상한(1000) 때문에 최근 1단계만 남음
        for _ in 0..2 {
            let len = doc.text().len();
            assert!(doc.replace_range(len..len, &"x".repeat(600)));
        }
        assert_eq!(doc.undo_len(), 1);
        assert!(doc.undo_bytes() <= 1000);

        // 상한보다 큰 단일 편집도 최근 1단계는 남겨 되돌릴 수 있다.
        let len = doc.text().len();
        assert!(doc.replace_range(len..len, &"y".repeat(5000)));
        assert_eq!(doc.undo_len(), 1);
        let before = doc.text().len();
        assert!(doc.undo());
        assert_eq!(doc.text().len(), before - 5000);
    }

    #[test]
    fn typing_is_grouped_until_whitespace_newline_or_break() {
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::Lf);

        type_chars(&mut doc, "안녕 하세요");
        assert_eq!(doc.undo_len(), 2); // "안녕" | " 하세요"
        type_chars(&mut doc, "\n");
        type_chars(&mut doc, "둘째");
        assert_eq!(doc.undo_len(), 4); // 줄바꿈은 단독 단계, 이후 새 묶음

        doc.break_undo_group();
        type_chars(&mut doc, "줄");
        assert_eq!(doc.undo_len(), 5);

        assert!(doc.undo());
        assert_eq!(doc.text(), "안녕 하세요\n둘째");
        assert!(doc.undo());
        assert_eq!(doc.text(), "안녕 하세요\n");
        assert!(doc.undo());
        assert_eq!(doc.text(), "안녕 하세요");
        assert!(doc.undo());
        assert_eq!(doc.text(), "안녕");
        assert!(doc.undo());
        assert_eq!(doc.text(), "");
        assert!(!doc.is_modified());
    }

    #[test]
    fn ime_composition_and_backspace_merge_into_one_step() {
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::Lf);

        // 한글 IME 조합: 조합 중 글자가 계속 바뀌어도 한 단계
        for (text, cursor) in [
            ("ㅎ", 1),
            ("하", 1),
            ("한", 1),
            ("한ㄱ", 2),
            ("한그", 2),
            ("한글", 2),
        ] {
            assert!(doc.apply_edit_with_cursor(text, cursor..cursor));
        }
        assert_eq!(doc.undo_len(), 1);
        assert_eq!(doc.undo_bytes(), "한글".len());

        // 커서 이동 후 Backspace 연속 → 한 단계
        doc.mark_saved(Some(PathBuf::from("a.txt")), None);
        type_chars(&mut doc, "입력");
        doc.break_undo_group();
        for (text, cursor) in [("한글입", 3), ("한글", 2), ("한", 1)] {
            assert!(doc.apply_edit_with_cursor(text, cursor..cursor));
        }
        assert_eq!(doc.undo_len(), 3);
        assert!(doc.undo());
        assert_eq!(doc.text(), "한글입력");
        assert_eq!(doc.cursor_char_range(), 4..4);

        // Delete 키 연속 → 한 단계
        doc.set_cursor_char_range(0..0);
        doc.break_undo_group();
        for text in ["글입력", "입력", "력"] {
            assert!(doc.apply_edit_with_cursor(text, 0..0));
        }
        assert_eq!(doc.text(), "력");
        assert!(doc.undo());
        assert_eq!(doc.text(), "한글입력");

        // 저장 시점에서 묶음이 끊기므로 Undo가 저장 시점("한글")에 멈춘다.
        assert!(doc.undo());
        assert_eq!(doc.text(), "한글");
        assert!(!doc.is_modified());
    }

    #[test]
    fn invalid_ranges_are_rejected_without_panic() {
        let mut doc = Document::from_bytes(None, "가나다".as_bytes(), false, None).unwrap();

        assert!(!doc.replace_range(1..2, "x")); // 문자 경계 아님
        assert!(!doc.replace_range(0..100, "x")); // 길이 초과
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = 6..3;
        assert!(!doc.replace_range(reversed, "x")); // 시작 > 끝
        assert_eq!(doc.text(), "가나다");
        assert_eq!(doc.undo_len(), 0);
        assert_eq!(doc.revision(), 0);

        // 일괄 치환: 잘못된 범위만 건너뛰고 나머지는 적용
        let applied =
            doc.apply_batch_replacements(&[(1..3, "x"), (3..6, "N"), (7..100, "y")], None);
        assert_eq!(applied, 1);
        assert_eq!(doc.text(), "가N다");
        assert_eq!(doc.apply_batch_replacements(&[(1..2, "x")], None), 0);
        assert!(doc.undo());
        assert_eq!(doc.text(), "가나다");
    }

    #[test]
    fn legacy_cr_only_file_needs_conversion_notice() {
        let mut doc = Document::from_bytes(None, b"a\rb\rc\r", false, None).unwrap();
        assert!(!doc.has_mixed_line_endings());
        assert!(doc.has_legacy_cr_line_endings());
        assert!(doc.line_ending_converts_on_save());
        assert!(doc.needs_line_ending_conversion_notice());
        assert!(!doc.is_modified());

        doc.mark_line_ending_conversion_notified();
        assert!(!doc.needs_line_ending_conversion_notice());

        // 같은 방식을 다시 골라도 단독 CR 통일을 선택한 것이므로 수정됨
        let current = doc.line_ending();
        assert!(doc.set_line_ending(current));
        assert!(!doc.has_legacy_cr_line_endings());
        assert!(doc.is_modified());

        // 저장 후에는 변환 대상이 아님
        let mut doc = Document::from_bytes(None, b"a\rb\r", false, None).unwrap();
        doc.mark_saved(None, None);
        assert!(!doc.line_ending_converts_on_save());
        assert!(!doc.needs_line_ending_conversion_notice());

        // 순수 CRLF 파일은 안내 대상이 아님
        let doc = Document::from_bytes(None, b"a\r\nb\r\n", false, None).unwrap();
        assert!(!doc.needs_line_ending_conversion_notice());
    }

    #[test]
    fn replace_unmappable_makes_document_savable_in_one_undo() {
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::CrLf);
        doc.set_text(
            "한글 😀
둘째 줄 👍 끝",
        );
        assert!(doc.convert_to_encoding(TextEncoding::EucKr));
        let err = doc.encode_for_save().unwrap_err();
        assert_eq!(err.unmappable.len(), 2);

        assert_eq!(doc.replace_unmappable(), 2);
        assert_eq!(
            doc.text(),
            "한글 ?
둘째 줄 ? 끝"
        );
        assert!(doc.encode_for_save().is_ok());
        assert_eq!(doc.replace_unmappable(), 0);

        assert!(doc.undo());
        assert_eq!(
            doc.text(),
            "한글 😀
둘째 줄 👍 끝"
        );
    }

    #[test]
    fn diff_range_aligns_to_char_boundaries() {
        // '가'(EA B0 80)와 '각'(EA B0 81)은 앞 2바이트가 같다.
        assert_eq!(diff_range("가", "각"), Some((0, 3, 3)));
        // '나'(EB 82 98) → '냐'(EB 83 90): 첫 바이트만 같음
        assert_eq!(diff_range("가나다", "가냐다"), Some((3, 6, 6)));
        assert_eq!(diff_range("abc", "abc"), None);
        assert_eq!(diff_range("aa", "aaa"), Some((2, 2, 3)));
        assert_eq!(diff_range("한글", ""), Some((0, 6, 0)));
    }
}
