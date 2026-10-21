//! 문서 모델 (`Document` — 경로, 버퍼, 인코딩, 줄바꿈, 수정/손상/읽기전용 플래그, 수정 카운터,
//! 탭별 독립 Undo/Redo 이력, 디스크 메타데이터 — PLAN.md §3.1, §3.3, §3.4, §4.1, P1-07).
//!
//! 설계 원칙:
//! - 각 탭은 독립된 [`Document`] 인스턴스를 소유하며, 커서 위치·스크롤 오프셋·Undo/Redo 이력을 탭별로 유지한다.
//! - 문서 내용이 변경될 때마다 단조 증가하는 `revision`(수정 카운터)을 갱신하여,
//!   하이라이트·검색·Markdown 파싱 캐시가 매 프레임 재계산 없이 변경 여부를 즉시 판별하게 한다 (PLAN.md §4.1).
//! - `apply_batch_replacements` 등 원자적 편집 메서드는 전체 변경을 단일 Undo 항목으로 기록하여
//!   향후 `Replace All`(전체 바꾸기)을 한 번의 `Undo`로 되돌릴 수 있게 한다 (PLAN.md §3.2).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::buffer::{LineIndex, StringBuffer, TextBuffer};
use crate::core::encoding::{
    DecodeOutcome, DecodedDocument, TextEncoding, UnmappableChar, UnmappableError,
    decode_bytes_with_default, encode_text_lossy_with_bom, encode_text_with_bom,
};
use crate::core::line_ending::{LineEnding, analyze_and_normalize_with_default, apply_line_ending};

/// 탭별 Undo/Redo 스택의 기본 최대 보관 단계 수.
pub const DEFAULT_MAX_UNDO_HISTORY: usize = 200;

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

/// Undo/Redo 스택에 저장되는 단일 스냅샷.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoryEntry {
    text: String,
    cursor_char_range: Range<usize>,
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
    had_mixed_line_endings: bool,
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
    /// `Mixed` 줄바꿈 통일 안내를 첫 저장 때 이미 표시했는지 여부.
    mixed_line_ending_notified: bool,
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
    /// 탭별 독립 Undo 스택.
    undo_stack: Vec<HistoryEntry>,
    /// 탭별 독립 Redo 스택.
    redo_stack: Vec<HistoryEntry>,
    /// Undo 스택 최대 깊이.
    max_undo_history: usize,
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
            had_mixed_line_endings: false,
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
            mixed_line_ending_notified: false,
            readonly: false,
            modified: false,
            revision: 0,
            cursor_char_range: 0..0,
            scroll_offset_y: 0.0,
            disk_metadata: None,
            saved_baseline: Some(baseline),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_undo_history: DEFAULT_MAX_UNDO_HISTORY,
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
            had_mixed_line_endings: norm.detection.mixed,
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
            mixed_line_ending_notified: false,
            readonly,
            modified: false,
            revision: 0,
            cursor_char_range: 0..0,
            scroll_offset_y: 0.0,
            disk_metadata,
            saved_baseline: Some(baseline),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_undo_history: DEFAULT_MAX_UNDO_HISTORY,
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

    /// 상태 표시줄에 표시할 줄바꿈 라벨 (`"CRLF"`, `"LF"`, `"Mixed (CRLF)"`, `"Mixed (LF)"`).
    pub fn line_ending_status_label(&self) -> &'static str {
        self.line_ending.status_label(self.mixed_line_endings)
    }

    /// 첫 저장 시 `Mixed` 줄바꿈 통일 안내가 필요한지 여부.
    pub fn needs_mixed_line_ending_notice(&self) -> bool {
        self.mixed_line_endings && !self.mixed_line_ending_notified
    }

    /// `Mixed` 줄바꿈 통일 안내를 표시 완료로 기록한다.
    pub fn mark_mixed_line_ending_notified(&mut self) {
        self.mixed_line_ending_notified = true;
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

    /// 전체 텍스트를 새 문자열로 교체하고(내부 `LF` 정규화 적용), 변경이 있으면 Undo 이력·수정 플래그·수정 카운터를 갱신한다.
    ///
    /// 읽기 전용(`readonly == true`)이거나 정규화 후 텍스트가 기존과 동일하면 아무것도 하지 않고 `false`를 반환한다.
    pub fn set_text(&mut self, new_text: impl Into<String>) -> bool {
        let cursor = self.cursor_char_range.clone();
        self.apply_edit_with_cursor(new_text, cursor)
    }

    /// 편집 후 커서 위치까지 함께 지정하여 텍스트를 변경한다.
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
            crate::core::line_ending::normalize_to_lf(&raw)
        } else {
            raw
        };

        if normalized == self.buffer.as_str() {
            self.cursor_char_range = cursor_after;
            return false;
        }

        self.push_undo_snapshot();
        self.redo_stack.clear();
        self.buffer.set_text(normalized);
        self.cursor_char_range = cursor_after;
        self.bump_revision_and_sync_modified();
        true
    }

    /// 지정한 바이트 범위를 `replacement`로 치환한다 (단일 Undo 단계로 기록).
    pub fn replace_range(&mut self, byte_range: Range<usize>, replacement: &str) -> bool {
        if self.readonly {
            return false;
        }
        let norm_rep = crate::core::line_ending::normalize_to_lf(replacement);
        if self.buffer.as_str().get(byte_range.clone()) == Some(norm_rep.as_str()) {
            return false;
        }

        self.push_undo_snapshot();
        self.redo_stack.clear();
        self.buffer.replace_range(byte_range.clone(), &norm_rep);
        let new_cursor_byte = byte_range.start + norm_rep.len();
        let new_cursor_char =
            crate::core::buffer::byte_to_char_offset(self.buffer.as_str(), new_cursor_byte);
        self.cursor_char_range = new_cursor_char..new_cursor_char;
        self.bump_revision_and_sync_modified();
        true
    }

    /// 여러 바이트 구간 치환(예: `Replace All` 전체 바꾸기)을 **단 한 번의 Undo 단계**로 일괄 적용한다 (PLAN.md §3.2).
    ///
    /// `replacements`는 오름차순·비중첩 바이트 범위 목록이어야 하며, 실제 치환된 개수를 반환한다.
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

        for (range, rep) in replacements {
            if range.start < last_end || range.end > src.len() || range.start > range.end {
                continue;
            }
            out.push_str(&src[last_end..range.start]);
            if rep.as_bytes().contains(&b'\r') {
                out.push_str(&crate::core::line_ending::normalize_to_lf(rep));
            } else {
                out.push_str(rep);
            }
            last_end = range.end;
            applied += 1;
        }

        if applied == 0 {
            return 0;
        }
        out.push_str(&src[last_end..]);

        if out == src {
            return 0;
        }

        self.push_undo_snapshot();
        self.redo_stack.clear();
        self.buffer.set_text(out);
        if let Some(cursor) = cursor_after {
            self.cursor_char_range = cursor;
        }
        self.bump_revision_and_sync_modified();
        applied
    }

    /// 마지막 편집을 실행 취소(`Undo`)하고 복원 여부를 반환한다.
    pub fn undo(&mut self) -> bool {
        if self.readonly {
            return false;
        }
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };

        self.redo_stack.push(HistoryEntry {
            text: self.buffer.as_string().clone(),
            cursor_char_range: self.cursor_char_range.clone(),
        });
        self.buffer.set_text(prev.text);
        self.cursor_char_range = prev.cursor_char_range;
        self.bump_revision_and_sync_modified();
        true
    }

    /// 취소했던 편집을 다시 실행(`Redo`)하고 복원 여부를 반환한다.
    pub fn redo(&mut self) -> bool {
        if self.readonly {
            return false;
        }
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };

        self.undo_stack.push(HistoryEntry {
            text: self.buffer.as_string().clone(),
            cursor_char_range: self.cursor_char_range.clone(),
        });
        self.buffer.set_text(next.text);
        self.cursor_char_range = next.cursor_char_range;
        self.bump_revision_and_sync_modified();
        true
    }

    /// Undo/Redo 이력을 모두 비운다.
    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
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
        self.bump_revision_and_sync_modified();
        true
    }

    /// 저장할 줄바꿈 방식을 변경한다 (`Command::SetLineEnding`, PLAN.md §3.4).
    ///
    /// 줄바꿈이 바뀌거나 기존에 `Mixed` 상태였으면 `mixed_line_endings`를 해제하고 수정됨 상태로 전환한다.
    pub fn set_line_ending(&mut self, target_line_ending: LineEnding) -> bool {
        if self.readonly {
            return false;
        }
        if self.line_ending == target_line_ending && !self.mixed_line_endings {
            return false;
        }

        self.line_ending = target_line_ending;
        self.mixed_line_endings = false;
        self.bump_revision_and_sync_modified();
        true
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

    /// 디스크 저장 성공 후 호출하여 수정 플래그(`modified`), `Mixed` 플래그, 손상 플래그를 초기화하고
    /// 경로·메타데이터 및 저장 기준점(`saved_baseline`)을 갱신한다.
    pub fn mark_saved(&mut self, new_path: Option<PathBuf>, disk_metadata: Option<DiskMetadata>) {
        if let Some(p) = new_path {
            self.path = Some(p);
            self.untitled_number = None;
        }
        if disk_metadata.is_some() {
            self.disk_metadata = disk_metadata;
        }
        self.mixed_line_endings = false;
        self.damaged = false;
        self.encoding_estimated = false;
        self.encoding_fallback_from = None;
        self.saved_baseline = Some(SavedBaseline {
            text: self.buffer.as_string().clone(),
            encoding: self.encoding,
            has_bom: self.has_bom,
            line_ending: self.line_ending,
            had_mixed_line_endings: false,
        });
        self.modified = false;
    }

    // ── 내부 도우미 ──

    fn push_undo_snapshot(&mut self) {
        if self.undo_stack.len() >= self.max_undo_history {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(HistoryEntry {
            text: self.buffer.as_string().clone(),
            cursor_char_range: self.cursor_char_range.clone(),
        });
    }

    fn bump_revision_and_sync_modified(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.modified = match &self.saved_baseline {
            Some(base) => {
                self.buffer.as_str() != base.text
                    || self.encoding != base.encoding
                    || self.has_bom != base.has_bom
                    || self.line_ending != base.line_ending
                    || self.mixed_line_endings != base.had_mixed_line_endings
            }
            None => true,
        };
    }
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
}
