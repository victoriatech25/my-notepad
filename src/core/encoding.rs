//! 문자 인코딩 감지·디코딩·인코딩 (PLAN.md §3.3 — P1-04, P1-05).
//!
//! P1-04 구현 범위:
//! - 지원 인코딩 열거형 [`TextEncoding`] (`UTF-8`, `UTF-8 with BOM`, `EUC-KR (CP949)`, `UTF-16 LE`, `UTF-16 BE`)
//! - 열기 시 6단계 인코딩 감지 ([`detect_encoding`], [`detect_encoding_with_default`]):
//!   1. BOM 확인 (`EF BB BF` → UTF-8 BOM, `FF FE` → UTF-16 LE, `FE FF` → UTF-16 BE)
//!   2. 빈 파일 → 설정의 기본 인코딩(기본값 `UTF-8`)
//!   3. UTF-8 유효성 검사 (`0x00` 바이트가 없고 전체가 유효한 UTF-8이면 `UTF-8`, ASCII 전용 포함.
//!      잘못된 구간이 적거나 끝에서 잘린 UTF-8은 `UTF-8` 추정 + 디코딩 손상)
//!   4. BOM 없는 UTF-16 휴리스틱 (`0x00`이 있을 때만: 짝수/홀수 위치 `0x00` 비율, 한글 위주면 두 엔디언 디코딩 비교)
//!   5. 바이너리 판정 (위에 해당하지 않으면서 `0x00` 바이트가 포함되면 [`DetectionOutcome::Binary`])
//!   6. 레거시 추정 (`chardetng`, `allow_utf8 = false`, 지원 목록 밖이면 `EUC-KR (CP949)`로 대체 + `fallback_from`에 원래 추정 결과)
//! - 디코딩 및 오류 처리 ([`decode_bytes`], [`decode_with_encoding`]):
//!   잘못된 바이트 시퀀스를 `U+FFFD`(`\u{FFFD}`)로 치환하고 `damaged = true` 플래그를 기록한다.
//!
//! P1-05 구현 범위:
//! - 인코딩 직렬화 ([`encode_text`], [`encode_text_with_bom`], [`encode_text_lossy`]):
//!   - BOM은 인코딩이 결정한다 (`UTF-8` 없음, `UTF-8 with BOM` 있음, `EUC-KR` 없음, UTF-16 기본 있음).
//!     원본 BOM 유무 보존은 UTF-16에만 적용하고, 인코딩을 바꾸면 [`DecodedDocument::set_encoding`]이 표준 정책으로 되돌린다.
//!   - `UTF-8 with BOM`: BOM(`EF BB BF`) 직접 기록
//!   - `UTF-16 LE` / `UTF-16 BE`: `encoding_rs`는 UTF-16 출력 인코딩을 지원하지 않으므로
//!     `str::encode_utf16()`으로 엔디언별 바이트 직렬화 및 BOM(`FF FE` / `FE FF`) 직접 기록
//!   - `EUC-KR (CP949)`: `encoding_rs`의 기본 `encode()`가 표현 불가 문자를 HTML 숫자 참조(`&#128512;`)로
//!     몰래 치환하는 문제를 방지하기 위해 `encode_from_utf8_to_vec_without_replacement`를 사용하여
//!     표현 불가 문자 목록([`UnmappableChar`])을 검출하고, 손실 저장 승인 시에만 `?`(`0x3F`)로 대체한다.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::EncoderResult;

/// UTF-8 BOM 바이트 시퀀스 (`EF BB BF`).
pub const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
/// UTF-16 LE BOM 바이트 시퀀스 (`FF FE`).
pub const UTF16LE_BOM: &[u8] = &[0xFF, 0xFE];
/// UTF-16 BE BOM 바이트 시퀀스 (`FE FF`).
pub const UTF16BE_BOM: &[u8] = &[0xFE, 0xFF];

/// My-Notepad가 지원하는 문서 인코딩 (PLAN.md §3.3-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    EucKr,
    Utf16Le,
    Utf16Be,
}

impl TextEncoding {
    pub const ALL: [Self; 5] = [
        Self::Utf8,
        Self::Utf8Bom,
        Self::EucKr,
        Self::Utf16Le,
        Self::Utf16Be,
    ];

    /// UI(상태 표시줄·메뉴)에 표기할 공식 인코딩 이름.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 with BOM",
            Self::EucKr => "EUC-KR (CP949)",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
        }
    }

    /// 저장 시 파일 맨 앞에 기록할 BOM 바이트 슬라이스.
    pub const fn bom_bytes(self) -> &'static [u8] {
        match self {
            Self::Utf8 | Self::EucKr => &[],
            Self::Utf8Bom => UTF8_BOM,
            Self::Utf16Le => UTF16LE_BOM,
            Self::Utf16Be => UTF16BE_BOM,
        }
    }

    /// `UTF-16 LE` / `UTF-16 BE`인지 여부. 저장 시 BOM 생략(원본 보존)은 UTF-16에만 허용한다.
    pub const fn is_utf16(self) -> bool {
        matches!(self, Self::Utf16Le | Self::Utf16Be)
    }
}

/// 텍스트 파일로 판정되었을 때의 인코딩 감지 결과 (PLAN.md §3.3-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectedEncoding {
    /// 감지된 인코딩.
    pub encoding: TextEncoding,
    /// 파일 시작 부분에 BOM이 존재했는지 여부.
    pub has_bom: bool,
    /// BOM이나 유효한 UTF-8로 확정되지 않고 휴리스틱 또는 `chardetng`로 추정되었는지 여부.
    pub estimated: bool,
    /// `chardetng` 추정 결과가 지원 목록 밖이라 `EUC-KR (CP949)`로 대체했을 때, 원래 추정된 인코딩 이름
    /// (`encoding_rs` 표기, 예: `windows-1252`). 대체하지 않았으면 `None`.
    pub fallback_from: Option<&'static str>,
}

impl DetectedEncoding {
    const fn certain(encoding: TextEncoding, has_bom: bool) -> Self {
        Self {
            encoding,
            has_bom,
            estimated: false,
            fallback_from: None,
        }
    }

    const fn estimated(encoding: TextEncoding) -> Self {
        Self {
            encoding,
            has_bom: false,
            estimated: true,
            fallback_from: None,
        }
    }
}

/// 바이트 슬라이스에 대한 인코딩 감지 결과 (PLAN.md §3.3-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionOutcome {
    /// 텍스트 파일로 판정됨.
    Text(DetectedEncoding),
    /// 텍스트가 아닌 바이너리 데이터(`0x00` 포함)로 판정됨.
    Binary,
}

/// 바이트를 디코딩한 문서 결과 (PLAN.md §3.3-2, §3.3-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedDocument {
    /// 디코딩된 텍스트 (디코딩 오류 구간은 `U+FFFD` 대체 문자로 치환됨).
    pub text: String,
    /// 적용된 인코딩.
    pub encoding: TextEncoding,
    /// 원본 바이트 앞부분에 해당 인코딩의 BOM이 존재했는지 여부.
    ///
    /// 저장 시에는 UTF-16에만 쓰인다 (BOM 없던 UTF-16 파일 보존). UTF-8은 `Utf8`/`Utf8Bom`, EUC-KR은 BOM 없음으로
    /// 인코딩이 BOM을 결정한다 ([`DecodedDocument::encode_preserving_bom`]).
    pub has_bom: bool,
    /// 휴리스틱 또는 `chardetng`로 추정된 인코딩인지 여부.
    pub estimated: bool,
    /// 지원 목록 밖 추정 결과를 `EUC-KR (CP949)`로 대체했을 때 원래 추정된 인코딩 이름 ([`DetectedEncoding::fallback_from`]).
    pub fallback_from: Option<&'static str>,
    /// 디코딩 중 잘못된 바이트가 발견되어 `U+FFFD`로 대체되었는지 여부 ("디코딩 손상" 상태).
    pub damaged: bool,
}

/// 자동 감지 후 디코딩한 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeOutcome {
    /// 텍스트 문서로 감지·디코딩 완료.
    Text(DecodedDocument),
    /// 바이너리 파일로 판정됨.
    Binary,
}

impl DecodedDocument {
    /// 문서 인코딩으로 직렬화한다. BOM은 인코딩이 결정하고, UTF-16만 원본 BOM 유무(`has_bom`)를 보존한다.
    pub fn encode_preserving_bom(&self) -> Result<Vec<u8>, UnmappableError> {
        encode_text_with_bom(&self.text, self.encoding, self.has_bom)
    }

    /// 저장할 인코딩을 바꾼다 ("인코딩 변환 저장", PLAN.md §3.3-5).
    ///
    /// 인코딩이 바뀌면 원본 BOM 유무는 더 이상 의미가 없으므로 `has_bom`을 새 인코딩의 표준 BOM 정책으로 되돌린다
    /// (UTF-16 → BOM 포함 저장, PLAN.md §3.3-1). 같은 인코딩이면 아무것도 바꾸지 않는다.
    pub fn set_encoding(&mut self, encoding: TextEncoding) {
        if self.encoding != encoding {
            self.encoding = encoding;
            self.has_bom = !encoding.bom_bytes().is_empty();
        }
    }
}

/// 대상 인코딩으로 표현할 수 없는 단일 문자와 문서 내 위치 정보 (PLAN.md §3.3-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnmappableChar {
    /// 표현할 수 없는 유니코드 문자 (예: 이모지 `😀`, 미지원 한자 등).
    pub ch: char,
    /// 1-기반 줄 번호 (`Ln`).
    pub line: usize,
    /// 해당 줄 안에서의 1-기반 문자 열 번호 (`Col`).
    pub column: usize,
    /// 문서 전체에서의 0-기반 문자 인덱스.
    pub char_index: usize,
    /// UTF-8 문자열(`&str`)에서의 0-기반 바이트 오프셋.
    pub byte_offset: usize,
}

/// 대상 인코딩으로 저장할 때 표현 불가 문자가 발견되었음을 나타내는 오류 (PLAN.md §3.3-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmappableError {
    /// 시도한 대상 인코딩.
    pub encoding: TextEncoding,
    /// 발견된 표현 불가 문자 목록 (문서 등장 순서).
    pub unmappable: Vec<UnmappableChar>,
    /// 표현 불가 문자를 `?`(`0x3F`)로 대체하여 인코딩한 바이트 결과 (HTML 숫자 참조 `&#...;` 미포함).
    pub lossy_bytes: Vec<u8>,
}

/// 기본 인코딩을 `UTF-8`로 두어 바이트 슬라이스의 인코딩을 감지한다 (PLAN.md §3.3-2).
pub fn detect_encoding(bytes: &[u8]) -> DetectionOutcome {
    detect_encoding_with_default(bytes, TextEncoding::Utf8)
}

/// 지정한 `default_encoding`(빈 파일에 적용)을 기준으로 바이트 슬라이스의 인코딩을 감지한다.
///
/// 감지 순서 (PLAN.md §3.3-2):
/// 1. BOM 확인 (`EF BB BF` → UTF-8 BOM, `FF FE` → UTF-16 LE, `FE FF` → UTF-16 BE)
/// 2. 빈 파일 → `default_encoding`
/// 3. UTF-8 유효성 검사 (`0x00` 바이트가 없고 전체가 유효한 UTF-8이면 `UTF-8`,
///    잘못된 구간이 적거나 끝에서 잘렸으면 `UTF-8` 추정)
/// 4. BOM 없는 UTF-16 휴리스틱 (`0x00`이 있을 때만, [`detect_utf16_without_bom`] 참고)
/// 5. 바이너리 판정 (위에 해당하지 않으면서 `0x00` 바이트가 포함되면 `Binary`)
/// 6. 레거시 추정 (`chardetng`, `allow_utf8 = false`, 지원 목록 밖이면 `EUC-KR (CP949)`로 대체하고 `fallback_from` 기록)
pub fn detect_encoding_with_default(
    bytes: &[u8],
    default_encoding: TextEncoding,
) -> DetectionOutcome {
    // 1. BOM 확인
    if bytes.starts_with(UTF8_BOM) {
        return DetectionOutcome::Text(DetectedEncoding::certain(TextEncoding::Utf8Bom, true));
    }
    if bytes.starts_with(UTF16LE_BOM) {
        return DetectionOutcome::Text(DetectedEncoding::certain(TextEncoding::Utf16Le, true));
    }
    if bytes.starts_with(UTF16BE_BOM) {
        return DetectionOutcome::Text(DetectedEncoding::certain(TextEncoding::Utf16Be, true));
    }

    // 2. 빈 파일
    if bytes.is_empty() {
        return DetectionOutcome::Text(DetectedEncoding::certain(default_encoding, false));
    }

    // 3. UTF-8 유효성 검사 (ASCII 전용 파일 포함, 단 NUL(0x00)이 포함된 경우 UTF-16/바이너리 판정으로 넘김)
    let has_nul = bytes.contains(&0);
    if !has_nul {
        match utf8_validity(bytes) {
            Utf8Validity::Valid => {
                return DetectionOutcome::Text(DetectedEncoding::certain(
                    TextEncoding::Utf8,
                    false,
                ));
            }
            // 끝이 잘렸거나 잘못된 바이트가 조금 섞인 UTF-8은 레거시로 추정하면 전체가 깨지므로
            // UTF-8로 추정하고 디코딩 손상으로 표시한다.
            Utf8Validity::MostlyValid => {
                return DetectionOutcome::Text(DetectedEncoding::estimated(TextEncoding::Utf8));
            }
            Utf8Validity::Invalid => {}
        }
    }

    // 4. BOM 없는 UTF-16 휴리스틱 (ASCII·공백·줄바꿈이 있으면 0x00 바이트가 생기므로 NUL이 있을 때만 시도)
    if has_nul && let Some(utf16_enc) = detect_utf16_without_bom(bytes) {
        return DetectionOutcome::Text(DetectedEncoding::estimated(utf16_enc));
    }

    // 5. 바이너리 판정
    if has_nul {
        return DetectionOutcome::Binary;
    }

    // 6. 레거시 추정 (chardetng, allow_utf8 = false).
    // chardetng는 UTF-16을 추정하지 않으므로 결과는 EUC-KR이거나 지원 목록 밖 인코딩이다.
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let guessed = detector.guess(None, Utf8Detection::Deny);
    let mut detected = DetectedEncoding::estimated(TextEncoding::EucKr);
    if guessed != encoding_rs::EUC_KR {
        // 지원 목록 밖이면 EUC-KR (CP949)로 대체하고 원래 추정 결과를 남긴다 (PLAN.md §3.3-2 #6)
        detected.fallback_from = Some(guessed.name());
    }
    DetectionOutcome::Text(detected)
}

/// UTF-8 유효성 검사 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Utf8Validity {
    /// 전체가 유효한 UTF-8.
    Valid,
    /// 유효한 멀티바이트 문자가 대부분이고 잘못된 구간이 적거나 끝에서 잘린 경우만 있음.
    MostlyValid,
    /// UTF-8로 보기 어려움.
    Invalid,
}

/// 잘못된 구간 1개당 요구하는 유효한 멀티바이트 문자 수.
const UTF8_VALID_PER_INVALID: usize = 8;

fn utf8_validity(bytes: &[u8]) -> Utf8Validity {
    let mut multibyte_chars = 0usize;
    let mut invalid_runs = 0usize;
    let mut truncated_tail = false;
    let mut chunks = bytes.utf8_chunks().peekable();
    while let Some(chunk) = chunks.next() {
        multibyte_chars += chunk.valid().chars().filter(|c| !c.is_ascii()).count();
        if !chunk.invalid().is_empty() {
            invalid_runs += 1;
            // 마지막 청크의 잘못된 바이트가 3바이트 이하면 끝에서 잘린 멀티바이트 문자일 수 있다.
            truncated_tail = chunks.peek().is_none() && chunk.invalid().len() < 4;
        }
    }

    if invalid_runs == 0 {
        return Utf8Validity::Valid;
    }
    if multibyte_chars == 0 {
        return Utf8Validity::Invalid;
    }
    let tail_only = invalid_runs == 1 && truncated_tail;
    if tail_only || multibyte_chars >= invalid_runs.saturating_mul(UTF8_VALID_PER_INVALID) {
        Utf8Validity::MostlyValid
    } else {
        Utf8Validity::Invalid
    }
}

/// 바이트 슬라이스의 인코딩을 자동 감지하고 디코딩한다.
pub fn decode_bytes(bytes: &[u8]) -> DecodeOutcome {
    decode_bytes_with_default(bytes, TextEncoding::Utf8)
}

/// 빈 파일 기본 인코딩을 지정해 자동 감지 및 디코딩을 수행한다.
pub fn decode_bytes_with_default(bytes: &[u8], default_encoding: TextEncoding) -> DecodeOutcome {
    match detect_encoding_with_default(bytes, default_encoding) {
        DetectionOutcome::Binary => DecodeOutcome::Binary,
        DetectionOutcome::Text(detected) => {
            let mut doc = decode_with_encoding(bytes, detected.encoding);
            doc.has_bom = detected.has_bom;
            doc.estimated = detected.estimated;
            doc.fallback_from = detected.fallback_from;
            DecodeOutcome::Text(doc)
        }
    }
}

/// [`decode_bytes_with_default`]와 같지만 바이트 소유권을 넘겨받아, 유효한 UTF-8이면 복사하지 않고
/// 그대로 `String`으로 바꾼다 (BOM은 제자리에서 지움). 큰 파일을 열 때 본문 복사본을 하나 줄인다
/// (P1-MAC 1-7a 뷰어 메모리 조사). 바이너리로 판정되면 받은 바이트를 `Err`로 돌려준다.
pub fn decode_owned_with_default(
    bytes: Vec<u8>,
    default_encoding: TextEncoding,
) -> Result<DecodedDocument, Vec<u8>> {
    let detected = match detect_encoding_with_default(&bytes, default_encoding) {
        DetectionOutcome::Binary => return Err(bytes),
        DetectionOutcome::Text(detected) => detected,
    };
    let mut doc = match detected.encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => {
            let mut bytes = bytes;
            let had_bom = bytes.starts_with(UTF8_BOM);
            if had_bom {
                bytes.drain(..UTF8_BOM.len());
            }
            match String::from_utf8(bytes) {
                Ok(text) => DecodedDocument {
                    text,
                    encoding: detected.encoding,
                    has_bom: had_bom,
                    estimated: false,
                    fallback_from: None,
                    damaged: false,
                },
                // 잘못된 바이트가 있으면 기존 경로로 `U+FFFD` 치환 디코딩한다 (BOM 여부는 아래에서 감지 결과로 덮음).
                Err(err) => decode_with_encoding(err.as_bytes(), detected.encoding),
            }
        }
        encoding => decode_with_encoding(&bytes, encoding),
    };
    doc.has_bom = detected.has_bom;
    doc.estimated = detected.estimated;
    doc.fallback_from = detected.fallback_from;
    Ok(doc)
}

/// 지정한 [`TextEncoding`]으로 바이트 슬라이스를 강제 디코딩한다.
///
/// - "인코딩으로 다시 읽기"(`Command::ReopenWithEncoding`)나 바이너리 파일을 읽기 전용으로 열 때 사용한다.
/// - 잘못된 바이트 시퀀스는 `U+FFFD`로 대체되고 반환값의 `damaged` 필드가 `true`로 설정된다.
pub fn decode_with_encoding(bytes: &[u8], encoding: TextEncoding) -> DecodedDocument {
    let (has_bom, payload) = match encoding {
        // EUC-KR에는 BOM이 없으므로 앞부분 바이트를 그대로 디코딩한다.
        TextEncoding::EucKr => (false, bytes),
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => strip_bom(bytes, UTF8_BOM),
        TextEncoding::Utf16Le => strip_bom(bytes, UTF16LE_BOM),
        TextEncoding::Utf16Be => strip_bom(bytes, UTF16BE_BOM),
    };
    let (text, damaged) = match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => {
            let (cow, had_errors) = encoding_rs::UTF_8.decode_without_bom_handling(payload);
            (cow.into_owned(), had_errors)
        }
        TextEncoding::EucKr => {
            let (cow, had_errors) = encoding_rs::EUC_KR.decode_without_bom_handling(payload);
            (cow.into_owned(), had_errors)
        }
        TextEncoding::Utf16Le => decode_utf16_bytes(payload, true),
        TextEncoding::Utf16Be => decode_utf16_bytes(payload, false),
    };
    DecodedDocument {
        text,
        encoding,
        has_bom,
        estimated: false,
        fallback_from: None,
        damaged,
    }
}

fn strip_bom<'a>(bytes: &'a [u8], bom: &[u8]) -> (bool, &'a [u8]) {
    match bytes.strip_prefix(bom) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    }
}

/// BOM 없는 UTF-16 휴리스틱이 검사하는 앞부분 바이트 수.
const UTF16_SAMPLE_LEN: usize = 8192;

/// 0x00 위치가 치우치지 않은 텍스트(한글 위주)를 UTF-16으로 인정할 텍스트 문자 비율(%).
const UTF16_TEXT_RATIO_PERCENT: usize = 90;

/// BOM이 없는 바이트 슬라이스가 UTF-16 LE 또는 UTF-16 BE 텍스트인지 휴리스틱으로 판정한다.
///
/// - ASCII 위주 텍스트: 짝수/홀수 위치 중 한쪽에 `0x00`이 뚜렷하게 몰린다.
/// - 한글 위주 텍스트: `가`(U+AC00)처럼 아래 바이트가 `0x00`인 음절 때문에 `0x00`이 양쪽에 섞이므로,
///   두 엔디언으로 디코딩해 유효하고 대부분이 텍스트 문자([`is_common_text_char`])인 쪽을 고른다.
///
/// `0x00`이 없는 바이트에는 호출하지 않는다 (CP949 한글 바이트는 UTF-16 BE 한글 음절 범위와 겹친다).
fn detect_utf16_without_bom(bytes: &[u8]) -> Option<TextEncoding> {
    if bytes.len() < 2 || !bytes.len().is_multiple_of(2) {
        return None;
    }

    let truncated = bytes.len() > UTF16_SAMPLE_LEN;
    let sample = &bytes[..bytes.len().min(UTF16_SAMPLE_LEN)];
    let total_pairs = sample.len() / 2;

    let mut even_zeros = 0usize;
    let mut odd_zeros = 0usize;
    let mut double_zeros = 0usize;

    for &[b0, b1] in sample.as_chunks::<2>().0 {
        match (b0, b1) {
            (0, 0) => double_zeros += 1,
            (0, _) => even_zeros += 1,
            (_, 0) => odd_zeros += 1,
            _ => {}
        }
    }

    // UTF-16 텍스트에는 U+0000(0x00 0x00) 코드 유닛이 등장하지 않아야 한다.
    if double_zeros > 0 {
        return None;
    }

    // 짝수/홀수 위치의 0x00 비율이 최소 10% 이상이고 한쪽으로 뚜렷하게 치우친 경우
    let skewed = if odd_zeros > 0
        && odd_zeros >= even_zeros.saturating_mul(4)
        && odd_zeros * 10 >= total_pairs
    {
        Some(TextEncoding::Utf16Le)
    } else if even_zeros > 0
        && even_zeros >= odd_zeros.saturating_mul(4)
        && even_zeros * 10 >= total_pairs
    {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    };
    if let Some(candidate) = skewed {
        let little_endian = candidate == TextEncoding::Utf16Le;
        return utf16_text_stats(sample, little_endian, truncated).map(|_| candidate);
    }

    // 0x00이 양쪽 위치에 섞인 경우 (한글 위주 텍스트)
    let is_mostly_text = |&(text_chars, total_chars): &(usize, usize)| {
        text_chars * 100 >= total_chars * UTF16_TEXT_RATIO_PERCENT
    };
    let le = utf16_text_stats(sample, true, truncated).filter(is_mostly_text);
    let be = utf16_text_stats(sample, false, truncated).filter(is_mostly_text);
    match (le, be) {
        (Some(_), None) => Some(TextEncoding::Utf16Le),
        (None, Some(_)) => Some(TextEncoding::Utf16Be),
        (Some((le_text, _)), Some((be_text, _))) if le_text > be_text => {
            Some(TextEncoding::Utf16Le)
        }
        (Some((le_text, _)), Some((be_text, _))) if be_text > le_text => {
            Some(TextEncoding::Utf16Be)
        }
        _ => None,
    }
}

/// 샘플을 UTF-16으로 디코딩해 `(텍스트 문자 수, 전체 문자 수)`를 센다.
///
/// 짝 없는 서로게이트나 비인쇄 제어 문자(탭·줄바꿈 제외)가 있으면 `None`.
/// `truncated`이면 샘플 끝에서 잘린 서로게이트 쌍의 앞쪽 유닛은 무시한다.
fn utf16_text_stats(sample: &[u8], little_endian: bool, truncated: bool) -> Option<(usize, usize)> {
    let mut units: Vec<u16> = sample
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| {
            if little_endian {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            }
        })
        .collect();
    if truncated && units.last().is_some_and(|u| (0xD800..=0xDBFF).contains(u)) {
        units.pop();
    }

    let mut text_chars = 0usize;
    let mut total_chars = 0usize;
    for decoded in char::decode_utf16(units) {
        let ch = decoded.ok()?;
        if ch.is_control() && !matches!(ch, '\t' | '\n' | '\r') {
            return None;
        }
        total_chars += 1;
        if is_common_text_char(ch) {
            text_chars += 1;
        }
    }
    Some((text_chars, total_chars))
}

/// 한국어·영어 문서에 흔한 문자인지 (UTF-16 엔디언 판정용).
fn is_common_text_char(ch: char) -> bool {
    ch.is_ascii_graphic()
        || ch.is_ascii_whitespace()
        || matches!(
            ch,
            '\u{00A0}'..='\u{00FF}' // Latin-1 보충
                | '\u{1100}'..='\u{11FF}' // 한글 자모
                | '\u{2000}'..='\u{206F}' // 일반 문장 부호
                | '\u{3000}'..='\u{303F}' // CJK 기호·문장 부호
                | '\u{3130}'..='\u{318F}' // 한글 호환 자모
                | '\u{4E00}'..='\u{9FFF}' // CJK 통합 한자
                | '\u{AC00}'..='\u{D7A3}' // 한글 음절
                | '\u{FF00}'..='\u{FFEF}' // 전각·반각
        )
}

fn decode_utf16_bytes(bytes: &[u8], little_endian: bool) -> (String, bool) {
    let mut damaged = !bytes.len().is_multiple_of(2);
    let units = bytes.as_chunks::<2>().0.iter().map(|&pair| {
        if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });

    let mut out = String::with_capacity(bytes.len() / 2);
    for item in char::decode_utf16(units) {
        match item {
            Ok(ch) => out.push(ch),
            Err(_) => {
                damaged = true;
                out.push(char::REPLACEMENT_CHARACTER);
            }
        }
    }

    if !bytes.len().is_multiple_of(2) {
        out.push(char::REPLACEMENT_CHARACTER);
    }

    (out, damaged)
}

/// 텍스트를 지정한 [`TextEncoding`]의 표준 BOM 규칙(PLAN.md §3.3-1)에 따라 바이트로 인코딩한다.
///
/// - `UTF-8`, `EUC-KR (CP949)`: BOM 없음
/// - `UTF-8 with BOM`: `EF BB BF` 기록
/// - `UTF-16 LE`: `FF FE` 기록 후 리틀 엔디언 직렬화
/// - `UTF-16 BE`: `FE FF` 기록 후 빅 엔디언 직렬화
/// - 표현 불가 문자가 있으면 `Err(UnmappableError)`를 반환하며, 결과 바이트에 HTML 숫자 참조(`&#...;`)를 절대 넣지 않는다.
pub fn encode_text(text: &str, encoding: TextEncoding) -> Result<Vec<u8>, UnmappableError> {
    encode_text_with_bom(text, encoding, true)
}

/// UTF-16 BOM 기록 여부를 지정하여 텍스트를 바이트로 인코딩한다.
///
/// `include_utf16_bom`은 UTF-16에만 적용된다. BOM이 없던 UTF-16 파일(`utf16le_no_bom.txt` 등)을 원본 그대로
/// 저장할 때 `false`를 넘긴다. UTF-8은 `Utf8`/`Utf8Bom`, EUC-KR은 BOM 없음으로 인코딩이 BOM을 결정한다.
pub fn encode_text_with_bom(
    text: &str,
    encoding: TextEncoding,
    include_utf16_bom: bool,
) -> Result<Vec<u8>, UnmappableError> {
    let (bytes, unmappable) = encode_text_lossy_with_bom(text, encoding, include_utf16_bom);
    if unmappable.is_empty() {
        Ok(bytes)
    } else {
        Err(UnmappableError {
            encoding,
            unmappable,
            lossy_bytes: bytes,
        })
    }
}

/// 표현 불가 문자를 `?`(`0x3F`)로 대체하여 인코딩하고, 대체된 문자 목록을 함께 반환한다.
///
/// `encoding_rs::Encoding::encode`와 달리 HTML 숫자 참조(`&#128512;`)를 절대 생성하지 않는다.
pub fn encode_text_lossy(text: &str, encoding: TextEncoding) -> (Vec<u8>, Vec<UnmappableChar>) {
    encode_text_lossy_with_bom(text, encoding, true)
}

/// UTF-16 BOM 기록 여부를 지정해 표현 불가 문자를 `?`(`0x3F`)로 대체하며 인코딩한다 ([`encode_text_with_bom`] 참고).
pub fn encode_text_lossy_with_bom(
    text: &str,
    encoding: TextEncoding,
    include_utf16_bom: bool,
) -> (Vec<u8>, Vec<UnmappableChar>) {
    if encoding == TextEncoding::EucKr {
        return encode_euc_kr_without_ncr(text);
    }

    let bom = if encoding.is_utf16() && !include_utf16_bom {
        &[][..]
    } else {
        encoding.bom_bytes()
    };
    let body_len = if encoding.is_utf16() {
        text.len().saturating_mul(2)
    } else {
        text.len()
    };
    let mut out = Vec::with_capacity(bom.len() + body_len);
    out.extend_from_slice(bom);
    match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => out.extend_from_slice(text.as_bytes()),
        TextEncoding::Utf16Le => push_utf16_units(&mut out, text, true),
        TextEncoding::Utf16Be => push_utf16_units(&mut out, text, false),
        TextEncoding::EucKr => unreachable!("EUC-KR은 위에서 처리"),
    }
    (out, Vec::new())
}

/// 대상 인코딩으로 표현할 수 없는 문자를 최대 `limit`개까지 찾아 반환한다.
pub fn find_unmappable_chars(
    text: &str,
    encoding: TextEncoding,
    limit: usize,
) -> Vec<UnmappableChar> {
    if limit == 0 || encoding != TextEncoding::EucKr {
        return Vec::new();
    }
    let (_, mut unmappable) = encode_euc_kr_without_ncr(text);
    unmappable.truncate(limit);
    unmappable
}

/// `str::encode_utf16()`을 사용해 엔디언에 맞게 UTF-16 코드 유닛을 직접 직렬화한다 (PLAN.md §3.3-5).
fn push_utf16_units(out: &mut Vec<u8>, text: &str, little_endian: bool) {
    for unit in text.encode_utf16() {
        let pair = if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        out.extend_from_slice(&pair);
    }
}

/// `encoding_rs::EUC_KR`의 `encode_from_utf8_to_vec_without_replacement`를 사용하여
/// HTML 숫자 참조(`&#...;`) 치환 없이 표현 불가 문자를 검출하고 `?`(`0x3F`)로 대체한다.
fn encode_euc_kr_without_ncr(text: &str) -> (Vec<u8>, Vec<UnmappableChar>) {
    let mut encoder = encoding_rs::EUC_KR.new_encoder();
    let initial_cap = encoder
        .max_buffer_length_from_utf8_without_replacement(text.len())
        .unwrap_or(text.len().saturating_mul(2))
        .max(16);
    let mut out = Vec::with_capacity(initial_cap);
    let mut unmappable_offsets: Vec<(usize, char)> = Vec::new();

    let mut total_read = 0usize;
    while total_read < text.len() {
        let remaining = &text[total_read..];
        let needed = encoder
            .max_buffer_length_from_utf8_without_replacement(remaining.len())
            .unwrap_or(remaining.len().saturating_mul(2))
            .max(16);
        if out.capacity() - out.len() < needed {
            out.reserve(needed);
        }

        let (result, read) =
            encoder.encode_from_utf8_to_vec_without_replacement(remaining, &mut out, true);
        total_read += read;

        match result {
            EncoderResult::InputEmpty => break,
            EncoderResult::OutputFull => {
                out.reserve(needed.max(64));
            }
            EncoderResult::Unmappable(ch) => {
                let char_byte_offset = total_read.saturating_sub(ch.len_utf8());
                unmappable_offsets.push((char_byte_offset, ch));
                out.push(b'?');
            }
        }
    }

    if unmappable_offsets.is_empty() {
        return (out, Vec::new());
    }

    // 표현 불가 문자가 있을 때만 1회 스캔하여 (줄 번호, 열 번호, 문자 인덱스)를 채운다.
    let mut unmappable = Vec::with_capacity(unmappable_offsets.len());
    let mut target_idx = 0usize;
    let mut line = 1usize;
    let mut column = 1usize;

    for (char_index, (byte_offset, ch)) in text.char_indices().enumerate() {
        if target_idx < unmappable_offsets.len() && unmappable_offsets[target_idx].0 == byte_offset
        {
            unmappable.push(UnmappableChar {
                ch,
                line,
                column,
                char_index,
                byte_offset,
            });
            target_idx += 1;
            if target_idx == unmappable_offsets.len() {
                break;
            }
        }

        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    (out, unmappable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    const SAMPLE_KOREAN_TEXT: &str = "My-Notepad 인코딩 픽스처\r\n\
1. 기본 한글: 다람쥐 헌 쳇바퀴에 타고파.\r\n\
2. 확장 완성형(CP949): 뷁 쀍 힣 똠방각하\r\n\
3. English & Digits: 0123456789\r\n";

    const SAMPLE_ASCII_TEXT: &str = "My-Notepad ASCII fixture\r\n\
The quick brown fox jumps over the lazy dog 0123456789.\r\n";

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("encoding")
    }

    fn read_fixture(name: &str) -> Vec<u8> {
        let path = fixtures_dir().join(name);
        fs::read(&path)
            .unwrap_or_else(|e| panic!("픽스처 파일 읽기 실패 ({}): {e}", path.display()))
    }

    #[test]
    fn detect_and_decode_all_encoding_fixtures() {
        // 1. UTF-8 (BOM 없음)
        let doc = match decode_bytes(&read_fixture("utf8.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("utf8.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::Utf8);
        assert!(!doc.has_bom);
        assert!(!doc.estimated);
        assert!(!doc.damaged);
        assert_eq!(doc.text, SAMPLE_KOREAN_TEXT);

        // 2. UTF-8 with BOM
        let doc = match decode_bytes(&read_fixture("utf8_bom.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("utf8_bom.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::Utf8Bom);
        assert!(doc.has_bom);
        assert!(!doc.estimated);
        assert!(!doc.damaged);
        assert_eq!(doc.text, SAMPLE_KOREAN_TEXT);

        // 3. CP949 (메모장 ANSI 저장본)
        let doc = match decode_bytes(&read_fixture("cp949.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("cp949.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::EucKr);
        assert!(!doc.has_bom);
        assert!(doc.estimated);
        assert!(!doc.damaged);
        assert_eq!(doc.text, SAMPLE_KOREAN_TEXT);

        // 4. UTF-16 LE (BOM 있음)
        let doc = match decode_bytes(&read_fixture("utf16le.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("utf16le.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::Utf16Le);
        assert!(doc.has_bom);
        assert!(!doc.estimated);
        assert!(!doc.damaged);
        assert_eq!(doc.text, SAMPLE_KOREAN_TEXT);

        // 5. UTF-16 BE (BOM 있음)
        let doc = match decode_bytes(&read_fixture("utf16be.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("utf16be.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::Utf16Be);
        assert!(doc.has_bom);
        assert!(!doc.estimated);
        assert!(!doc.damaged);
        assert_eq!(doc.text, SAMPLE_KOREAN_TEXT);

        // 6. UTF-16 LE (BOM 없음 — 휴리스틱 감지)
        let doc = match decode_bytes(&read_fixture("utf16le_no_bom.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("utf16le_no_bom.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::Utf16Le);
        assert!(!doc.has_bom);
        assert!(doc.estimated);
        assert!(!doc.damaged);
        assert_eq!(doc.text, SAMPLE_KOREAN_TEXT);

        // 7. UTF-16 BE (BOM 없음 — 휴리스틱 감지)
        let doc = match decode_bytes(&read_fixture("utf16be_no_bom.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("utf16be_no_bom.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::Utf16Be);
        assert!(!doc.has_bom);
        assert!(doc.estimated);
        assert!(!doc.damaged);
        assert_eq!(doc.text, SAMPLE_KOREAN_TEXT);

        // 8. ASCII 전용 파일 -> UTF-8로 감지
        let doc = match decode_bytes(&read_fixture("ascii.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("ascii.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::Utf8);
        assert!(!doc.has_bom);
        assert!(!doc.estimated);
        assert!(!doc.damaged);
        assert_eq!(doc.text, SAMPLE_ASCII_TEXT);

        // 9. 빈 파일 -> 기본 인코딩(UTF-8)
        let doc = match decode_bytes(&read_fixture("empty.txt")) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("empty.txt가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, TextEncoding::Utf8);
        assert!(!doc.has_bom);
        assert!(!doc.estimated);
        assert!(!doc.damaged);
        assert!(doc.text.is_empty());

        // 설정 기본 인코딩이 EUC-KR일 때 빈 파일 감지 확인
        let doc_custom =
            match decode_bytes_with_default(&read_fixture("empty.txt"), TextEncoding::EucKr) {
                DecodeOutcome::Text(doc) => doc,
                DecodeOutcome::Binary => panic!("empty.txt가 바이너리로 판정됨"),
            };
        assert_eq!(doc_custom.encoding, TextEncoding::EucKr);

        // 10. 바이너리 파일
        assert_eq!(
            detect_encoding(&read_fixture("binary.bin")),
            DetectionOutcome::Binary
        );
        assert_eq!(
            decode_bytes(&read_fixture("binary.bin")),
            DecodeOutcome::Binary
        );
    }

    #[test]
    fn damaged_cp949_sets_damaged_flag_and_inserts_replacement_char() {
        let bytes = read_fixture("cp949_damaged.bin");
        let doc = match decode_bytes(&bytes) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("cp949_damaged.bin이 바이너리로 판정됨"),
        };

        assert_eq!(doc.encoding, TextEncoding::EucKr);
        assert!(doc.estimated);
        assert!(
            doc.damaged,
            "잘못된 바이트가 섞인 CP949는 damaged 플래그가 true여야 함"
        );
        assert!(
            doc.text.contains(char::REPLACEMENT_CHARACTER),
            "손상된 바이트 위치에 U+FFFD 대체 문자가 포함되어야 함"
        );
        assert!(doc.text.starts_with("정상 한글 앞부분\r\n손상 구간: "));
        assert!(doc.text.ends_with("정상 한글 뒷부분\r\n"));
    }

    #[test]
    fn decode_with_encoding_marks_invalid_utf16_as_damaged() {
        // 홀수 길이 및 짝 없는 서로게이트(0xD800) 포함 UTF-16 LE
        let bad_utf16le = [0x41, 0x00, 0x00, 0xD8, 0x42, 0x00, 0xFF];
        let doc = decode_with_encoding(&bad_utf16le, TextEncoding::Utf16Le);
        assert_eq!(doc.encoding, TextEncoding::Utf16Le);
        assert!(doc.damaged);
        assert!(doc.text.contains(char::REPLACEMENT_CHARACTER));
        assert!(doc.text.starts_with('A'));
    }

    #[test]
    fn decode_owned_matches_borrowed_decode_for_all_fixtures() {
        let mut fixtures: Vec<(String, Vec<u8>)> = fs::read_dir(fixtures_dir())
            .expect("픽스처 폴더")
            .map(|entry| {
                let name = entry.expect("픽스처 항목").file_name();
                let name = name.to_string_lossy().into_owned();
                let bytes = read_fixture(&name);
                (name, bytes)
            })
            .collect();
        // 잘못된 UTF-8 + BOM: 소유 경로의 `U+FFFD` 대체 분기
        fixtures.push((
            "utf8_bom_damaged".to_owned(),
            vec![0xEF, 0xBB, 0xBF, b'a', 0xFF, b'b'],
        ));
        for (name, bytes) in fixtures {
            for default in [TextEncoding::Utf8, TextEncoding::EucKr] {
                let borrowed = decode_bytes_with_default(&bytes, default);
                let owned = decode_owned_with_default(bytes.clone(), default);
                match (borrowed, owned) {
                    (DecodeOutcome::Text(b), Ok(o)) => assert_eq!(b, o, "{name}"),
                    (DecodeOutcome::Binary, Err(back)) => assert_eq!(back, bytes, "{name}"),
                    (b, o) => panic!("{name}: 판정이 다름 {b:?} / {o:?}"),
                }
            }
        }
    }

    fn decode_text(bytes: &[u8]) -> DecodedDocument {
        match decode_bytes(bytes) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("텍스트가 바이너리로 판정됨: {bytes:02X?}"),
        }
    }

    fn utf16_bytes(text: &str, little_endian: bool) -> Vec<u8> {
        text.encode_utf16()
            .flat_map(|u| {
                if little_endian {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                }
            })
            .collect()
    }

    #[test]
    fn utf8_truncated_at_end_is_detected_as_damaged_utf8() {
        // 멀티바이트 문자 중간에서 잘린 로그 파일
        let mut bytes = "한글 로그 첫 줄\n둘째 줄 끝".as_bytes().to_vec();
        bytes.pop();
        let doc = decode_text(&bytes);
        assert_eq!(doc.encoding, TextEncoding::Utf8);
        assert!(doc.estimated);
        assert!(doc.damaged);
        assert_eq!(doc.fallback_from, None);
        assert!(doc.text.starts_with("한글 로그 첫 줄\n둘째 줄 "));
        assert!(doc.text.ends_with(char::REPLACEMENT_CHARACTER));
    }

    #[test]
    fn utf8_with_few_invalid_bytes_is_detected_as_damaged_utf8() {
        let mut bytes = "한글 문서 앞부분입니다. ".as_bytes().to_vec();
        bytes.push(0xFF);
        bytes.extend_from_slice("한글 문서 뒷부분입니다.".as_bytes());
        let doc = decode_text(&bytes);
        assert_eq!(doc.encoding, TextEncoding::Utf8);
        assert!(doc.estimated);
        assert!(doc.damaged);
        assert_eq!(
            doc.text,
            "한글 문서 앞부분입니다. \u{FFFD}한글 문서 뒷부분입니다."
        );
    }

    #[test]
    fn cp949_is_not_mistaken_for_damaged_utf8_or_utf16() {
        // 짝수 길이의 순수 한글 CP949 (바이트 쌍이 UTF-16 BE 한글 음절 범위와 겹침)
        let (bytes, _, unmappable) = encoding_rs::EUC_KR.encode("가나다라마바사아자차카타파하");
        assert!(!unmappable);
        assert!(bytes.len().is_multiple_of(2));
        let doc = decode_text(&bytes);
        assert_eq!(doc.encoding, TextEncoding::EucKr);
        assert_eq!(doc.fallback_from, None);
        assert!(!doc.damaged);

        let doc = decode_text(&read_fixture("cp949.txt"));
        assert_eq!(doc.encoding, TextEncoding::EucKr);
        assert_eq!(doc.fallback_from, None);
    }

    #[test]
    fn hangul_only_utf16_without_bom_is_detected() {
        // '가'(U+AC00)·'간'(U+AC04)… 아래 바이트가 0x00인 음절이 섞여 0x00이 짝수/홀수 위치에 모두 나타남
        let texts = [
            "가나다라마바사아자차카타파하가나다라마바사",
            "가각간갇갈감갑값갓갔강갖갗같",
            "가나다 한글 문서\r\n두 번째 줄입니다\r\n",
        ];
        for text in texts {
            for little_endian in [true, false] {
                let doc = decode_text(&utf16_bytes(text, little_endian));
                let expected = if little_endian {
                    TextEncoding::Utf16Le
                } else {
                    TextEncoding::Utf16Be
                };
                assert_eq!(doc.encoding, expected, "{text:?}");
                assert!(doc.estimated);
                assert!(!doc.damaged);
                assert_eq!(doc.text, text);
            }
        }
    }

    #[test]
    fn utf16_sample_boundary_inside_surrogate_pair_is_still_utf16() {
        // 샘플 끝(8192바이트)이 서로게이트 쌍 가운데에 걸리는 경우
        let mut text = "a".repeat(UTF16_SAMPLE_LEN / 2 - 1);
        text.push('😀');
        text.push_str("bcd");
        let doc = decode_text(&utf16_bytes(&text, true));
        assert_eq!(doc.encoding, TextEncoding::Utf16Le);
        assert!(!doc.damaged);
        assert_eq!(doc.text, text);
    }

    #[test]
    fn unsupported_legacy_guess_falls_back_to_euc_kr_with_source_name() {
        // Windows-1252(라틴) 문서: 지원 목록 밖 → EUC-KR (CP949)로 대체하고 원래 추정 결과를 남김
        let bytes = b"Caf\xE9 cr\xE8me br\xFBl\xE9e, na\xEFve r\xE9sum\xE9 \xE0 la fran\xE7aise.";
        let doc = decode_text(bytes);
        assert_eq!(doc.encoding, TextEncoding::EucKr);
        assert!(doc.estimated);
        let source = doc
            .fallback_from
            .expect("대체 전 추정 인코딩이 기록되어야 함");
        assert_ne!(source, encoding_rs::EUC_KR.name());
    }

    #[test]
    fn binary_with_nul_is_still_binary() {
        assert_eq!(detect_encoding(b"hello\0world"), DetectionOutcome::Binary);
        let mut bytes = vec![0u8; 64];
        bytes.extend((0..=255u8).cycle().take(512));
        assert_eq!(detect_encoding(&bytes), DetectionOutcome::Binary);
    }
}
