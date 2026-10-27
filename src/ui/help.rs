//! 도움말 창 내용 (P5-UX): 단축키 목록·정규식 문법·버전 정보.
//!
//! - 단축키 목록은 [`Keymap`]에서 만들므로 메뉴 표기·실제 단축키와 늘 같다. 찾기 바 안에서만 쓰는 키
//!   (`Enter`·`Esc` 등, `search_bar.rs`가 직접 처리)는 [`SEARCH_BAR_KEYS`]에 따로 적는다.
//! - 정규식 문법은 찾기 엔진(`core::search`, Rust `regex`)이 실제로 지원하는 것만 적는다.
//! - 그리는 일은 `dialogs.rs`가 맡는다 (공통 대화상자 모양).

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

use crate::commands::{Command, Keymap};

/// 단축키 목록의 한 묶음 (메뉴 이름).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSection {
    pub title: &'static str,
    pub rows: Vec<ShortcutRow>,
}

/// 단축키 목록의 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutRow {
    pub action: String,
    /// 표시용 단축키 (여러 개면 ` · `로 잇는다)
    pub keys: String,
}

impl ShortcutRow {
    /// 검색어가 기능 이름이나 단축키 표기에 들어 있는지 (대소문자 무시).
    pub fn matches(&self, filter: &str) -> bool {
        let filter = filter.trim().to_lowercase();
        filter.is_empty()
            || self.action.to_lowercase().contains(&filter)
            || self.keys.to_lowercase().contains(&filter)
    }
}

/// 단축키 목록에 싣는 명령 (메뉴 순서). `GotoTab`은 1~8번을 한 줄로 묶는다.
const SECTIONS: &[(&str, &[Command])] = &[
    (
        "파일",
        &[
            Command::NewTab,
            Command::OpenFile,
            Command::Save,
            Command::SaveAs,
            Command::CloseTab,
            Command::CloseAllTabs,
            Command::Exit,
        ],
    ),
    (
        "편집",
        &[
            Command::Undo,
            Command::Redo,
            Command::Cut,
            Command::Copy,
            Command::Paste,
            Command::SelectAll,
            Command::GoToLine,
        ],
    ),
    (
        "검색",
        &[
            Command::Find,
            Command::Replace,
            Command::FindNext,
            Command::FindPrev,
        ],
    ),
    (
        "보기",
        &[
            Command::ToggleSidebar,
            Command::ZoomIn,
            Command::ZoomOut,
            Command::ZoomReset,
            Command::ToggleMdPreview,
            Command::ToggleMdSplit,
        ],
    ),
    (
        "창",
        &[
            Command::NextTab,
            Command::PrevTab,
            Command::GotoTab(0),
            Command::LastTab,
            Command::MoveTabLeft,
            Command::MoveTabRight,
        ],
    ),
];

/// 찾기 바 안에서 쓰는 키 (`search_bar.rs`가 직접 처리 — 전역 단축키 표에 없다).
const SEARCH_BAR_KEYS: &[(&str, Modifiers, Key)] = &[
    ("다음 매치 (찾을 내용 칸)", Modifiers::NONE, Key::Enter),
    ("이전 매치 (찾을 내용 칸)", Modifiers::SHIFT, Key::Enter),
    (
        "바꾸고 다음으로 (바꿀 내용 칸)",
        Modifiers::NONE,
        Key::Enter,
    ),
    (
        "모두 바꾸기 (바꿀 내용 칸)",
        Modifiers::COMMAND.plus(Modifiers::ALT),
        Key::Enter,
    ),
    ("찾기 바 닫기", Modifiers::NONE, Key::Escape),
];

/// 현재 OS 키맵으로 단축키 목록을 만든다 (`ctx`는 OS별 표기용 — Windows `Ctrl+S`, macOS `⌘S`).
pub fn shortcut_sections(keymap: &Keymap, ctx: &egui::Context) -> Vec<ShortcutSection> {
    let format = |shortcuts: &[KeyboardShortcut]| {
        let mut keys: Vec<String> = Vec::new();
        for s in shortcuts {
            let text = ctx.format_shortcut(s);
            if !keys.contains(&text) {
                keys.push(text);
            }
        }
        keys.join(" · ")
    };

    let mut sections: Vec<ShortcutSection> = SECTIONS
        .iter()
        .map(|(title, commands)| {
            let rows = commands
                .iter()
                .filter_map(|&command| {
                    let (action, keys) = match command {
                        Command::GotoTab(_) => (
                            "1~8번째 탭으로 이동".to_owned(),
                            format!(
                                "{} ~ {}",
                                format(&keymap.shortcuts_for(Command::GotoTab(0))),
                                format(&keymap.shortcuts_for(Command::GotoTab(7)))
                            ),
                        ),
                        Command::ToggleMdSplit => {
                            (command.label(), keymap.format_shortcut_for(command, ctx)?)
                        }
                        _ => (command.label(), format(&keymap.shortcuts_for(command))),
                    };
                    (!keys.is_empty()).then_some(ShortcutRow { action, keys })
                })
                .collect();
            ShortcutSection { title, rows }
        })
        .collect();

    sections.insert(
        3,
        ShortcutSection {
            title: "찾기 바 안에서",
            rows: SEARCH_BAR_KEYS
                .iter()
                .map(|&(action, modifiers, key)| ShortcutRow {
                    action: action.to_owned(),
                    keys: ctx.format_shortcut(&KeyboardShortcut::new(modifiers, key)),
                })
                .collect(),
        },
    );
    sections
}

/// 정규식 문법 표의 한 묶음: (제목, [(문법, 설명)]).
pub type RegexSection = (&'static str, &'static [(&'static str, &'static str)]);

/// 정규식 문법 창 내용 (찾기 엔진 `core::search` — Rust `regex` 문법).
pub const REGEX_SECTIONS: &[RegexSection] = &[
    (
        "글자",
        &[
            (".", "줄바꿈을 뺀 아무 글자 하나"),
            (r"\d", "숫자"),
            (r"\w", "단어 글자 (한글·영문·숫자·_)"),
            (r"\s", "공백 (스페이스·탭·줄바꿈)"),
            (r"\D  \W  \S", "위 세 가지의 반대"),
            ("[abc]", "a, b, c 중 하나"),
            ("[^abc]", "a, b, c를 뺀 글자 하나"),
            (
                "[a-z]  [가-힣]",
                "범위 안의 글자 하나 (영문 소문자, 한글 음절)",
            ),
            (r"\t  \n", "탭, 줄바꿈"),
            (r"\.  \*  \(  \\", "특수 문자를 글자 그대로"),
        ],
    ),
    (
        "위치",
        &[
            ("^", "줄의 처음"),
            ("$", "줄의 끝"),
            (r"\b", "단어 경계"),
            (r"\A  \z", "문서 전체의 처음, 끝"),
        ],
    ),
    (
        "반복",
        &[
            ("*", "0번 이상"),
            ("+", "1번 이상"),
            ("?", "0번 또는 1번"),
            ("{3}", "정확히 3번"),
            ("{2,5}  {2,}", "2~5번, 2번 이상"),
            ("*?  +?", "가능한 한 짧게"),
        ],
    ),
    (
        "그룹·선택",
        &[
            ("a|b", "a 또는 b"),
            ("(…)", "그룹 — 바꿀 내용에서 $1, $2…로 쓴다"),
            (
                "(?<name>…)",
                "이름 있는 그룹 — 바꿀 내용에서 ${name}으로 쓴다",
            ),
            ("(?:…)", "번호를 매기지 않는 그룹"),
            ("(?i)", "이 위치부터 대소문자 무시"),
        ],
    ),
    (
        "바꿀 내용 (정규식 모드)",
        &[
            ("$1  ${name}", "찾은 그룹의 내용"),
            (
                "${1}a",
                "그룹 바로 뒤에 글자를 붙일 때 ($1a는 그룹 이름 \"1a\"로 읽힘)",
            ),
            ("$$", "$ 글자"),
            (r"\n  \t  \\", "줄바꿈, 탭, 역슬래시"),
        ],
    ),
    (
        "예시",
        &[
            (r"\s+$", "줄 끝 공백 (모두 바꾸기로 지우기)"),
            (r"^\s*\n", "빈 줄"),
            (
                r"(\d{4})-(\d{2})-(\d{2})",
                "2026-10-27 → 바꿀 내용 $1.$2.$3 으로 2026.10.27",
            ),
        ],
    ),
    (
        "지원하지 않음",
        &[
            ("(?=…)  (?!…)", "뒤 내용 미리 보기 (전방 탐색)"),
            ("(?<=…)  (?<!…)", "앞 내용 확인 (후방 탐색)"),
            (r"\1", "찾을 내용 안에서 앞 그룹 다시 쓰기 (역참조)"),
        ],
    ),
];

/// 정보 창에 보이는 버전 (`Cargo.toml`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 정보 창·버그 신고용 한 줄 (`Vic Notepad 0.1.0 (Windows x86_64)`).
pub fn version_line() -> String {
    format!("{} {VERSION} ({})", crate::branding::APP_NAME, platform())
}

/// 실행 중인 OS·CPU (`Windows x86_64`, `macOS aarch64`).
pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    format!("{os} {}", std::env::consts::ARCH)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::core::search::{SearchOptions, SearchQuery};

    /// 키맵의 모든 단축키가 목록 어딘가에 실려 있어야 한다 (새 단축키를 추가하고 목록을 빠뜨리지 않게).
    #[test]
    fn every_bound_command_is_listed() {
        for is_macos in [false, true] {
            let keymap = Keymap::for_os(is_macos);
            let listed: Vec<Command> = SECTIONS
                .iter()
                .flat_map(|(_, commands)| commands.iter().copied())
                .collect();
            for binding in keymap.bindings() {
                let command = match binding.command {
                    Command::GotoTab(_) => Command::GotoTab(0),
                    other => other,
                };
                assert!(
                    listed.contains(&command),
                    "단축키 목록에 없음 (is_macos={is_macos}): {command:?}"
                );
            }
            for command in [
                Command::Cut,
                Command::Copy,
                Command::Paste,
                Command::SelectAll,
            ] {
                assert!(listed.contains(&command));
            }
        }
    }

    /// 첫 프레임을 한 번 돌린 컨텍스트 — macOS 단축키 표기(`⌘`)는 글꼴을 확인하므로 첫 프레임 전에는 만들 수 없다
    /// (앱에서는 늘 프레임 안에서 부른다).
    pub(crate) fn started_context() -> egui::Context {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        ctx
    }

    #[test]
    fn sections_show_os_shortcuts_and_filter() {
        let ctx = started_context();
        let sections = shortcut_sections(&Keymap::current(), &ctx);
        let titles: Vec<_> = sections.iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            ["파일", "편집", "검색", "찾기 바 안에서", "보기", "창"]
        );
        assert!(sections.iter().all(|s| !s.rows.is_empty()));

        let row = |action: &str| {
            sections
                .iter()
                .flat_map(|s| &s.rows)
                .find(|r| r.action == action)
                .unwrap_or_else(|| panic!("{action} 줄 없음"))
                .clone()
        };
        let save = row("저장");
        assert_eq!(
            save.keys,
            ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
        );
        // 다음 탭은 단축키가 여러 개
        assert!(row("다음 탭").keys.contains(" · "));
        assert!(row("1~8번째 탭으로 이동").keys.contains(" ~ "));
        assert!(row(&Command::ToggleMdSplit.label()).keys.ends_with(" V"));

        assert!(save.matches("저장"));
        assert!(save.matches(&save.keys.to_lowercase()));
        assert!(save.matches("  "));
        assert!(!save.matches("탭"));
    }

    /// 문법 창의 예시·글자 항목이 실제 찾기 엔진에서 컴파일되고, "지원하지 않음"은 오류가 나야 한다.
    #[test]
    fn regex_help_matches_search_engine() {
        let options = SearchOptions {
            regex: true,
            ..SearchOptions::default()
        };
        for (title, rows) in REGEX_SECTIONS {
            if matches!(*title, "바꿀 내용 (정규식 모드)") {
                continue;
            }
            for (syntax, _) in *rows {
                for pattern in syntax.split("  ") {
                    let mut pattern = pattern.replace('…', "a");
                    if *title == "반복" {
                        pattern.insert(0, 'a');
                    }
                    let result = SearchQuery::new(&pattern, &options);
                    if *title == "지원하지 않음" {
                        assert!(result.is_err(), "지원하지 않아야 함: {pattern}");
                    } else {
                        assert!(result.is_ok(), "{title}: {pattern} → {:?}", result.err());
                    }
                }
            }
        }
    }

    #[test]
    fn version_line_names_product_and_version() {
        let line = version_line();
        assert!(line.starts_with("Vic Notepad "));
        assert!(line.contains(VERSION));
    }
}
