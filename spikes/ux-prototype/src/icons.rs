//! P0-UX2 파일·폴더 아이콘 A/B 비교 (PLAN.md §3.1-3).
//!
//! 결정 순서: 파일 이름 정확히 일치 → 확장자 → 기본. 폴더는 이름별 + 열림/닫힘.
//! - A: Phosphor 아이콘 폰트(단색) + 종류별 색
//! - B: Material Icon Theme 다색 SVG

use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText, Vec2};

pub const ICON_FAMILY: &str = "file-icons";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconStyle {
    None,
    /// A. 단색 아이콘 폰트
    Font,
    /// B. 다색 SVG
    Svg,
}

impl IconStyle {
    pub fn label(self) -> &'static str {
        match self {
            IconStyle::None => "아이콘 없음",
            IconStyle::Font => "A. 단색 아이콘 폰트 (Phosphor)",
            IconStyle::Svg => "B. 다색 SVG (Material Icon Theme)",
        }
    }
}

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

/// 한 항목의 아이콘: A용 글리프·색, B용 SVG 이름
struct Icon {
    glyph: &'static str,
    color: Color32,
    svg: &'static str,
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
const FOLDER: Color32 = Color32::from_rgb(0x90, 0xA4, 0xAE);
const FOLDER_SRC: Color32 = Color32::from_rgb(0x4C, 0xAF, 0x50);
const FOLDER_DOCS: Color32 = Color32::from_rgb(0x42, 0xA5, 0xF5);
const FOLDER_TEST: Color32 = Color32::from_rgb(0x26, 0xA6, 0x9A);
const FOLDER_TARGET: Color32 = Color32::from_rgb(0xEF, 0x53, 0x50);
const FOLDER_RUST: Color32 = Color32::from_rgb(0xDE, 0x6A, 0x3B);
const PURPLE: Color32 = Color32::from_rgb(0xAB, 0x47, 0xBC);

const fn icon(glyph: &'static str, color: Color32, svg: &'static str) -> Icon {
    Icon { glyph, color, svg }
}

fn file_icon(name: &str) -> Icon {
    let lower = name.to_ascii_lowercase();
    // ① 파일 이름 정확히 일치
    match lower.as_str() {
        "cargo.toml" => return icon(PH_GEAR_SIX, RUST, "settings"),
        "cargo.lock" => return icon(PH_FILE_LOCK, YELLOW, "lock"),
        ".gitignore" | ".gitattributes" => return icon(PH_GIT_BRANCH, GIT, "git"),
        ".editorconfig" => return icon(PH_GEAR_SIX, GRAY, "editorconfig"),
        "readme.md" => return icon(PH_INFO, BLUE, "readme"),
        "rust-toolchain.toml" => return icon(PH_FILE_RS, RUST, "rust"),
        _ => {}
    }
    // ② 확장자
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "rs" => icon(PH_FILE_RS, RUST, "rust"),
        "md" | "markdown" => icon(PH_FILE_MD, BLUE, "markdown"),
        "toml" => icon(PH_GEAR_SIX, GRAY, "toml"),
        "json" => icon(PH_BRACKETS_CURLY, YELLOW, "json"),
        "txt" => icon(PH_FILE_TXT, GRAY, "document"),
        "log" => icon(PH_FILE_TXT, GREEN, "log"),
        "exe" => icon(PH_APP_WINDOW, PURPLE, "exe"),
        "pdb" => icon(PH_BUG, GRAY, "database"),
        "png" | "jpg" | "svg" => icon(PH_FILE_CODE, GREEN, "image"),
        // ③ 기본
        _ => icon(PH_FILE, GRAY, "file"),
    }
}

fn folder_icon(name: &str, open: bool) -> Icon {
    let (color, base) = match name.to_ascii_lowercase().as_str() {
        "src" => (FOLDER_SRC, "folder-src"),
        "docs" | "doc" => (FOLDER_DOCS, "folder-docs"),
        "tests" | "test" | "fixtures" => (FOLDER_TEST, "folder-test"),
        "target" => (FOLDER_TARGET, "folder-target"),
        ".cargo" => (FOLDER_RUST, "folder-rust"),
        ".git" => (GIT, "folder-git"),
        "screens" | "images" => (GREEN, "folder-images"),
        "xtask" | "scripts" | "tools" => (YELLOW, "folder-tools"),
        "config" | ".github" => (GRAY, "folder-config"),
        _ => (FOLDER, "folder"),
    };
    let svg: &'static str = match (base, open) {
        ("folder", false) => "folder",
        ("folder", true) => "folder-open",
        ("folder-src", false) => "folder-src",
        ("folder-src", true) => "folder-src-open",
        ("folder-docs", false) => "folder-docs",
        ("folder-docs", true) => "folder-docs-open",
        ("folder-test", false) => "folder-test",
        ("folder-test", true) => "folder-test-open",
        ("folder-target", false) => "folder-target",
        ("folder-target", true) => "folder-target-open",
        ("folder-rust", false) => "folder-rust",
        ("folder-rust", true) => "folder-rust-open",
        ("folder-git", false) => "folder-git",
        ("folder-git", true) => "folder-git-open",
        ("folder-images", false) => "folder-images",
        ("folder-images", true) => "folder-images-open",
        ("folder-tools", false) => "folder-tools",
        ("folder-tools", true) => "folder-tools-open",
        ("folder-config", false) => "folder-config",
        ("folder-config", true) => "folder-config-open",
        _ => "folder",
    };
    icon(if open { PH_FOLDER_OPEN } else { PH_FOLDER }, color, svg)
}

macro_rules! material {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../../assets/file-icons/material/", $name, ".svg")) as &[u8])),*]
    };
}

const MATERIAL: &[(&str, &[u8])] = material!(
    "rust",
    "markdown",
    "toml",
    "json",
    "document",
    "log",
    "exe",
    "git",
    "lock",
    "readme",
    "editorconfig",
    "settings",
    "file",
    "image",
    "database",
    "folder",
    "folder-open",
    "folder-src",
    "folder-src-open",
    "folder-docs",
    "folder-docs-open",
    "folder-test",
    "folder-test-open",
    "folder-target",
    "folder-target-open",
    "folder-rust",
    "folder-rust-open",
    "folder-git",
    "folder-git-open",
    "folder-images",
    "folder-images-open",
    "folder-tools",
    "folder-tools-open",
    "folder-config",
    "folder-config-open",
);

/// 아이콘을 그린다. `dimmed`면 .gitignore 대상처럼 흐리게.
pub fn show(
    ui: &mut egui::Ui,
    style: IconStyle,
    name: &str,
    is_dir: bool,
    open: bool,
    dimmed: bool,
) {
    if style == IconStyle::None {
        return;
    }
    let icon = if is_dir {
        folder_icon(name, open)
    } else {
        file_icon(name)
    };
    let size = ui.text_style_height(&egui::TextStyle::Body).max(16.0);
    match style {
        IconStyle::Font => {
            let color = if dimmed {
                icon.color.gamma_multiply(0.45)
            } else {
                icon.color
            };
            ui.label(
                RichText::new(icon.glyph)
                    .family(FontFamily::Name(ICON_FAMILY.into()))
                    .size(size)
                    .color(color),
            );
        }
        IconStyle::Svg => {
            let bytes = MATERIAL
                .iter()
                .find(|(n, _)| *n == icon.svg)
                .map_or(MATERIAL[12].1, |(_, b)| *b);
            let image =
                egui::Image::from_bytes(format!("bytes://file-icons/{}.svg", icon.svg), bytes)
                    .fit_to_exact_size(Vec2::splat(size))
                    .tint(if dimmed {
                        Color32::from_white_alpha(115)
                    } else {
                        Color32::WHITE
                    });
            ui.add(image);
        }
        IconStyle::None => {}
    }
}
