//! 파일·폴더 아이콘 결정 규칙 (PLAN.md §3.1-3, P0-UX2 A안: Phosphor 단색 아이콘 폰트 + 종류별 색).
//!
//! 규칙은 매핑 표 `assets/file-icons/icons.toml`에 있고 실행 파일에 내장된다 (처음 쓸 때 한 번 읽음).
//! - 파일: 이름 정확히 일치 → 확장자 → 기본. 대소문자 무시
//! - 폴더: 열림/닫힘 글리프를 나누고, 색은 폴더 이름별 (없으면 기본 폴더 색)
//!
//! 탭 바(P1-10)와 사이드바(P2-06)가 같은 규칙을 쓴다.

use std::collections::HashMap;
use std::sync::OnceLock;

/// 아이콘 글리프(Phosphor 코드포인트)와 색(RGB). UI 계층이 `egui` 색으로 바꿔 그린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIcon {
    pub glyph: &'static str,
    pub rgb: [u8; 3],
}

/// 내장 매핑 표.
const ICONS_TOML: &str = include_str!("../../assets/file-icons/icons.toml");

/// 파일 이름(경로 없이)으로 아이콘을 고른다. 대소문자는 구분하지 않는다.
pub fn file_icon(file_name: &str) -> FileIcon {
    let table = table();
    let lower = file_name.to_lowercase();
    // ① 파일 이름 정확히 일치
    if let Some(icon) = table.names.get(&lower) {
        return *icon;
    }
    // ② 확장자 (`.gitignore`처럼 점으로 시작하는 이름 전체는 확장자로 보지 않는다)
    let ext = lower
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or("", |(_, e)| e);
    // ③ 기본
    table.extensions.get(ext).copied().unwrap_or(table.default)
}

/// 폴더 이름과 열림 여부로 아이콘을 고른다. 대소문자는 구분하지 않는다.
pub fn folder_icon(folder_name: &str, open: bool) -> FileIcon {
    let table = table();
    FileIcon {
        glyph: if open {
            table.folder_open
        } else {
            table.folder_closed
        },
        rgb: table
            .folder_colors
            .get(&folder_name.to_lowercase())
            .copied()
            .unwrap_or(table.folder_color),
    }
}

#[derive(Debug)]
struct IconTable {
    /// 글리프 이름 → 글리프 (표에 정의된 전부 — 폰트 누락 검사용)
    #[cfg_attr(not(test), allow(dead_code))]
    glyphs: HashMap<String, &'static str>,
    names: HashMap<String, FileIcon>,
    extensions: HashMap<String, FileIcon>,
    default: FileIcon,
    folder_closed: &'static str,
    folder_open: &'static str,
    folder_color: [u8; 3],
    folder_colors: HashMap<String, [u8; 3]>,
}

fn table() -> &'static IconTable {
    static TABLE: OnceLock<IconTable> = OnceLock::new();
    TABLE.get_or_init(|| parse(ICONS_TOML).expect("내장 icons.toml 오류 (테스트로 검사됨)"))
}

/// 매핑 표를 읽는다. 정의되지 않은 글리프·색 이름을 쓰면 오류.
fn parse(source: &str) -> Result<IconTable, String> {
    let root: toml::Table = source.parse().map_err(|e| format!("TOML 문법: {e}"))?;
    let section = |value: &toml::Value, path: &str| -> Result<toml::Table, String> {
        value
            .as_table()
            .cloned()
            .ok_or_else(|| format!("[{path}]가 표가 아닙니다"))
    };
    let get = |table: &toml::Table, key: &str, path: &str| -> Result<toml::Value, String> {
        table
            .get(key)
            .cloned()
            .ok_or_else(|| format!("[{path}]에 `{key}`가 없습니다"))
    };
    let string = |value: &toml::Value, what: &str| -> Result<String, String> {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("{what}: 문자열이어야 합니다"))
    };

    let mut glyphs = HashMap::new();
    for (name, value) in section(&get(&root, "glyphs", "")?, "glyphs")? {
        let hex = string(&value, &format!("glyphs.{name}"))?;
        let c = u32::from_str_radix(&hex, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| format!("glyphs.{name}: 코드포인트 `{hex}`가 잘못되었습니다"))?;
        // 프로그램이 끝날 때까지 쓰는 짧은 문자열 십여 개라 한 번 고정해 둔다.
        let glyph: &'static str = Box::leak(c.to_string().into_boxed_str());
        glyphs.insert(name, glyph);
    }
    let mut colors = HashMap::new();
    for (name, value) in section(&get(&root, "colors", "")?, "colors")? {
        let hex = string(&value, &format!("colors.{name}"))?;
        let rgb =
            parse_rgb(&hex).ok_or_else(|| format!("colors.{name}: 색 `{hex}`가 잘못되었습니다"))?;
        colors.insert(name, rgb);
    }

    let glyph = |name: &str, what: &str| -> Result<&'static str, String> {
        glyphs
            .get(name)
            .copied()
            .ok_or_else(|| format!("{what}: 정의되지 않은 글리프 `{name}`"))
    };
    let color = |name: &str, what: &str| -> Result<[u8; 3], String> {
        colors
            .get(name)
            .copied()
            .ok_or_else(|| format!("{what}: 정의되지 않은 색 `{name}`"))
    };
    let file_entry = |value: &toml::Value, what: &str| -> Result<FileIcon, String> {
        let entry = section(value, what)?;
        Ok(FileIcon {
            glyph: glyph(&string(&get(&entry, "glyph", what)?, what)?, what)?,
            rgb: color(&string(&get(&entry, "color", what)?, what)?, what)?,
        })
    };
    let entries = |table: &toml::Table, path: &str| -> Result<HashMap<String, FileIcon>, String> {
        table
            .iter()
            .map(|(key, value)| {
                Ok((
                    key.to_lowercase(),
                    file_entry(value, &format!("{path}.{key}"))?,
                ))
            })
            .collect()
    };

    let files = section(&get(&root, "files", "")?, "files")?;
    let folders = section(&get(&root, "folders", "")?, "folders")?;
    let folder_names = section(&get(&folders, "names", "folders")?, "folders.names")?;
    let mut folder_colors = HashMap::new();
    for (name, value) in &folder_names {
        let what = format!("folders.names.{name}");
        folder_colors.insert(name.to_lowercase(), color(&string(value, &what)?, &what)?);
    }

    Ok(IconTable {
        names: entries(
            &section(&get(&files, "names", "files")?, "files.names")?,
            "files.names",
        )?,
        extensions: entries(
            &section(&get(&files, "extensions", "files")?, "files.extensions")?,
            "files.extensions",
        )?,
        default: file_entry(&get(&files, "default", "files")?, "files.default")?,
        folder_closed: glyph(
            &string(&get(&folders, "closed", "folders")?, "folders.closed")?,
            "folders.closed",
        )?,
        folder_open: glyph(
            &string(&get(&folders, "open", "folders")?, "folders.open")?,
            "folders.open",
        )?,
        folder_color: color(
            &string(&get(&folders, "color", "folders")?, "folders.color")?,
            "folders.color",
        )?,
        folder_colors,
        glyphs,
    })
}

/// `#RRGGBB` → RGB
fn parse_rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(name: &str) -> &'static str {
        table().glyphs[name]
    }

    fn color(name: &str) -> [u8; 3] {
        let source: toml::Table = ICONS_TOML.parse().unwrap();
        parse_rgb(source["colors"][name].as_str().unwrap()).unwrap()
    }

    #[test]
    fn name_match_wins_over_extension() {
        assert_eq!(file_icon("README.md").glyph, glyph("info"));
        assert_eq!(file_icon("notes.md").glyph, glyph("file-md"));
        // `Cargo.toml` → Cargo 아이콘(러스트 색), `x.toml` → TOML 아이콘(회색)
        assert_eq!(file_icon("Cargo.toml").glyph, glyph("gear-six"));
        assert_eq!(file_icon("Cargo.toml").rgb, color("rust"));
        assert_eq!(file_icon("x.toml").rgb, color("gray"));
        assert_ne!(file_icon("Cargo.toml"), file_icon("x.toml"));
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(file_icon("CARGO.TOML"), file_icon("Cargo.toml"));
        assert_eq!(file_icon("Main.RS"), file_icon("main.rs"));
        assert_eq!(file_icon("LOG.TXT").glyph, glyph("file-txt"));
        assert_eq!(folder_icon("SRC", false), folder_icon("src", false));
    }

    #[test]
    fn unknown_and_untitled_use_default() {
        let default = file_icon("Untitled-1");
        assert_eq!(default.glyph, glyph("file"));
        assert_eq!(file_icon("archive.zip"), default);
        assert_eq!(file_icon("Makefile"), default);
        assert_eq!(file_icon("trailing."), default);
        // 점으로 시작하는 이름은 이름 전체가 확장자가 아니다 (`.md`라는 파일은 기본 아이콘)
        assert_eq!(file_icon(".md"), default);
        assert_eq!(file_icon(".gitignore").glyph, glyph("git-branch"));
    }

    #[test]
    fn folders_have_open_closed_glyphs_and_name_colors() {
        assert_eq!(folder_icon("src", false).glyph, glyph("folder"));
        assert_eq!(folder_icon("src", true).glyph, glyph("folder-open"));
        assert_eq!(folder_icon("src", true).rgb, color("green"));
        assert_eq!(folder_icon("target", false).rgb, color("red"));
        assert_eq!(folder_icon("anything", false).rgb, color("folder"));
    }

    /// 표에 정의된 글리프가 모두 내장 Phosphor 폰트에 실제로 있다 (A안이라 아이콘 파일 대신 폰트 글리프 누락 검사).
    #[test]
    fn every_glyph_exists_in_phosphor_font() {
        let font = include_bytes!("../../assets/file-icons/phosphor/Phosphor.ttf");
        let face = ttf_parser::Face::parse(font, 0).unwrap();
        let missing: Vec<_> = table()
            .glyphs
            .iter()
            .filter(|(_, g)| {
                let c = g.chars().next().unwrap();
                face.glyph_index(c).is_none()
            })
            .map(|(name, g)| format!("{name} (U+{:04X})", g.chars().next().unwrap() as u32))
            .collect();
        assert!(missing.is_empty(), "폰트에 없는 글리프: {missing:?}");
        assert!(table().glyphs.len() >= 14);
    }

    #[test]
    fn invalid_tables_are_rejected() {
        let minimal = r##"
            [glyphs]
            file = "E230"
            [colors]
            gray = "#9E9E9E"
            [files]
            default = { glyph = "file", color = "gray" }
            [files.names]
            [files.extensions]
            [folders]
            closed = "file"
            open = "file"
            color = "gray"
            [folders.names]
        "##;
        assert!(parse(minimal).is_ok());
        let unknown_glyph = minimal.replace(
            r#"default = { glyph = "file""#,
            r#"default = { glyph = "nope""#,
        );
        assert!(parse(&unknown_glyph).unwrap_err().contains("nope"));
        let unknown_color = minimal.replace(r#"color = "gray" }"#, r#"color = "pink" }"#);
        assert!(parse(&unknown_color).unwrap_err().contains("pink"));
        let bad_codepoint = minimal.replace(r#"file = "E230""#, r#"file = "ZZZZ""#);
        assert!(parse(&bad_codepoint).is_err());
        assert_eq!(parse_rgb("#DE6A3B"), Some([0xDE, 0x6A, 0x3B]));
        assert_eq!(parse_rgb("DE6A3B"), None);
        assert_eq!(parse_rgb("#DE6A"), None);
    }
}
