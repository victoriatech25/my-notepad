//! 파일·폴더 아이콘 (PLAN.md §3.1-3) — P0-UX2에서 A안(Phosphor 단색 아이콘 폰트 + 종류별 색) 채택.
//!
//! 결정 순서: 파일 이름 정확히 일치 → 확장자 → 기본. 폴더는 이름별 색 + 열림/닫힘 글리프.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText};

pub const ICON_FAMILY: &str = "file-icons";

/// 아이콘 폰트를 별도 패밀리로 등록한다.
pub fn add_icon_font(defs: &mut FontDefinitions) {
    defs.font_data.insert(
        "phosphor".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../../assets/file-icons/phosphor/Phosphor.ttf"
        ))),
    );
    defs.families.insert(
        FontFamily::Name(ICON_FAMILY.into()),
        vec!["phosphor".to_owned()],
    );
}

// Phosphor 코드포인트 (egui-phosphor 0.14.0 codepoints.rs 기준)
const PH_FILE: &str = "\u{E230}";
const PH_FILE_CODE: &str = "\u{E914}";
const PH_FILE_LOCK: &str = "\u{E95C}";
const PH_FILE_MD: &str = "\u{ED50}";
const PH_FILE_RS: &str = "\u{EB28}";
const PH_FILE_TXT: &str = "\u{EB35}";
const PH_FOLDER: &str = "\u{E24A}";
const PH_FOLDER_OPEN: &str = "\u{E256}";
const PH_GEAR_SIX: &str = "\u{E272}";
const PH_GIT_BRANCH: &str = "\u{E278}";
const PH_INFO: &str = "\u{E2CE}";
const PH_APP_WINDOW: &str = "\u{E5DA}";
const PH_BRACKETS_CURLY: &str = "\u{E860}";
const PH_BUG: &str = "\u{E5F4}";

const RUST: Color32 = Color32::from_rgb(0xDE, 0x6A, 0x3B);
const BLUE: Color32 = Color32::from_rgb(0x42, 0xA5, 0xF5);
const GRAY: Color32 = Color32::from_rgb(0x9E, 0x9E, 0x9E);
const YELLOW: Color32 = Color32::from_rgb(0xF4, 0xC4, 0x30);
const GIT: Color32 = Color32::from_rgb(0xE6, 0x4A, 0x19);
const GREEN: Color32 = Color32::from_rgb(0x66, 0xBB, 0x6A);
const TEAL: Color32 = Color32::from_rgb(0x26, 0xA6, 0x9A);
const RED: Color32 = Color32::from_rgb(0xEF, 0x53, 0x50);
const PURPLE: Color32 = Color32::from_rgb(0xAB, 0x47, 0xBC);
const FOLDER: Color32 = Color32::from_rgb(0x90, 0xA4, 0xAE);

fn file_icon(name: &str) -> (&'static str, Color32) {
    let lower = name.to_ascii_lowercase();
    // ① 파일 이름 정확히 일치
    match lower.as_str() {
        "cargo.toml" => return (PH_GEAR_SIX, RUST),
        "cargo.lock" => return (PH_FILE_LOCK, YELLOW),
        ".gitignore" | ".gitattributes" => return (PH_GIT_BRANCH, GIT),
        ".editorconfig" => return (PH_GEAR_SIX, GRAY),
        "readme.md" => return (PH_INFO, BLUE),
        "rust-toolchain.toml" => return (PH_FILE_RS, RUST),
        _ => {}
    }
    // ② 확장자
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "rs" => (PH_FILE_RS, RUST),
        "md" | "markdown" => (PH_FILE_MD, BLUE),
        "toml" => (PH_GEAR_SIX, GRAY),
        "json" => (PH_BRACKETS_CURLY, YELLOW),
        "txt" => (PH_FILE_TXT, GRAY),
        "log" => (PH_FILE_TXT, GREEN),
        "exe" => (PH_APP_WINDOW, PURPLE),
        "pdb" => (PH_BUG, GRAY),
        "png" | "jpg" | "svg" => (PH_FILE_CODE, GREEN),
        // ③ 기본
        _ => (PH_FILE, GRAY),
    }
}

fn folder_color(name: &str) -> Color32 {
    match name.to_ascii_lowercase().as_str() {
        "src" => GREEN,
        "docs" | "doc" => BLUE,
        "tests" | "test" | "fixtures" => TEAL,
        "target" => RED,
        ".cargo" => RUST,
        ".git" => GIT,
        "screens" | "images" => GREEN,
        "xtask" | "scripts" | "tools" => YELLOW,
        _ => FOLDER,
    }
}

/// 아이콘을 그린다. `dimmed`면 .gitignore 대상처럼 흐리게.
pub fn show(ui: &mut egui::Ui, name: &str, is_dir: bool, open: bool, dimmed: bool) {
    let (glyph, color) = if is_dir {
        (
            if open { PH_FOLDER_OPEN } else { PH_FOLDER },
            folder_color(name),
        )
    } else {
        file_icon(name)
    };
    let color = if dimmed {
        color.gamma_multiply(0.45)
    } else {
        color
    };
    let size = ui.text_style_height(&egui::TextStyle::Body).max(16.0);
    ui.label(
        RichText::new(glyph)
            .family(FontFamily::Name(ICON_FAMILY.into()))
            .size(size)
            .color(color),
    );
}
