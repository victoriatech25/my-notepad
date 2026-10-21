//! P1-05 인코딩 저장(인코딩) 및 라운드트립 통합 테스트 (PLAN.md §3.3-5, §3.3-6).
//!
//! 검증 항목 (WORKPLAN.md P1-05):
//! 1. 모든 텍스트 픽스처("읽기 → 저장 → 바이트 동일") 라운드트립 통과
//! 2. 이모지·미지원 문자 포함 텍스트를 EUC-KR로 저장 시도 → 표현 불가 문자 목록(`UnmappableChar`) 반환,
//!    손실 저장 바이트(`lossy_bytes` / `encode_text_lossy`)에 HTML 숫자 참조(`&#`)가 절대 포함되지 않고 `?`로만 대체됨
//! 3. UTF-16 LE / UTF-16 BE 저장 결과의 BOM(`FF FE` / `FE FF`) 및 엔디언(리틀/빅, 서로게이트 쌍 포함) 검증
//! 4. BOM은 인코딩이 결정하고 UTF-16만 원본 BOM 유무를 보존하며, 인코딩 변환 시 표준 BOM 정책으로 초기화

use std::fs;
use std::path::{Path, PathBuf};

use my_notepad::core::encoding::{
    DecodeOutcome, DecodedDocument, TextEncoding, UTF8_BOM, UTF16BE_BOM, UTF16LE_BOM,
    UnmappableChar, decode_bytes, decode_bytes_with_default, decode_with_encoding, encode_text,
    encode_text_lossy, encode_text_with_bom, find_unmappable_chars,
};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("encoding")
        .join(name)
}

#[test]
fn all_text_fixtures_roundtrip_byte_for_byte() {
    // 표준 BOM 정책을 따르는 픽스처: encode_text 와 encode_preserving_bom 모두 원본과 바이트가 100% 동일해야 한다.
    let standard_fixtures = [
        ("utf8.txt", TextEncoding::Utf8),
        ("utf8_bom.txt", TextEncoding::Utf8Bom),
        ("cp949.txt", TextEncoding::EucKr),
        ("utf16le.txt", TextEncoding::Utf16Le),
        ("utf16be.txt", TextEncoding::Utf16Be),
        ("ascii.txt", TextEncoding::Utf8),
        ("empty.txt", TextEncoding::Utf8),
    ];

    for (name, expected_encoding) in standard_fixtures {
        let original_bytes =
            fs::read(fixture_path(name)).unwrap_or_else(|e| panic!("{name} 픽스처 읽기 실패: {e}"));

        let doc = match decode_bytes(&original_bytes) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("{name} 픽스처가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, expected_encoding, "{name} 인코딩 감지 불일치");
        assert!(!doc.damaged, "{name} 디코딩 손상 없어야 함");

        let encoded = encode_text(&doc.text, doc.encoding)
            .unwrap_or_else(|e| panic!("{name} 재인코딩 실패: {e:?}"));
        assert_eq!(
            encoded, original_bytes,
            "{name}: encode_text 라운드트립 바이트가 원본과 다릅니다"
        );

        let preserved = doc
            .encode_preserving_bom()
            .unwrap_or_else(|e| panic!("{name} encode_preserving_bom 실패: {e:?}"));
        assert_eq!(
            preserved, original_bytes,
            "{name}: encode_preserving_bom 라운드트립 바이트가 원본과 다릅니다"
        );
    }

    // BOM 없는 UTF-16 픽스처:
    // - encode_preserving_bom()은 원본(BOM 없음)과 바이트가 100% 동일해야 한다.
    // - encode_text()는 PLAN.md §3.3-1("UTF-16 LE / UTF-16 BE (BOM 포함 저장)")에 따라 앞에 BOM이 붙고 본문 바이트가 동일해야 한다.
    let bomless_utf16_fixtures = [
        ("utf16le_no_bom.txt", TextEncoding::Utf16Le, UTF16LE_BOM),
        ("utf16be_no_bom.txt", TextEncoding::Utf16Be, UTF16BE_BOM),
    ];

    for (name, expected_encoding, expected_bom) in bomless_utf16_fixtures {
        let original_bytes =
            fs::read(fixture_path(name)).unwrap_or_else(|e| panic!("{name} 픽스처 읽기 실패: {e}"));

        let doc = match decode_bytes(&original_bytes) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("{name} 픽스처가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, expected_encoding);
        assert!(!doc.has_bom);
        assert!(!doc.damaged);

        let preserved = doc
            .encode_preserving_bom()
            .unwrap_or_else(|e| panic!("{name} encode_preserving_bom 실패: {e:?}"));
        assert_eq!(
            preserved, original_bytes,
            "{name}: BOM 유무 보존 라운드트립 바이트가 원본과 다릅니다"
        );

        let with_standard_bom = encode_text(&doc.text, doc.encoding)
            .unwrap_or_else(|e| panic!("{name} encode_text 실패: {e:?}"));
        assert!(with_standard_bom.starts_with(expected_bom));
        assert_eq!(
            &with_standard_bom[expected_bom.len()..],
            &original_bytes[..]
        );
    }
}

#[test]
fn euc_kr_detects_unmappable_emojis_and_never_emits_html_ncr() {
    let text = "첫 줄: 안녕하세요 😀 반갑습니다\n둘째 줄: 로켓 🚀 끝";

    // 1. 엄격 인코딩 시도 -> Err(UnmappableError) 반환 및 문자·위치(줄/열) 검증
    let err = encode_text(text, TextEncoding::EucKr)
        .expect_err("이모지가 포함된 텍스트는 EUC-KR 인코딩 시 UnmappableError를 반환해야 함");
    assert_eq!(err.encoding, TextEncoding::EucKr);
    assert_eq!(
        err.unmappable,
        vec![
            UnmappableChar {
                ch: '😀',
                line: 1,
                column: 12,
                char_index: 11,
                byte_offset: "첫 줄: 안녕하세요 ".len(),
            },
            UnmappableChar {
                ch: '🚀',
                line: 2,
                column: 10,
                char_index: 28,
                byte_offset: "첫 줄: 안녕하세요 😀 반갑습니다\n둘째 줄: 로켓 ".len(),
            },
        ]
    );

    // 2. 결과 바이트(lossy_bytes 및 encode_text_lossy)에 HTML 숫자 참조("&#")가 절대 포함되지 않아야 함
    assert!(
        !err.lossy_bytes.windows(2).any(|w| w == b"&#"),
        "EUC-KR 손실 저장 바이트에 HTML 숫자 참조(&#)가 포함되어 있습니다"
    );

    let (lossy, list) = encode_text_lossy(text, TextEncoding::EucKr);
    assert_eq!(lossy, err.lossy_bytes);
    assert_eq!(list, err.unmappable);

    // 3. `?`로 대체된 바이트를 다시 EUC-KR로 디코딩하면 이모지 자리가 `?`로 바뀌어 있어야 함
    let decoded = match decode_bytes(&lossy) {
        DecodeOutcome::Text(doc) => doc,
        DecodeOutcome::Binary => panic!("EUC-KR 손실 저장 결과가 바이너리로 판정됨"),
    };
    assert_eq!(
        decoded.text,
        "첫 줄: 안녕하세요 ? 반갑습니다\n둘째 줄: 로켓 ? 끝"
    );
    assert!(!decoded.text.contains("&#"));

    // 4. find_unmappable_chars limit 동작 확인
    let first_only = find_unmappable_chars(text, TextEncoding::EucKr, 1);
    assert_eq!(first_only.len(), 1);
    assert_eq!(first_only[0].ch, '😀');
}

#[test]
fn utf8_bom_and_utf16_le_be_bom_and_endianness_are_verified() {
    // 'A' (U+0041), '한' (U+D55C), '😀' (U+1F600 -> 서로게이트 쌍 D83D DE00)
    let sample = "A한😀";

    // UTF-8 with BOM
    let utf8_bom = encode_text(sample, TextEncoding::Utf8Bom).unwrap();
    assert!(utf8_bom.starts_with(UTF8_BOM));
    assert_eq!(&utf8_bom[UTF8_BOM.len()..], sample.as_bytes());

    // UTF-16 LE: BOM(FF FE) + 41 00 + 5C D5 + 3D D8 00 DE
    let utf16le = encode_text(sample, TextEncoding::Utf16Le).unwrap();
    assert_eq!(
        utf16le,
        vec![
            0xFF, 0xFE, // UTF-16 LE BOM
            0x41, 0x00, // 'A' (U+0041)
            0x5C, 0xD5, // '한' (U+D55C)
            0x3D, 0xD8, 0x00, 0xDE, // '😀' (U+1F600 = D83D DE00)
        ]
    );

    // UTF-16 BE: BOM(FE FF) + 00 41 + D5 5C + D8 3D DE 00
    let utf16be = encode_text(sample, TextEncoding::Utf16Be).unwrap();
    assert_eq!(
        utf16be,
        vec![
            0xFE, 0xFF, // UTF-16 BE BOM
            0x00, 0x41, // 'A' (U+0041)
            0xD5, 0x5C, // '한' (U+D55C)
            0xD8, 0x3D, 0xDE, 0x00, // '😀' (U+1F600 = D83D DE00)
        ]
    );

    // 디코딩 라운드트립 일치 확인
    for (bytes, expected_enc) in [
        (&utf8_bom, TextEncoding::Utf8Bom),
        (&utf16le, TextEncoding::Utf16Le),
        (&utf16be, TextEncoding::Utf16Be),
    ] {
        let doc = match decode_bytes(bytes) {
            DecodeOutcome::Text(doc) => doc,
            DecodeOutcome::Binary => panic!("{expected_enc:?} 결과가 바이너리로 판정됨"),
        };
        assert_eq!(doc.encoding, expected_enc);
        assert!(doc.has_bom);
        assert!(!doc.damaged);
        assert_eq!(doc.text, sample);
    }
}

fn decode_text(bytes: &[u8], default_encoding: TextEncoding) -> DecodedDocument {
    match decode_bytes_with_default(bytes, default_encoding) {
        DecodeOutcome::Text(doc) => doc,
        DecodeOutcome::Binary => panic!("텍스트가 바이너리로 판정됨: {bytes:02X?}"),
    }
}

#[test]
fn empty_file_with_utf8_bom_default_saves_bom() {
    // 기본 인코딩이 UTF-8 with BOM이면 빈 파일도 BOM을 기록해 다시 열었을 때 같은 인코딩으로 감지되어야 한다.
    let doc = decode_text(b"", TextEncoding::Utf8Bom);
    assert_eq!(doc.encoding, TextEncoding::Utf8Bom);
    let saved = doc.encode_preserving_bom().unwrap();
    assert_eq!(saved, UTF8_BOM);
    assert_eq!(
        decode_text(&saved, TextEncoding::Utf8).encoding,
        TextEncoding::Utf8Bom
    );
}

#[test]
fn converting_encoding_resets_bom_to_standard_policy() {
    let text = "한글 abc\n";

    // UTF-8 → UTF-16 LE / BE: BOM 포함 저장 (PLAN.md §3.3-1)
    for (target, bom) in [
        (TextEncoding::Utf16Le, UTF16LE_BOM),
        (TextEncoding::Utf16Be, UTF16BE_BOM),
    ] {
        let mut doc = decode_text(text.as_bytes(), TextEncoding::Utf8);
        doc.set_encoding(target);
        let saved = doc.encode_preserving_bom().unwrap();
        assert!(
            saved.starts_with(bom),
            "{target:?} 변환 저장에 BOM이 없습니다"
        );
        assert_eq!(decode_text(&saved, TextEncoding::Utf8).text, text);
    }

    // UTF-8 with BOM → UTF-8: BOM 제거
    let mut bom_bytes = UTF8_BOM.to_vec();
    bom_bytes.extend_from_slice(text.as_bytes());
    let mut doc = decode_text(&bom_bytes, TextEncoding::Utf8);
    assert!(doc.has_bom);
    doc.set_encoding(TextEncoding::Utf8);
    assert_eq!(doc.encode_preserving_bom().unwrap(), text.as_bytes());

    // UTF-8 → UTF-8 with BOM: BOM 추가
    let mut doc = decode_text(text.as_bytes(), TextEncoding::Utf8);
    doc.set_encoding(TextEncoding::Utf8Bom);
    assert_eq!(doc.encode_preserving_bom().unwrap(), bom_bytes);

    // BOM 없는 UTF-16 LE → UTF-16 BE: 인코딩이 바뀌었으므로 BOM 포함
    let original = fs::read(fixture_path("utf16le_no_bom.txt")).unwrap();
    let mut doc = decode_text(&original, TextEncoding::Utf8);
    doc.set_encoding(TextEncoding::Utf16Be);
    assert!(
        doc.encode_preserving_bom()
            .unwrap()
            .starts_with(UTF16BE_BOM)
    );

    // 같은 인코딩으로 다시 지정하면 원본(BOM 없음)을 그대로 보존
    let mut doc = decode_text(&original, TextEncoding::Utf8);
    doc.set_encoding(TextEncoding::Utf16Le);
    assert_eq!(doc.encode_preserving_bom().unwrap(), original);
}

#[test]
fn bom_is_decided_by_encoding_except_utf16() {
    let text = "abc";

    // UTF-8 / EUC-KR: BOM 플래그와 무관하게 인코딩이 결정
    assert_eq!(
        encode_text_with_bom(text, TextEncoding::Utf8, true).unwrap(),
        b"abc"
    );
    assert_eq!(
        encode_text_with_bom(text, TextEncoding::EucKr, true).unwrap(),
        b"abc"
    );
    let mut with_bom = UTF8_BOM.to_vec();
    with_bom.extend_from_slice(b"abc");
    assert_eq!(
        encode_text_with_bom(text, TextEncoding::Utf8Bom, false).unwrap(),
        with_bom
    );

    // UTF-16만 BOM 생략 가능
    assert_eq!(
        encode_text_with_bom(text, TextEncoding::Utf16Le, false).unwrap(),
        [0x61, 0x00, 0x62, 0x00, 0x63, 0x00]
    );

    // BOM 있는 파일을 "UTF-8로 다시 읽기"하면 has_bom은 원본 사실(true)을 남기지만 저장은 UTF-8(BOM 없음)
    let doc = decode_with_encoding(&with_bom, TextEncoding::Utf8);
    assert!(doc.has_bom);
    assert_eq!(doc.encode_preserving_bom().unwrap(), b"abc");
}
