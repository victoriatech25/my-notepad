//! 파일·폴더 아이콘 결정 규칙 (PLAN.md §3.1-3, P0-UX2 A안: Phosphor 단색 아이콘 폰트 + 종류별 색).
//!
//! 결정 순서: 파일 이름 정확히 일치 → 확장자 → 기본.
//! - P1-10: 탭 바에서 쓰는 파일 아이콘 규칙
//! - P2-05: 사이드바용 폴더 아이콘(이름별 색, 열림/닫힘)과 흐린 표시 추가 예정

/// 아이콘 글리프(Phosphor 코드포인트)와 색(RGB). UI 계층이 `egui` 색으로 바꿔 그린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIcon {
    pub glyph: &'static str,
    pub rgb: [u8; 3],
}

// Phosphor 코드포인트 (egui-phosphor 0.14.0 codepoints.rs 기준)
const PH_FILE: &str = "\u{E230}";
const PH_FILE_CODE: &str = "\u{E914}";
const PH_FILE_LOCK: &str = "\u{E95C}";
const PH_FILE_MD: &str = "\u{ED50}";
const PH_FILE_RS: &str = "\u{EB28}";
const PH_FILE_TXT: &str = "\u{EB35}";
const PH_GEAR_SIX: &str = "\u{E272}";
const PH_GIT_BRANCH: &str = "\u{E278}";
const PH_INFO: &str = "\u{E2CE}";
const PH_APP_WINDOW: &str = "\u{E5DA}";
const PH_BRACKETS_CURLY: &str = "\u{E860}";
const PH_BUG: &str = "\u{E5F4}";

const RUST: [u8; 3] = [0xDE, 0x6A, 0x3B];
const BLUE: [u8; 3] = [0x42, 0xA5, 0xF5];
const GRAY: [u8; 3] = [0x9E, 0x9E, 0x9E];
const YELLOW: [u8; 3] = [0xF4, 0xC4, 0x30];
const GIT: [u8; 3] = [0xE6, 0x4A, 0x19];
const GREEN: [u8; 3] = [0x66, 0xBB, 0x6A];
const PURPLE: [u8; 3] = [0xAB, 0x47, 0xBC];

const fn icon(glyph: &'static str, rgb: [u8; 3]) -> FileIcon {
    FileIcon { glyph, rgb }
}

/// 파일 이름(경로 없이)으로 아이콘을 고른다. 대소문자는 구분하지 않는다.
pub fn file_icon(file_name: &str) -> FileIcon {
    let lower = file_name.to_ascii_lowercase();
    // ① 파일 이름 정확히 일치
    match lower.as_str() {
        "cargo.toml" => return icon(PH_GEAR_SIX, RUST),
        "cargo.lock" => return icon(PH_FILE_LOCK, YELLOW),
        ".gitignore" | ".gitattributes" => return icon(PH_GIT_BRANCH, GIT),
        ".editorconfig" => return icon(PH_GEAR_SIX, GRAY),
        "readme.md" => return icon(PH_INFO, BLUE),
        "rust-toolchain.toml" => return icon(PH_FILE_RS, RUST),
        _ => {}
    }
    // ② 확장자
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "rs" => icon(PH_FILE_RS, RUST),
        "md" | "markdown" => icon(PH_FILE_MD, BLUE),
        "toml" => icon(PH_GEAR_SIX, GRAY),
        "json" => icon(PH_BRACKETS_CURLY, YELLOW),
        "txt" => icon(PH_FILE_TXT, GRAY),
        "log" => icon(PH_FILE_TXT, GREEN),
        "exe" => icon(PH_APP_WINDOW, PURPLE),
        "pdb" => icon(PH_BUG, GRAY),
        "png" | "jpg" | "jpeg" | "gif" | "svg" => icon(PH_FILE_CODE, GREEN),
        // ③ 기본
        _ => icon(PH_FILE, GRAY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_match_wins_over_extension() {
        assert_eq!(file_icon("README.md").glyph, PH_INFO);
        assert_eq!(file_icon("notes.md").glyph, PH_FILE_MD);
        assert_eq!(file_icon("Cargo.toml").glyph, PH_GEAR_SIX);
        assert_eq!(file_icon("Cargo.toml").rgb, RUST);
        assert_eq!(file_icon("other.toml").rgb, GRAY);
    }

    #[test]
    fn unknown_and_untitled_use_default() {
        assert_eq!(file_icon("Untitled-1").glyph, PH_FILE);
        assert_eq!(file_icon("archive.zip").glyph, PH_FILE);
        assert_eq!(file_icon("LOG.TXT").glyph, PH_FILE_TXT);
    }
}
