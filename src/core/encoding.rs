//! 문자 인코딩 감지·디코딩·인코딩 (PLAN.md §3.3 — P1-04, P1-05).
//!
//! P1-04 구현 범위:
//! - 지원 인코딩 열거형 [`TextEncoding`] (`UTF-8`, `UTF-8 with BOM`, `EUC-KR (CP949)`, `UTF-16 LE`, `UTF-16 BE`)
//! - 열기 시 6단계 인코딩 감지 ([`detect_encoding`], [`detect_encoding_with_default`]):
//!   1. BOM 확인 (`EF BB BF` → UTF-8 BOM, `FF FE` → UTF-16 LE, `FE FF` → UTF-16 BE)
//!   2. 빈 파일 → 설정의 기본 인코딩(기본값 `UTF-8`)
//!   3. UTF-8 유효성 검사 (`0x00` 바이트가 없고 전체가 유효한 UTF-8이면 `UTF-8`, ASCII 전용 포함)
//!   4. BOM 없는 UTF-16 휴리스틱 (짝수/홀수 위치 `0x00` 비율 및 유효 코드 유닛 판정)
//!   5. 바이너리 판정 (위에 해당하지 않으면서 `0x00` 바이트가 포함되면 [`DetectionOutcome::Binary`])
//!   6. 레거시 추정 (`chardetng`, `allow_utf8 = false`, 지원 목록 밖이면 `EUC-KR (CP949)`로 대체 + `estimated = true`)
//! - 디코딩 및 오류 처리 ([`decode_bytes`], [`decode_with_encoding`]):
//!   잘못된 바이트 시퀀스를 `U+FFFD`(`\u{FFFD}`)로 치환하고 `damaged = true` 플래그를 기록한다.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};

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
    pub has_bom: bool,
    /// 휴리스틱 또는 `chardetng`로 추정된 인코딩인지 여부.
    pub estimated: bool,
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

/// 기본 인코딩을 `UTF-8`로 두어 바이트 슬라이스의 인코딩을 감지한다 (PLAN.md §3.3-2).
pub fn detect_encoding(bytes: &[u8]) -> DetectionOutcome {
    detect_encoding_with_default(bytes, TextEncoding::Utf8)
}

/// 지정한 `default_encoding`(빈 파일에 적용)을 기준으로 바이트 슬라이스의 인코딩을 감지한다.
///
/// 감지 순서 (PLAN.md §3.3-2):
/// 1. BOM 확인 (`EF BB BF` → UTF-8 BOM, `FF FE` → UTF-16 LE, `FE FF` → UTF-16 BE)
/// 2. 빈 파일 → `default_encoding`
/// 3. UTF-8 유효성 검사 (`0x00` 바이트가 없고 전체가 유효한 UTF-8이면 `UTF-8`)
/// 4. BOM 없는 UTF-16 휴리스틱 (짝수/홀수 바이트 위치의 `0x00` 비율로 판정)
/// 5. 바이너리 판정 (위에 해당하지 않으면서 `0x00` 바이트가 포함되면 `Binary`)
/// 6. 레거시 추정 (`chardetng`, `allow_utf8 = false`, 지원 목록 밖이면 `EUC-KR (CP949)`로 대체)
pub fn detect_encoding_with_default(
    bytes: &[u8],
    default_encoding: TextEncoding,
) -> DetectionOutcome {
    // 1. BOM 확인
    if bytes.starts_with(UTF8_BOM) {
        return DetectionOutcome::Text(DetectedEncoding {
            encoding: TextEncoding::Utf8Bom,
            has_bom: true,
            estimated: false,
        });
    }
    if bytes.starts_with(UTF16LE_BOM) {
        return DetectionOutcome::Text(DetectedEncoding {
            encoding: TextEncoding::Utf16Le,
            has_bom: true,
            estimated: false,
        });
    }
    if bytes.starts_with(UTF16BE_BOM) {
        return DetectionOutcome::Text(DetectedEncoding {
            encoding: TextEncoding::Utf16Be,
            has_bom: true,
            estimated: false,
        });
    }

    // 2. 빈 파일
    if bytes.is_empty() {
        return DetectionOutcome::Text(DetectedEncoding {
            encoding: default_encoding,
            has_bom: false,
            estimated: false,
        });
    }

    // 3. UTF-8 유효성 검사 (ASCII 전용 파일 포함, 단 NUL(0x00)이 포함된 경우 UTF-16/바이너리 판정으로 넘김)
    let has_nul = bytes.contains(&0);
    if !has_nul && std::str::from_utf8(bytes).is_ok() {
        return DetectionOutcome::Text(DetectedEncoding {
            encoding: TextEncoding::Utf8,
            has_bom: false,
            estimated: false,
        });
    }

    // 4. BOM 없는 UTF-16 휴리스틱
    if let Some(utf16_enc) = detect_utf16_without_bom(bytes) {
        return DetectionOutcome::Text(DetectedEncoding {
            encoding: utf16_enc,
            has_bom: false,
            estimated: true,
        });
    }

    // 5. 바이너리 판정
    if has_nul {
        return DetectionOutcome::Binary;
    }

    // 6. 레거시 추정 (chardetng, allow_utf8 = false)
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let guessed = detector.guess(None, Utf8Detection::Deny);
    let encoding = if guessed == encoding_rs::EUC_KR {
        TextEncoding::EucKr
    } else if guessed == encoding_rs::UTF_16LE {
        TextEncoding::Utf16Le
    } else if guessed == encoding_rs::UTF_16BE {
        TextEncoding::Utf16Be
    } else {
        // 지원 목록 밖이면 EUC-KR (CP949)로 대체 (PLAN.md §3.3-2 #6)
        TextEncoding::EucKr
    };

    DetectionOutcome::Text(DetectedEncoding {
        encoding,
        has_bom: false,
        estimated: true,
    })
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
            DecodeOutcome::Text(doc)
        }
    }
}

/// 지정한 [`TextEncoding`]으로 바이트 슬라이스를 강제 디코딩한다.
///
/// - "인코딩으로 다시 읽기"(`Command::ReopenWithEncoding`)나 바이너리 파일을 읽기 전용으로 열 때 사용한다.
/// - 잘못된 바이트 시퀀스는 `U+FFFD`로 대체되고 반환값의 `damaged` 필드가 `true`로 설정된다.
pub fn decode_with_encoding(bytes: &[u8], encoding: TextEncoding) -> DecodedDocument {
    match encoding {
        TextEncoding::Utf8 => {
            let (has_bom, payload) = match bytes.strip_prefix(UTF8_BOM) {
                Some(rest) => (true, rest),
                None => (false, bytes),
            };
            let (cow, had_errors) = encoding_rs::UTF_8.decode_without_bom_handling(payload);
            DecodedDocument {
                text: cow.into_owned(),
                encoding: TextEncoding::Utf8,
                has_bom,
                estimated: false,
                damaged: had_errors,
            }
        }
        TextEncoding::Utf8Bom => {
            let (has_bom, payload) = match bytes.strip_prefix(UTF8_BOM) {
                Some(rest) => (true, rest),
                None => (false, bytes),
            };
            let (cow, had_errors) = encoding_rs::UTF_8.decode_without_bom_handling(payload);
            DecodedDocument {
                text: cow.into_owned(),
                encoding: TextEncoding::Utf8Bom,
                has_bom,
                estimated: false,
                damaged: had_errors,
            }
        }
        TextEncoding::EucKr => {
            let (cow, had_errors) = encoding_rs::EUC_KR.decode_without_bom_handling(bytes);
            DecodedDocument {
                text: cow.into_owned(),
                encoding: TextEncoding::EucKr,
                has_bom: false,
                estimated: false,
                damaged: had_errors,
            }
        }
        TextEncoding::Utf16Le => {
            let (has_bom, payload) = match bytes.strip_prefix(UTF16LE_BOM) {
                Some(rest) => (true, rest),
                None => (false, bytes),
            };
            let (text, damaged) = decode_utf16_bytes(payload, true);
            DecodedDocument {
                text,
                encoding: TextEncoding::Utf16Le,
                has_bom,
                estimated: false,
                damaged,
            }
        }
        TextEncoding::Utf16Be => {
            let (has_bom, payload) = match bytes.strip_prefix(UTF16BE_BOM) {
                Some(rest) => (true, rest),
                None => (false, bytes),
            };
            let (text, damaged) = decode_utf16_bytes(payload, false);
            DecodedDocument {
                text,
                encoding: TextEncoding::Utf16Be,
                has_bom,
                estimated: false,
                damaged,
            }
        }
    }
}

/// BOM이 없는 바이트 슬라이스가 UTF-16 LE 또는 UTF-16 BE 텍스트인지 휴리스틱으로 판정한다.
fn detect_utf16_without_bom(bytes: &[u8]) -> Option<TextEncoding> {
    if bytes.len() < 2 || !bytes.len().is_multiple_of(2) {
        return None;
    }

    let sample_len = bytes.len().min(8192);
    let sample = &bytes[..sample_len];
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

    // 짝수/홀수 위치의 0x00 비율이 최소 10% 이상이고 한쪽으로 뚜렷하게 치우쳐야 한다.
    let (candidate, little_endian) = if odd_zeros > 0
        && odd_zeros >= even_zeros.saturating_mul(4)
        && odd_zeros * 10 >= total_pairs
    {
        (TextEncoding::Utf16Le, true)
    } else if even_zeros > 0
        && even_zeros >= odd_zeros.saturating_mul(4)
        && even_zeros * 10 >= total_pairs
    {
        (TextEncoding::Utf16Be, false)
    } else {
        return None;
    };

    // 샘플 구간이 유효한 UTF-16 시퀀스이고 비인쇄 제어 문자가 없는지 확인한다.
    let units = sample.as_chunks::<2>().0.iter().map(|&pair| {
        if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });

    for decoded in char::decode_utf16(units) {
        let ch = decoded.ok()?;
        if ch.is_control() && !matches!(ch, '\t' | '\n' | '\r') {
            return None;
        }
    }

    Some(candidate)
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
}
