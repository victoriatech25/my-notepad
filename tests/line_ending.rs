//! P1-06 줄바꿈(`CRLF` / `LF` / `CR` / `Mixed`) 감지·정규화·복원 통합 테스트 (PLAN.md §3.4).
//!
//! 검증 항목 (WORKPLAN.md P1-06):
//! - `CRLF` / `LF` / `CR` 감지 (`Mixed` 포함, 다수 방식 채택)
//! - 내부 `LF` 정규화 (`normalize_to_lf`, `analyze_and_normalize`)
//! - 저장 시 대상 줄바꿈 방식으로 변환 (`apply_line_ending`)
//! - 파일 끝 개행 유무(有/無) 보존
//! - 저장 시 변환 여부(`converts_on_save`)와 단독 `CR` 다수·동수일 때 기본값 규칙

use std::fs;
use std::path::Path;

use my_notepad::core::line_ending::{
    LineEnding, LineEndingCounts, analyze_and_normalize_with_default, apply_line_ending,
    detect_line_ending_with_default, normalize_to_lf,
};

#[test]
fn pure_crlf_lf_and_cr_detection_normalization_and_restoration() {
    // 1. 저장소 픽스처 tests/fixtures/line_ending/crlf.txt 확인
    let crlf_fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("line_ending")
        .join("crlf.txt");
    let crlf_bytes = fs::read(&crlf_fixture_path).expect("crlf.txt 픽스처 읽기 실패");
    let crlf_str = std::str::from_utf8(&crlf_bytes).expect("crlf.txt는 유효한 UTF-8이어야 함");

    let norm_crlf = analyze_and_normalize_with_default(crlf_str, LineEnding::Lf);
    assert_eq!(norm_crlf.detection.line_ending, LineEnding::CrLf);
    assert!(!norm_crlf.detection.mixed);
    assert!(!norm_crlf.detection.has_legacy_cr);
    assert!(norm_crlf.detection.has_trailing_newline);
    assert_eq!(norm_crlf.detection.status_label(), "CRLF");
    assert_eq!(norm_crlf.text, "first line\nsecond line\n한글 줄\n");
    assert_eq!(
        apply_line_ending(&norm_crlf.text, norm_crlf.detection.line_ending),
        crlf_str
    );

    // 2. 순수 LF 문서
    let lf_str = "첫째 줄\n둘째 줄\n셋째 줄\n";
    let norm_lf = analyze_and_normalize_with_default(lf_str, LineEnding::CrLf);
    assert_eq!(norm_lf.detection.line_ending, LineEnding::Lf);
    assert!(!norm_lf.detection.mixed);
    assert!(norm_lf.detection.has_trailing_newline);
    assert_eq!(norm_lf.detection.status_label(), "LF");
    assert_eq!(norm_lf.text, lf_str);
    assert_eq!(apply_line_ending(&norm_lf.text, LineEnding::Lf), lf_str);
    assert_eq!(
        apply_line_ending(&norm_lf.text, LineEnding::CrLf),
        "첫째 줄\r\n둘째 줄\r\n셋째 줄\r\n"
    );

    // 3. 구형 Mac 단독 CR 문서 -> 내부 LF로 정규화되고 has_legacy_cr = true
    let cr_str = "mac line 1\rmac line 2\rmac line 3\r";
    let norm_cr = analyze_and_normalize_with_default(cr_str, LineEnding::CrLf);
    assert_eq!(
        norm_cr.detection.counts,
        LineEndingCounts {
            crlf: 0,
            lf: 0,
            cr: 3
        }
    );
    // 단독 CR은 저장 방식이 아니므로 CRLF·LF 동수(0:0) → 기본값(CRLF)
    assert_eq!(norm_cr.detection.line_ending, LineEnding::CrLf);
    assert!(!norm_cr.detection.mixed);
    assert!(norm_cr.detection.has_legacy_cr);
    assert!(norm_cr.detection.converts_on_save());
    assert!(norm_cr.detection.has_trailing_newline);
    assert_eq!(norm_cr.text, "mac line 1\nmac line 2\nmac line 3\n");
}

#[test]
fn mixed_line_endings_choose_majority_and_unify_on_save() {
    // 1. CRLF 다수 (CRLF 3개, LF 1개) -> Mixed (CRLF)
    let mixed_crlf_majority = "줄1\r\n줄2\n줄3\r\n줄4\r\n";
    let res1 = analyze_and_normalize_with_default(mixed_crlf_majority, LineEnding::Lf);
    assert_eq!(
        res1.detection.counts,
        LineEndingCounts {
            crlf: 3,
            lf: 1,
            cr: 0
        }
    );
    assert_eq!(res1.detection.line_ending, LineEnding::CrLf);
    assert!(res1.detection.mixed);
    assert_eq!(res1.detection.status_label(), "Mixed (CRLF)");
    assert!(res1.detection.has_trailing_newline);
    assert_eq!(res1.text, "줄1\n줄2\n줄3\n줄4\n");

    // 저장 시 채택된 다수 방식(CRLF)으로 통일됨
    let saved1 = apply_line_ending(&res1.text, res1.detection.line_ending);
    assert_eq!(saved1, "줄1\r\n줄2\r\n줄3\r\n줄4\r\n");

    // 2. LF 다수 (CRLF 1개, LF 3개, CR 1개) -> Mixed (LF)
    let mixed_lf_majority = "a\nb\r\nc\nd\re\n";
    let res2 = analyze_and_normalize_with_default(mixed_lf_majority, LineEnding::CrLf);
    assert_eq!(
        res2.detection.counts,
        LineEndingCounts {
            crlf: 1,
            lf: 3,
            cr: 1
        }
    );
    assert_eq!(res2.detection.line_ending, LineEnding::Lf);
    assert!(res2.detection.mixed);
    assert!(res2.detection.has_legacy_cr);
    assert_eq!(res2.detection.status_label(), "Mixed (LF)");
    assert!(res2.detection.has_trailing_newline);
    assert_eq!(res2.text, "a\nb\nc\nd\ne\n");

    // 저장 시 채택된 다수 방식(LF)으로 통일됨
    let saved2 = apply_line_ending(&res2.text, res2.detection.line_ending);
    assert_eq!(saved2, "a\nb\nc\nd\ne\n");

    // 3. CRLF == LF 동수인 경우 default_line_ending을 따름
    let tie = "줄1\r\n줄2\n";
    let tie_win = detect_line_ending_with_default(tie, LineEnding::CrLf);
    assert_eq!(tie_win.line_ending, LineEnding::CrLf);
    assert!(tie_win.mixed);

    let tie_mac = detect_line_ending_with_default(tie, LineEnding::Lf);
    assert_eq!(tie_mac.line_ending, LineEnding::Lf);
    assert!(tie_mac.mixed);
}

#[test]
fn trailing_newline_presence_and_absence_are_preserved() {
    // 1. 끝 개행이 있는 문서 (CRLF / LF / Mixed)
    for (raw, expected_norm) in [
        ("가\r\n나\r\n", "가\n나\n"),
        ("가\n나\n", "가\n나\n"),
        ("가\r\n나\n", "가\n나\n"),
    ] {
        let norm = analyze_and_normalize_with_default(raw, LineEnding::CrLf);
        assert!(
            norm.detection.has_trailing_newline,
            "끝 개행 있음이 감지되어야 함: {raw:?}"
        );
        assert_eq!(norm.text, expected_norm);
        assert_eq!(apply_line_ending(&norm.text, LineEnding::Lf), "가\n나\n");
        assert_eq!(
            apply_line_ending(&norm.text, LineEnding::CrLf),
            "가\r\n나\r\n"
        );
    }

    // 2. 끝 개행이 없는 문서 (CRLF / LF / Mixed)
    for (raw, expected_norm) in [
        ("가\r\n나", "가\n나"),
        ("가\n나", "가\n나"),
        ("가\r\n나\n다", "가\n나\n다"),
    ] {
        let norm = analyze_and_normalize_with_default(raw, LineEnding::CrLf);
        assert!(
            !norm.detection.has_trailing_newline,
            "끝 개행 없음이 감지되어야 함: {raw:?}"
        );
        assert_eq!(norm.text, expected_norm);
        assert!(
            !norm.text.ends_with('\n'),
            "정규화 결과 끝에 개행이 추가되면 안 됨"
        );

        let as_lf = apply_line_ending(&norm.text, LineEnding::Lf);
        assert!(!as_lf.ends_with('\n'));
        assert_eq!(as_lf, expected_norm);

        let as_crlf = apply_line_ending(&norm.text, LineEnding::CrLf);
        assert!(!as_crlf.ends_with('\n') && !as_crlf.ends_with('\r'));
        assert_eq!(as_crlf, expected_norm.replace('\n', "\r\n"));
    }

    // 3. 개행이 전혀 없는 한 줄 문서 및 빈 문서
    let single_line = "개행 없는 한 줄 문서";
    let norm_single = analyze_and_normalize_with_default(single_line, LineEnding::CrLf);
    assert_eq!(norm_single.detection.line_ending, LineEnding::CrLf);
    assert!(!norm_single.detection.mixed);
    assert!(!norm_single.detection.has_trailing_newline);
    assert_eq!(norm_single.text, single_line);
    assert_eq!(
        apply_line_ending(&norm_single.text, LineEnding::CrLf),
        single_line
    );
    assert_eq!(normalize_to_lf(""), "");
}

#[test]
fn converts_on_save_flags_mixed_and_legacy_cr_only() {
    for (raw, expected) in [
        ("a\r\nb\r\n", false),
        ("a\nb\n", false),
        ("개행 없는 한 줄", false),
        ("", false),
        ("a\r\nb\n", true),
        ("a\rb\rc\r", true),
        ("a\r\nb\r", true),
    ] {
        let detection = detect_line_ending_with_default(raw, LineEnding::CrLf);
        assert_eq!(detection.converts_on_save(), expected, "{raw:?}");

        // 변환이 없다고 판정되면 채택 방식으로 저장한 결과가 원본과 바이트 동일해야 한다.
        if !expected {
            let norm = analyze_and_normalize_with_default(raw, LineEnding::CrLf);
            assert_eq!(apply_line_ending(&norm.text, detection.line_ending), raw);
        }
    }
}

#[test]
fn legacy_cr_majority_and_ties_follow_crlf_lf_comparison_then_default() {
    // CR 전용: CRLF·LF 동수(0:0) → 기본값
    for default in LineEnding::ALL {
        let d = detect_line_ending_with_default("a\rb\rc\r", default);
        assert_eq!(d.line_ending, default);
        assert!(d.has_legacy_cr);
        assert!(!d.mixed);
    }

    // CR 다수 + CRLF 1·LF 1 동수 → 기본값
    for default in LineEnding::ALL {
        let d = detect_line_ending_with_default("a\rb\rc\r\nd\ne", default);
        assert_eq!(d.line_ending, default);
        assert!(d.mixed);
    }

    // CR 다수 + CRLF 2·LF 1 → CRLF (기본값 LF여도)
    let d = detect_line_ending_with_default("a\rb\rc\rd\r\ne\r\nf\ng", LineEnding::Lf);
    assert_eq!(d.line_ending, LineEnding::CrLf);

    // CR 다수 + CRLF 1·LF 2 → LF (기본값 CRLF여도)
    let d = detect_line_ending_with_default("a\rb\rc\rd\r\ne\nf\ng", LineEnding::CrLf);
    assert_eq!(d.line_ending, LineEnding::Lf);

    // 개행 없음 → 기본값
    for default in LineEnding::ALL {
        assert_eq!(
            detect_line_ending_with_default("한 줄", default).line_ending,
            default
        );
    }
}
