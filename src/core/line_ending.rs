//! 줄바꿈(`CRLF` / `LF` / `CR` / `Mixed`) 감지, 내부 `LF` 정규화, 저장용 변환 (PLAN.md §3.4 — P1-06).
//!
//! 설계 원칙 (PLAN.md §3.4):
//! - **내부 표현은 `LF`(`\n`)로 정규화**하고, 문서별로 저장 시 사용할 줄바꿈 방식([`LineEnding`])을 별도 보관한다.
//! - **감지**: 파일 로드 시 `CRLF`(`\r\n`) / 단독 `LF`(`\n`) / 단독 `CR`(`\r`, 구형 Mac) 개수를 세어
//!   저장 방식으로 고를 수 있는 `CRLF`·`LF` 중 많은 쪽을 채택한다. 둘 이상의 줄바꿈 방식이 섞여 있으면 `mixed = true`로
//!   표시하고 상태 표시줄에는 `Mixed (CRLF)` 또는 `Mixed (LF)` 형태로 표기한다. 단독 `CR`은 저장 시 변환되므로
//!   `has_legacy_cr = true`로 표시한다 ([`LineEndingDetection::converts_on_save`]).
//! - **새 문서, 개행 없는 문서, `CRLF`·`LF` 동수 문서 기본값**: Windows는 `CRLF`, macOS/Linux는 `LF`.
//! - **파일 끝 개행 보존**: 원본 텍스트의 끝 개행 유무(있음/없음)를 정규화 및 저장 시 그대로 보존한다.

/// 문서 저장 및 UI 선택에 사용하는 줄바꿈 방식 (PLAN.md §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnding {
    CrLf,
    Lf,
}

impl LineEnding {
    pub const ALL: [Self; 2] = [Self::CrLf, Self::Lf];

    /// 현재 OS의 새 문서 기본 줄바꿈 방식 (Windows: `CRLF`, macOS/Linux: `LF`).
    pub const fn default_for_os() -> Self {
        if cfg!(windows) { Self::CrLf } else { Self::Lf }
    }

    /// 실제 줄바꿈 문자열 슬라이스 (`"\r\n"` 또는 `"\n"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CrLf => "\r\n",
            Self::Lf => "\n",
        }
    }

    /// 상태 표시줄에 표시할 짧은 이름 (`"CRLF"` 또는 `"LF"`).
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::CrLf => "CRLF",
            Self::Lf => "LF",
        }
    }

    /// 혼합(`Mixed`) 여부를 반영한 상태 표시줄 표기 문자열 (`"CRLF"`, `"LF"`, `"Mixed (CRLF)"`, `"Mixed (LF)"`).
    pub const fn status_label(self, mixed: bool) -> &'static str {
        match (self, mixed) {
            (Self::CrLf, false) => "CRLF",
            (Self::Lf, false) => "LF",
            (Self::CrLf, true) => "Mixed (CRLF)",
            (Self::Lf, true) => "Mixed (LF)",
        }
    }

    /// 상단 메뉴·팝업에 표시할 설명 포함 이름 (`"CRLF (Windows)"`, `"LF (macOS / Linux)"`).
    pub const fn menu_label(self) -> &'static str {
        match self {
            Self::CrLf => "CRLF (Windows)",
            Self::Lf => "LF (macOS / Linux)",
        }
    }
}

/// 텍스트 내에 등장한 각 줄바꿈 종류별 개수.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineEndingCounts {
    /// `\r\n` (Windows 표준) 개수.
    pub crlf: usize,
    /// `\r`이 앞에 없는 단독 `\n` (Unix / macOS 표준) 개수.
    pub lf: usize,
    /// `\n`이 뒤에 따르지 않는 단독 `\r` (구형 Mac) 개수.
    pub cr: usize,
}

impl LineEndingCounts {
    /// 전체 줄바꿈 개수 합계.
    pub const fn total(self) -> usize {
        self.crlf + self.lf + self.cr
    }

    /// 서로 다른 줄바꿈 방식이 몇 종류 등장했는지 반환한다 (`0..=3`).
    pub const fn kinds_present(self) -> u8 {
        (self.crlf > 0) as u8 + (self.lf > 0) as u8 + (self.cr > 0) as u8
    }

    /// 둘 이상의 줄바꿈 방식이 섞여 있는지 여부.
    pub const fn is_mixed(self) -> bool {
        self.kinds_present() >= 2
    }
}

/// 원본 텍스트의 줄바꿈 감지 결과 요약 (PLAN.md §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineEndingDetection {
    /// 채택된 대표(다수) 줄바꿈 방식.
    pub line_ending: LineEnding,
    /// 둘 이상의 줄바꿈 방식(`CRLF` / `LF` / `CR`)이 섞여 있었는지 여부.
    pub mixed: bool,
    /// 단독 `CR`(`\r`, 구형 Mac)이 1개 이상 포함되어 저장 시 변환이 일어나는지 여부.
    pub has_legacy_cr: bool,
    /// 파일 끝에 개행 문자(`\r\n`, `\n`, `\r`)가 있었는지 여부.
    ///
    /// 정보 표시용이다. 끝 개행은 정규화 텍스트에 그대로 남으므로 저장 시 이 값을 다시 적용하지 않는다
    /// (편집 중 사용자가 마지막 개행을 지우면 그대로 저장된다).
    pub has_trailing_newline: bool,
    /// 종류별 줄바꿈 개수.
    pub counts: LineEndingCounts,
}

impl LineEndingDetection {
    /// 상태 표시줄에 표시할 줄바꿈 라벨 (`"CRLF"`, `"LF"`, `"Mixed (CRLF)"`, `"Mixed (LF)"`).
    pub const fn status_label(&self) -> &'static str {
        self.line_ending.status_label(self.mixed)
    }

    /// 채택된 방식([`Self::line_ending`])으로 저장하면 원본 줄바꿈 바이트가 바뀌는지 여부.
    ///
    /// 여러 방식이 섞였거나(`mixed`) 단독 `CR`(구형 Mac, 저장 방식으로 지원하지 않음)이 있으면 `true`다.
    /// "저장 시 채택된 방식으로 통일됨" 첫 저장 1회 알림(PLAN.md §3.4)은 이 값을 기준으로 한다.
    pub const fn converts_on_save(&self) -> bool {
        self.mixed || self.has_legacy_cr
    }
}

/// 줄바꿈 감지 결과와 내부 `LF` 정규화 텍스트를 함께 담는 구조체.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDocumentText {
    /// 모든 `\r\n` 및 단독 `\r`이 `\n`(`LF`)으로 정규화된 내부 텍스트.
    pub text: String,
    /// 원본 텍스트에서 감지된 줄바꿈 메타데이터.
    pub detection: LineEndingDetection,
}

/// 텍스트 내의 `CRLF`, 단독 `LF`, 단독 `CR` 개수를 센다.
pub fn count_line_endings(text: &str) -> LineEndingCounts {
    let bytes = text.as_bytes();
    let mut counts = LineEndingCounts::default();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    counts.crlf += 1;
                    i += 2;
                } else {
                    counts.cr += 1;
                    i += 1;
                }
            }
            b'\n' => {
                counts.lf += 1;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    counts
}

/// 현재 OS 기본 줄바꿈을 기준으로 원본 텍스트의 줄바꿈 방식을 감지한다.
pub fn detect_line_ending(text: &str) -> LineEndingDetection {
    detect_line_ending_with_default(text, LineEnding::default_for_os())
}

/// 개행이 없거나 동수일 때 사용할 `default_line_ending`을 지정해 줄바꿈 방식을 감지한다.
///
/// 규칙 (PLAN.md §3.4):
/// - 저장 방식으로 고를 수 있는 `CRLF`와 `LF` 중 더 많은 쪽을 채택한다.
/// - 둘이 동수이면(개행이 없거나 단독 `CR`만 있는 경우 포함) `default_line_ending`을 따른다.
/// - 단독 `CR`(구형 Mac)은 저장 방식으로 지원하지 않으므로 개수가 가장 많아도 채택 대상이 아니며,
///   `has_legacy_cr = true`로 표시해 저장 시 변환됨([`LineEndingDetection::converts_on_save`])을 알린다.
pub fn detect_line_ending_with_default(
    text: &str,
    default_line_ending: LineEnding,
) -> LineEndingDetection {
    let counts = count_line_endings(text);
    let has_trailing_newline = text.ends_with('\n') || text.ends_with('\r');

    let line_ending = match counts.crlf.cmp(&counts.lf) {
        std::cmp::Ordering::Greater => LineEnding::CrLf,
        std::cmp::Ordering::Less => LineEnding::Lf,
        std::cmp::Ordering::Equal => default_line_ending,
    };

    LineEndingDetection {
        line_ending,
        mixed: counts.is_mixed(),
        has_legacy_cr: counts.cr > 0,
        has_trailing_newline,
        counts,
    }
}

/// 모든 `\r\n`과 단독 `\r`을 `\n`(`LF`)으로 정규화한 문자열을 반환한다.
///
/// 파일 끝 개행 유무는 원본 그대로 보존된다.
pub fn normalize_to_lf(text: &str) -> String {
    let bytes = text.as_bytes();
    if !bytes.contains(&b'\r') {
        return text.to_owned();
    }

    let mut out = String::with_capacity(text.len());
    let mut seg_start = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'\r' {
            out.push_str(&text[seg_start..i]);
            out.push('\n');
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
            seg_start = i;
        } else {
            i += 1;
        }
    }

    if seg_start < text.len() {
        out.push_str(&text[seg_start..]);
    }

    out
}

/// 원본 텍스트의 줄바꿈을 감지하고 내부 `LF` 표현으로 정규화한다 (현재 OS 기본값 사용).
pub fn analyze_and_normalize(text: &str) -> NormalizedDocumentText {
    analyze_and_normalize_with_default(text, LineEnding::default_for_os())
}

/// 지정한 기본 줄바꿈(`default_line_ending`)을 사용해 줄바꿈 감지 및 `LF` 정규화를 수행한다.
pub fn analyze_and_normalize_with_default(
    text: &str,
    default_line_ending: LineEnding,
) -> NormalizedDocumentText {
    let detection = detect_line_ending_with_default(text, default_line_ending);
    let normalized = if detection.counts.crlf == 0 && detection.counts.cr == 0 {
        text.to_owned()
    } else {
        normalize_to_lf(text)
    };

    NormalizedDocumentText {
        text: normalized,
        detection,
    }
}

/// 내부 텍스트(또는 임의의 텍스트)의 줄바꿈을 저장용 `target` 방식(`CRLF` 또는 `LF`)으로 변환한다.
///
/// - 원본(또는 정규화 텍스트)의 파일 끝 개행 유무(있음/없음)를 그대로 보존한다 (PLAN.md §3.4).
/// - 입력에 혹시 `\r\n`이나 단독 `\r`이 섞여 있더라도 중복 `\r\r\n`이 생기지 않도록 안전하게 처리한다.
pub fn apply_line_ending(text: &str, target: LineEnding) -> String {
    match target {
        LineEnding::Lf => normalize_to_lf(text),
        LineEnding::CrLf => {
            let bytes = text.as_bytes();
            let mut out =
                String::with_capacity(text.len() + bytes.iter().filter(|&&b| b == b'\n').count());
            let mut seg_start = 0usize;
            let mut i = 0usize;

            while i < bytes.len() {
                match bytes[i] {
                    b'\r' => {
                        out.push_str(&text[seg_start..i]);
                        out.push_str("\r\n");
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                            i += 2;
                        } else {
                            i += 1;
                        }
                        seg_start = i;
                    }
                    b'\n' => {
                        out.push_str(&text[seg_start..i]);
                        out.push_str("\r\n");
                        i += 1;
                        seg_start = i;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }

            if seg_start < text.len() {
                out.push_str(&text[seg_start..]);
            }

            out
        }
    }
}
