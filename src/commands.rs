//! 앱 전역 `Command` 열거형, OS별 단축키 매핑(`Keymap`), 위젯 선처리 입력 디스패처(`KeyDispatcher`)
//! (PLAN.md §3.5, §4.1 — P1-03).
//!
//! 설계 원칙:
//! - 메뉴·단축키·도구 모음은 모두 동일한 [`Command`]를 발행하고 `app.rs`에서 단일 경로로 처리한다.
//! - egui의 `TextEdit`이 키 입력을 먼저 소비하거나 문자를 삽입하지 않도록, 전역 단축키는
//!   위젯 렌더링 **이전에** [`KeyDispatcher::dispatch`]로 선처리한다.
//! - 수식키는 **정확히 일치**할 때만 매칭한다 ([`shortcut_matches`]). egui의 `consume_shortcut`은
//!   바인딩에 없는 `Shift`/`Alt`를 무시하므로(`Ctrl+Shift+Z` → 실행 취소, AltGr(`Ctrl+Alt`)+7 → 7번째 탭)
//!   쓰지 않는다. 예외: `+`처럼 `Shift`로 입력되는 기호 키, `Shift`로 바뀐 기호(`{` → `[`).
//! - 바인딩 목록은 수식키가 더 많은 조합을 먼저 둔다 (정확 일치이므로 정확성에는 영향 없음, 가독성용).
//! - 연속 입력 코드(`Ctrl/Cmd+K` 다음 `V` → [`Command::ToggleMdSplit`])의 두 번째 키는 논리 키 또는
//!   물리 키(`physical_key`)가 `V`이면 인식하고, 같은 프레임의 `Event::Text`와 IME 조합 이벤트
//!   (`v`/`V`/두벌식 `ㅍ`)를 제거해 한글 입력 상태에서도 본문에 글자가 찍히지 않게 한다.

use eframe::egui::{self, Event, ImeEvent, Key, KeyboardShortcut, Modifiers, ThemePreference};

/// `Ctrl/Cmd+K` 다음 `V` 키가 만들어 내는 문자 (영문 소문자·대문자, 한글 두벌식 `ㅍ`).
const CHORD_V_TEXTS: [&str; 3] = ["v", "V", "ㅍ"];

/// 입력된 키·수식키가 단축키와 일치하는지 판정한다.
///
/// - `Ctrl`·`Cmd`·`Alt`·`Shift`는 모두 바인딩과 정확히 같아야 한다. 바인딩의 `COMMAND`는
///   Windows에서 `Ctrl`, macOS에서 `Cmd(⌘)`이고 `CTRL`은 두 OS 모두 `Ctrl`이다.
///   (egui의 `cmd_ctrl_matches`는 macOS에서 추가로 눌린 `Ctrl`을 허용하므로 쓰지 않는다.)
/// - `Shift`로 입력되는 기호 키(`+`)는 `Shift` 여부를 따지지 않는다 (US 배열 `Shift+=`, 숫자 키패드 `+`).
/// - `Shift`를 포함한 바인딩은 `Shift`로 바뀐 기호(`{` `}`)도 원래 키(`[` `]`)로 인정한다
///   (macOS `Cmd+Shift+[`가 논리 키 `{`로 들어오는 경우).
pub fn shortcut_matches(
    shortcut: &KeyboardShortcut,
    key: Key,
    modifiers: Modifiers,
    is_macos: bool,
) -> bool {
    let pattern = shortcut.modifiers;
    let (want_ctrl, want_cmd) = if is_macos {
        (pattern.ctrl, pattern.command || pattern.mac_cmd)
    } else {
        (pattern.ctrl || pattern.command, false)
    };
    let (has_ctrl, has_cmd) = if is_macos {
        (modifiers.ctrl, modifiers.command || modifiers.mac_cmd)
    } else {
        (modifiers.ctrl || modifiers.command, false)
    };
    if has_ctrl != want_ctrl || has_cmd != want_cmd || modifiers.alt != pattern.alt {
        return false;
    }
    if key == shortcut.logical_key {
        return modifiers.shift == pattern.shift || matches!(key, Key::Plus);
    }
    let shifted_base = match key {
        Key::OpenCurlyBracket => Some(Key::OpenBracket),
        Key::CloseCurlyBracket => Some(Key::CloseBracket),
        _ => None,
    };
    modifiers.shift && pattern.shift && shifted_base == Some(shortcut.logical_key)
}

/// [`shortcut_matches`]로 일치하는 첫 키 누름 이벤트를 소비한다.
fn consume_exact(
    input: &mut egui::InputState,
    shortcut: &KeyboardShortcut,
    is_macos: bool,
) -> bool {
    let position = input.events.iter().position(|e| {
        matches!(e, Event::Key { key, pressed: true, modifiers, .. }
            if shortcut_matches(shortcut, *key, *modifiers, is_macos))
    });
    if let Some(position) = position {
        input.events.remove(position);
        true
    } else {
        false
    }
}

/// 문서 인코딩 선택지 (PLAN.md §3.3 — `core::encoding::TextEncoding` 재노출).
pub use crate::core::encoding::TextEncoding as EncodingChoice;

/// 줄바꿈 방식 선택지 (PLAN.md §3.4 — `core::line_ending::LineEnding` 재노출).
pub use crate::core::line_ending::LineEnding as LineEndingChoice;

/// Markdown 문서 보기 모드 (PLAN.md §3.8). 설정 파일에는 `"source"`·`"split"`·`"preview"`로 저장한다 (P3-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    Source,
    Split,
    Preview,
}

impl ViewMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Source => "소스",
            Self::Split => "분할",
            Self::Preview => "미리보기",
        }
    }
}

/// 앱 테마 설정 (PLAN.md §3.7). 설정 파일에는 `"system"`·`"light"`·`"dark"`로 저장한다 (P2-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "시스템 설정 따르기",
            Self::Light => "라이트",
            Self::Dark => "다크",
        }
    }

    pub const fn to_egui(self) -> ThemePreference {
        match self {
            Self::System => ThemePreference::System,
            Self::Light => ThemePreference::Light,
            Self::Dark => ThemePreference::Dark,
        }
    }
}

/// 메뉴·단축키·UI 버튼이 발행하는 단일 애플리케이션 커맨드 열거형.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    // ── 파일 (File) ──
    NewTab,
    OpenFile,
    OpenFolder,
    OpenRecent(usize),
    ClearRecentFiles,
    Save,
    SaveAs,
    CloseTab,
    Exit,

    // ── 편집 (Edit) ──
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    GoToLine,

    // ── 검색 (Search) ──
    Find,
    Replace,
    FindNext,
    FindPrev,
    /// 찾기: 끝에서 처음으로 이어서 찾기(Wrap Around) 토글 (P4-04)
    ToggleSearchWrapAround,
    /// 선택 단어 강조: 단어 단위 일치 ↔ 부분 일치 (P2-04)
    ToggleHighlightWholeWord,
    /// 선택 단어 강조: 대소문자 구분 토글 (P2-04)
    ToggleHighlightMatchCase,

    // ── 보기 (View) ──
    ToggleSidebar,
    ToggleWordWrap,
    /// 탭 너비 (공백 칸 수, P2-02)
    SetTabWidth(u8),
    /// `Tab` 키로 탭 문자 대신 공백 넣기 토글 (P2-02)
    ToggleTabSpaces,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    SetViewMode(ViewMode),
    ToggleMdPreview,
    ToggleMdSplit,
    ToggleScrollSync,
    SetTheme(ThemeMode),

    // ── 인코딩·줄바꿈 (Encoding & Line Ending) ──
    ReopenWithEncoding(EncodingChoice),
    ConvertToEncoding(EncodingChoice),
    SetLineEnding(LineEndingChoice),

    // ── 창·탭 (Window & Tab) ──
    NextTab,
    PrevTab,
    /// 0-기반 탭 인덱스 (`0..8`은 `Ctrl/Cmd+1..=8` 바로가기 대응).
    GotoTab(usize),
    LastTab,
    MoveTabLeft,
    MoveTabRight,
    CloseOtherTabs,
    CloseTabsToRight,
    CloseSavedTabs,
    CloseAllTabs,
    CopyTabPath,
    ShowWindowList,

    // ── 도움말 (Help) ──
    ShowShortcuts,
    ShowRegexHelp,
    ShowAbout,
}

impl Command {
    /// 상태 표시줄·로그·테스트에서 표시할 한국어 커맨드 이름.
    pub fn label(self) -> String {
        match self {
            Self::NewTab => "새 탭".to_owned(),
            Self::OpenFile => "열기".to_owned(),
            Self::OpenFolder => "폴더 열기".to_owned(),
            Self::OpenRecent(i) => format!("최근 파일 열기 (#{})", i + 1),
            Self::ClearRecentFiles => "최근 파일 목록 지우기".to_owned(),
            Self::Save => "저장".to_owned(),
            Self::SaveAs => "다른 이름으로 저장".to_owned(),
            Self::CloseTab => "탭 닫기".to_owned(),
            Self::Exit => "종료".to_owned(),

            Self::Undo => "실행 취소".to_owned(),
            Self::Redo => "다시 실행".to_owned(),
            Self::Cut => "잘라내기".to_owned(),
            Self::Copy => "복사".to_owned(),
            Self::Paste => "붙여넣기".to_owned(),
            Self::SelectAll => "모두 선택".to_owned(),
            Self::GoToLine => "줄 이동".to_owned(),

            Self::Find => "찾기".to_owned(),
            Self::Replace => "바꾸기".to_owned(),
            Self::FindNext => "다음 찾기".to_owned(),
            Self::FindPrev => "이전 찾기".to_owned(),
            Self::ToggleSearchWrapAround => "찾기: 끝에서 처음으로 이어서 찾기 토글".to_owned(),
            Self::ToggleHighlightWholeWord => "선택 단어 강조: 단어 단위 일치 토글".to_owned(),
            Self::ToggleHighlightMatchCase => "선택 단어 강조: 대소문자 구분 토글".to_owned(),

            Self::ToggleSidebar => "사이드바 토글".to_owned(),
            Self::ToggleWordWrap => "자동 줄바꿈 토글".to_owned(),
            Self::SetTabWidth(width) => format!("탭 너비: {width}칸"),
            Self::ToggleTabSpaces => "Tab 키로 공백 넣기 토글".to_owned(),
            Self::ZoomIn => "확대".to_owned(),
            Self::ZoomOut => "축소".to_owned(),
            Self::ZoomReset => "원래 크기".to_owned(),
            Self::SetViewMode(mode) => format!("Markdown 보기: {}", mode.label()),
            Self::ToggleMdPreview => "Markdown 소스 ↔ 미리보기 전환".to_owned(),
            Self::ToggleMdSplit => "Markdown 분할 보기 토글".to_owned(),
            Self::ToggleScrollSync => "스크롤 동기화 토글".to_owned(),
            Self::SetTheme(theme) => format!("테마: {}", theme.label()),

            Self::ReopenWithEncoding(enc) => format!("인코딩으로 다시 읽기: {}", enc.label()),
            Self::ConvertToEncoding(enc) => format!("인코딩 변환 저장: {}", enc.label()),
            Self::SetLineEnding(le) => format!("줄바꿈 변경: {}", le.short_label()),

            Self::NextTab => "다음 탭".to_owned(),
            Self::PrevTab => "이전 탭".to_owned(),
            Self::GotoTab(i) => format!("{}번째 탭으로 이동", i + 1),
            Self::LastTab => "마지막 탭으로 이동".to_owned(),
            Self::MoveTabLeft => "탭을 왼쪽으로 이동".to_owned(),
            Self::MoveTabRight => "탭을 오른쪽으로 이동".to_owned(),
            Self::CloseOtherTabs => "다른 탭 모두 닫기".to_owned(),
            Self::CloseTabsToRight => "오른쪽 탭 모두 닫기".to_owned(),
            Self::CloseSavedTabs => "저장된 탭 닫기".to_owned(),
            Self::CloseAllTabs => "모든 탭 닫기".to_owned(),
            Self::CopyTabPath => "탭 경로 복사".to_owned(),
            Self::ShowWindowList => "창 목록".to_owned(),

            Self::ShowShortcuts => "단축키 목록".to_owned(),
            Self::ShowRegexHelp => "정규식 문법".to_owned(),
            Self::ShowAbout => format!("{} 정보", crate::branding::APP_NAME),
        }
    }
}

/// 단일 키보드 단축키와 실행할 [`Command`]의 매핑.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutBinding {
    pub shortcut: KeyboardShortcut,
    pub command: Command,
}

/// OS별 단축키 테이블 (PLAN.md §3.5).
///
/// - `bindings`: 위젯보다 먼저 입력에서 소비할 전역 단축키 목록 (우선순위 순 정렬:
///   같은 키에서는 `Shift`/`Alt`가 포함된 조합이 항상 먼저 위치).
/// - `editor_menu_shortcuts`: `TextEdit` 위젯이 자체 처리하는 클립보드/선택 단축키
///   (`Cut`, `Copy`, `Paste`, `SelectAll`)의 메뉴 표기용 매핑.
/// - `chord_k`: 연속 입력 단축키(`Ctrl/Cmd+K V` → [`Command::ToggleMdSplit`])의 첫 번째 키.
#[derive(Debug, Clone)]
pub struct Keymap {
    is_macos: bool,
    bindings: Vec<ShortcutBinding>,
    editor_menu_shortcuts: Vec<ShortcutBinding>,
    chord_k: KeyboardShortcut,
}

impl Keymap {
    /// 현재 실행 중인 OS에 맞는 키맵을 생성한다.
    pub fn current() -> Self {
        Self::for_os(cfg!(target_os = "macos"))
    }

    /// 지정한 OS(`is_macos = true`이면 macOS, `false`이면 Windows/기타)용 키맵을 생성한다.
    ///
    /// 단위 테스트에서 한 플랫폼 위에서도 Windows와 macOS 양쪽 키맵을 모두 검증할 수 있도록
    /// OS 여부를 인자로 받는다.
    pub fn for_os(is_macos: bool) -> Self {
        let cmd = Modifiers::COMMAND;
        let ctrl = Modifiers::CTRL;
        let shift = Modifiers::SHIFT;
        let alt = Modifiers::ALT;
        let b = |modifiers: Modifiers, logical_key: Key, command: Command| ShortcutBinding {
            shortcut: KeyboardShortcut::new(modifiers, logical_key),
            command,
        };

        let mut bindings = Vec::with_capacity(36);

        // 1. 파일: Shift 조합(SaveAs, CloseAllTabs)을 기본 조합(Save, CloseTab)보다 먼저 배치
        bindings.push(b(cmd, Key::N, Command::NewTab));
        bindings.push(b(cmd, Key::O, Command::OpenFile));
        bindings.push(b(cmd | shift, Key::S, Command::SaveAs));
        bindings.push(b(cmd, Key::S, Command::Save));
        bindings.push(b(cmd | shift, Key::W, Command::CloseAllTabs));
        bindings.push(b(cmd, Key::W, Command::CloseTab));
        if is_macos {
            bindings.push(b(cmd, Key::Q, Command::Exit));
        } else {
            bindings.push(b(alt, Key::F4, Command::Exit));
        }

        // 2. 탭 이동·정렬: Shift/Option 조합(MoveTabLeft/Right, PrevTab)을 NextTab보다 먼저 배치
        if is_macos {
            bindings.push(b(cmd | shift | alt, Key::ArrowLeft, Command::MoveTabLeft));
            bindings.push(b(cmd | shift | alt, Key::ArrowRight, Command::MoveTabRight));
            bindings.push(b(ctrl | shift, Key::Tab, Command::PrevTab));
            bindings.push(b(cmd | shift, Key::OpenBracket, Command::PrevTab));
            bindings.push(b(cmd | alt, Key::ArrowLeft, Command::PrevTab));
            bindings.push(b(ctrl, Key::Tab, Command::NextTab));
            bindings.push(b(cmd | shift, Key::CloseBracket, Command::NextTab));
            bindings.push(b(cmd | alt, Key::ArrowRight, Command::NextTab));
        } else {
            bindings.push(b(cmd | shift, Key::PageUp, Command::MoveTabLeft));
            bindings.push(b(cmd | shift, Key::PageDown, Command::MoveTabRight));
            bindings.push(b(ctrl | shift, Key::Tab, Command::PrevTab));
            bindings.push(b(cmd, Key::PageUp, Command::PrevTab));
            bindings.push(b(ctrl, Key::Tab, Command::NextTab));
            bindings.push(b(cmd, Key::PageDown, Command::NextTab));
        }

        // 1~8번째 탭 및 마지막 탭(9)
        let digit_keys = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
        ];
        for (idx, key) in digit_keys.into_iter().enumerate() {
            bindings.push(b(cmd, key, Command::GotoTab(idx)));
        }
        bindings.push(b(cmd, Key::Num9, Command::LastTab));

        // 3. 편집: macOS에서는 Redo(Cmd+Shift+Z)가 Undo(Cmd+Z)보다 먼저 와야 함
        if is_macos {
            bindings.push(b(cmd | shift, Key::Z, Command::Redo));
            bindings.push(b(cmd, Key::Z, Command::Undo));
            bindings.push(b(cmd, Key::L, Command::GoToLine));
        } else {
            bindings.push(b(cmd, Key::Y, Command::Redo));
            bindings.push(b(cmd, Key::Z, Command::Undo));
            bindings.push(b(cmd, Key::G, Command::GoToLine));
        }

        // 4. 검색: macOS에서는 Replace(Cmd+Option+F)가 Find(Cmd+F)보다 먼저,
        //    FindPrev(Shift+F3 / Cmd+Shift+G)가 FindNext(F3 / Cmd+G)보다 먼저 와야 함
        if is_macos {
            bindings.push(b(cmd | alt, Key::F, Command::Replace));
            bindings.push(b(cmd, Key::F, Command::Find));
            bindings.push(b(cmd | shift, Key::G, Command::FindPrev));
            bindings.push(b(cmd, Key::G, Command::FindNext));
        } else {
            bindings.push(b(cmd, Key::H, Command::Replace));
            bindings.push(b(cmd, Key::F, Command::Find));
            bindings.push(b(shift, Key::F3, Command::FindPrev));
            bindings.push(b(Modifiers::NONE, Key::F3, Command::FindNext));
        }

        // 5. 보기·확대/축소·Markdown
        bindings.push(b(cmd, Key::B, Command::ToggleSidebar));
        bindings.push(b(cmd | shift, Key::V, Command::ToggleMdPreview));
        bindings.push(b(cmd, Key::Equals, Command::ZoomIn));
        bindings.push(b(cmd, Key::Plus, Command::ZoomIn));
        bindings.push(b(cmd, Key::Minus, Command::ZoomOut));
        bindings.push(b(cmd, Key::Num0, Command::ZoomReset));

        let editor_menu_shortcuts = vec![
            b(cmd, Key::X, Command::Cut),
            b(cmd, Key::C, Command::Copy),
            b(cmd, Key::V, Command::Paste),
            b(cmd, Key::A, Command::SelectAll),
        ];

        Self {
            is_macos,
            bindings,
            editor_menu_shortcuts,
            chord_k: KeyboardShortcut::new(cmd, Key::K),
        }
    }

    /// macOS 키맵 여부 (수식키 판정 규칙이 OS마다 다르다 — [`shortcut_matches`]).
    pub fn is_macos(&self) -> bool {
        self.is_macos
    }

    /// 위젯보다 먼저 처리되는 전역 단축키 바인딩 목록 (우선순위 순).
    pub fn bindings(&self) -> &[ShortcutBinding] {
        &self.bindings
    }

    /// 연속 입력(`Ctrl/Cmd+K V`)의 첫 번째 키(`Ctrl/Cmd+K`).
    pub fn chord_k(&self) -> KeyboardShortcut {
        self.chord_k
    }

    /// 특정 [`Command`]에 대응하는 대표(첫 번째) [`KeyboardShortcut`]을 반환한다.
    pub fn primary_shortcut(&self, command: Command) -> Option<KeyboardShortcut> {
        self.bindings
            .iter()
            .chain(self.editor_menu_shortcuts.iter())
            .find(|b| b.command == command)
            .map(|b| b.shortcut)
    }

    /// 특정 [`Command`]에 대응하는 모든 [`KeyboardShortcut`] (도움말 → 단축키 목록, P5-UX).
    pub fn shortcuts_for(&self, command: Command) -> Vec<KeyboardShortcut> {
        self.bindings
            .iter()
            .chain(self.editor_menu_shortcuts.iter())
            .filter(|b| b.command == command)
            .map(|b| b.shortcut)
            .collect()
    }

    /// 메뉴 항목 우측에 표시할 단축키 문자열을 포맷한다.
    ///
    /// - 일반 단축키: `ctx.format_shortcut(...)` (Windows `Ctrl+S`, macOS `⌘S` 등)
    /// - 연속 입력(`Command::ToggleMdSplit`): `Ctrl+K V` / `⌘K V`
    /// - 메뉴 전용 항목: `None`
    pub fn format_shortcut_for(&self, command: Command, ctx: &egui::Context) -> Option<String> {
        if command == Command::ToggleMdSplit {
            return Some(format!("{} V", ctx.format_shortcut(&self.chord_k)));
        }
        self.primary_shortcut(command)
            .map(|s| ctx.format_shortcut(&s))
    }
}

/// 전역 단축키 및 연속 입력(`Ctrl/Cmd+K V`) 상태를 관리하는 디스패처.
#[derive(Debug, Default, Clone)]
pub struct KeyDispatcher {
    chord_k_pending: bool,
}

impl KeyDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// 현재 `Ctrl/Cmd+K` 입력 후 다음 키(`V`)를 기다리고 있는지 여부.
    pub fn is_chord_k_pending(&self) -> bool {
        self.chord_k_pending
    }

    /// 위젯 렌더링 **이전에** 호출하여 이번 프레임의 단축키 입력을 소비하고
    /// 발생한 [`Command`]를 반환한다.
    pub fn dispatch(&mut self, ctx: &egui::Context, keymap: &Keymap) -> Option<Command> {
        // 1. 이전 프레임에서 Ctrl/Cmd+K가 눌려 대기 중이면 V 키를 먼저 확인한다.
        if self.chord_k_pending {
            let (v_pressed, v_from_ime, any_other_key) = ctx.input_mut(|i| {
                // 한글 입력 상태에서는 논리 키가 `V`가 아닐 수 있으므로 물리 키도 확인한다.
                // `Shift`는 허용(대문자 입력), `Ctrl`/`Cmd`/`Alt`가 눌린 경우는 다른 단축키로 본다.
                let position = i.events.iter().position(|e| {
                    matches!(e, Event::Key { key, physical_key, pressed: true, modifiers, .. }
                        if (*key == Key::V || *physical_key == Some(Key::V))
                            && !modifiers.alt
                            && !modifiers.ctrl
                            && !modifiers.command
                            && !modifiers.mac_cmd)
                });
                // macOS에서 한글 입력기가 조합 중일 때 누른 V는 winit이 키 이벤트 없이
                // 조합(`Preedit("ㅍ")`)으로만 보낸다 (P1-MAC 1-5c). 이 조합 시작도 V로 본다.
                let from_ime = position.is_none()
                    && i.events.iter().any(|e| {
                        matches!(e, Event::Ime(ImeEvent::Preedit { text, .. })
                            if CHORD_V_TEXTS.contains(&text.as_str()))
                    });
                let pressed = position.is_some() || from_ime;
                if let Some(position) = position {
                    i.events.remove(position);
                }
                if pressed {
                    // 키 이벤트와 같은 프레임에 들어온 문자 입력·IME 조합('v'/'V'/'ㅍ')까지 지워야
                    // TextEdit에 글자가 입력되지 않는다 (P0-04 검증 패턴).
                    i.events.retain(|e| match e {
                        Event::Text(t)
                        | Event::Ime(ImeEvent::Commit(t))
                        | Event::Ime(ImeEvent::Preedit { text: t, .. }) => {
                            !CHORD_V_TEXTS.contains(&t.as_str())
                        }
                        _ => true,
                    });
                }
                // macOS 입력기가 처리한 다른 키도 키 이벤트 없이 조합으로만 오므로, 글자가 있는 조합도
                // "다른 키"로 보고 대기를 푼다 (뒤늦은 'ㅍ'가 분할 보기로 처리되지 않게).
                let other = i.events.iter().any(|e| match e {
                    Event::Key { pressed: true, .. } => true,
                    Event::Ime(ImeEvent::Preedit { text, .. } | ImeEvent::Commit(text)) => {
                        !text.is_empty()
                    }
                    _ => false,
                });
                (pressed, from_ime, other)
            });

            if v_from_ime {
                // 입력기 쪽 조합('ㅍ')도 끊어야 다음 모음과 합쳐져 글자가 되살아나지 않는다.
                ctx.memory_mut(|m| m.interrupt_ime());
            }
            if v_pressed {
                self.chord_k_pending = false;
                return Some(Command::ToggleMdSplit);
            }
            if any_other_key {
                self.chord_k_pending = false;
            }
        }

        // 2. Ctrl/Cmd+K 연속 입력 시작 확인
        let chord_k = keymap.chord_k();
        let is_macos = keymap.is_macos();
        if ctx.input_mut(|i| consume_exact(i, &chord_k, is_macos)) {
            self.chord_k_pending = true;
            return None;
        }

        // 3. 단일 단축키 검사·소비 (수식키 정확 일치)
        ctx.input_mut(|i| {
            for binding in keymap.bindings() {
                if consume_exact(i, &binding.shortcut, is_macos) {
                    return Some(binding.command);
                }
            }
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 대상 OS에서 두 `Modifiers`가 논리적으로 같은 키 조합인지 비교하기 위한 정규화 튜플.
    ///
    /// - Windows(`is_macos = false`)에서는 `Modifiers::COMMAND`와 `Modifiers::CTRL`이 모두 물리 `Ctrl` 키를 의미한다.
    /// - macOS(`is_macos = true`)에서는 `Modifiers::COMMAND`가 `Cmd(⌘)`, `Modifiers::CTRL`이 `Ctrl(⌃)`로 구분된다.
    fn normalized_key_tuple(
        shortcut: KeyboardShortcut,
        is_macos: bool,
    ) -> (bool, bool, bool, bool, Key) {
        let m = shortcut.modifiers;
        let ctrl_bit = if is_macos {
            m.ctrl
        } else {
            m.ctrl || m.command
        };
        let cmd_bit = if is_macos {
            m.command || m.mac_cmd
        } else {
            false
        };
        (ctrl_bit, cmd_bit, m.alt, m.shift, shortcut.logical_key)
    }

    /// `egui-winit`이 실제 OS에서 생성하는 `Event::Key.modifiers`와 동일한 형태의 `Modifiers`를 만든다.
    fn runtime_modifiers(pattern: Modifiers, is_macos: bool) -> Modifiers {
        if is_macos {
            Modifiers {
                alt: pattern.alt,
                ctrl: pattern.ctrl,
                shift: pattern.shift,
                mac_cmd: pattern.command || pattern.mac_cmd,
                command: pattern.command || pattern.mac_cmd,
            }
        } else {
            Modifiers {
                alt: pattern.alt,
                ctrl: pattern.ctrl || pattern.command,
                shift: pattern.shift,
                mac_cmd: false,
                command: pattern.ctrl || pattern.command,
            }
        }
    }

    /// 헤드리스 `egui::Context`에서 단일 단축키 이벤트를 주입해 디스패치 결과를 얻는다.
    fn simulate_shortcut(keymap: &Keymap, shortcut: KeyboardShortcut) -> Option<Command> {
        let modifiers = runtime_modifiers(shortcut.modifiers, keymap.is_macos());
        simulate_key(keymap, shortcut.logical_key, modifiers)
    }

    /// 헤드리스 `egui::Context`에서 키 이벤트 하나(런타임 수식키 그대로)를 주입해 디스패치 결과를 얻는다.
    fn simulate_key(keymap: &Keymap, key: Key, modifiers: Modifiers) -> Option<Command> {
        let ctx = egui::Context::default();
        let mut dispatcher = KeyDispatcher::new();
        let mut raw = egui::RawInput::default();
        raw.events.push(Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });

        let mut dispatched = None;
        let mut out = ctx.run_ui(raw, |ui| {
            dispatched = dispatcher.dispatch(ui.ctx(), keymap);
        });
        out.textures_delta.clear();
        dispatched
    }

    /// `Ctrl/Cmd+K`를 누른 뒤, 두 번째 프레임에 `events`를 주입해 결과 커맨드와 남은 문자·IME 이벤트 수를 얻는다.
    fn simulate_chord(is_macos: bool, events: Vec<Event>) -> (Option<Command>, bool, usize) {
        let km = Keymap::for_os(is_macos);
        let ctx = egui::Context::default();
        let mut dispatcher = KeyDispatcher::new();

        let mut raw1 = egui::RawInput::default();
        raw1.events.push(Event::Key {
            key: Key::K,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: runtime_modifiers(Modifiers::COMMAND, is_macos),
        });
        let mut out1 = ctx.run_ui(raw1, |ui| {
            assert_eq!(dispatcher.dispatch(ui.ctx(), &km), None);
        });
        out1.textures_delta.clear();
        assert!(dispatcher.is_chord_k_pending());

        let raw2 = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut cmd = None;
        let mut remaining = 0;
        let mut out2 = ctx.run_ui(raw2, |ui| {
            let ctx = ui.ctx();
            cmd = dispatcher.dispatch(ctx, &km);
            remaining = ctx.input(|i| {
                i.events
                    .iter()
                    .filter(|e| matches!(e, Event::Text(_) | Event::Ime(_)))
                    .count()
            });
        });
        out2.textures_delta.clear();
        (cmd, dispatcher.is_chord_k_pending(), remaining)
    }

    #[test]
    fn no_duplicate_shortcuts_on_windows_and_macos() {
        for is_macos in [false, true] {
            let keymap = Keymap::for_os(is_macos);
            let mut seen = HashSet::new();

            // chord_k 도 일반 단축키와 겹치면 안 된다.
            let chord_tuple = normalized_key_tuple(keymap.chord_k(), is_macos);
            assert!(
                seen.insert(chord_tuple),
                "chord_k가 중복되었습니다 (is_macos={is_macos})"
            );

            for binding in keymap.bindings() {
                let tuple = normalized_key_tuple(binding.shortcut, is_macos);
                assert!(
                    seen.insert(tuple),
                    "중복 단축키 발견 (is_macos={is_macos}): {:?} -> {:?}",
                    binding.shortcut,
                    binding.command
                );
            }
        }
    }

    #[test]
    fn shift_and_alt_combinations_precede_plain_shortcuts_for_same_key() {
        for is_macos in [false, true] {
            let keymap = Keymap::for_os(is_macos);
            let bindings = keymap.bindings();

            for i in 0..bindings.len() {
                for j in (i + 1)..bindings.len() {
                    let a = bindings[i].shortcut;
                    let b = bindings[j].shortcut;
                    if a.logical_key != b.logical_key {
                        continue;
                    }
                    let (a_ctrl, a_cmd, a_alt, a_shift, _) = normalized_key_tuple(a, is_macos);
                    let (b_ctrl, b_cmd, b_alt, b_shift, _) = normalized_key_tuple(b, is_macos);

                    // 뒤쪽(j) 바인딩이 앞쪽(i) 바인딩의 수식키 초집합이면 앞쪽에서 먼저 소비될 위험이 있다.
                    let b_is_superset_of_a = (b_ctrl || !a_ctrl)
                        && (b_cmd || !a_cmd)
                        && (b_alt || !a_alt)
                        && (b_shift || !a_shift)
                        && (b_ctrl != a_ctrl
                            || b_cmd != a_cmd
                            || b_alt != a_alt
                            || b_shift != a_shift);

                    assert!(
                        !b_is_superset_of_a,
                        "수식키 우선순위 위반 (is_macos={is_macos}): {:?}({:?})가 {:?}({:?}) 뒤에 배치됨",
                        b, bindings[j].command, a, bindings[i].command
                    );
                }
            }
        }
    }

    #[test]
    fn windows_keymap_matches_plan_section_3_5_table() {
        let km = Keymap::for_os(false);
        let cmd = Modifiers::COMMAND;
        let ctrl = Modifiers::CTRL;
        let shift = Modifiers::SHIFT;
        let alt = Modifiers::ALT;
        let s = KeyboardShortcut::new;

        let expected = [
            (s(cmd, Key::N), Command::NewTab),
            (s(cmd, Key::O), Command::OpenFile),
            (s(cmd, Key::S), Command::Save),
            (s(cmd | shift, Key::S), Command::SaveAs),
            (s(cmd, Key::W), Command::CloseTab),
            // 다음 탭 (2가지)
            (s(ctrl, Key::Tab), Command::NextTab),
            (s(cmd, Key::PageDown), Command::NextTab),
            // 이전 탭 (2가지)
            (s(ctrl | shift, Key::Tab), Command::PrevTab),
            (s(cmd, Key::PageUp), Command::PrevTab),
            // 1~8번째 탭 / 마지막 탭
            (s(cmd, Key::Num1), Command::GotoTab(0)),
            (s(cmd, Key::Num2), Command::GotoTab(1)),
            (s(cmd, Key::Num3), Command::GotoTab(2)),
            (s(cmd, Key::Num4), Command::GotoTab(3)),
            (s(cmd, Key::Num5), Command::GotoTab(4)),
            (s(cmd, Key::Num6), Command::GotoTab(5)),
            (s(cmd, Key::Num7), Command::GotoTab(6)),
            (s(cmd, Key::Num8), Command::GotoTab(7)),
            (s(cmd, Key::Num9), Command::LastTab),
            // 탭 좌/우 이동
            (s(cmd | shift, Key::PageUp), Command::MoveTabLeft),
            (s(cmd | shift, Key::PageDown), Command::MoveTabRight),
            // 모든 탭 닫기
            (s(cmd | shift, Key::W), Command::CloseAllTabs),
            // 실행 취소 / 다시 실행
            (s(cmd, Key::Z), Command::Undo),
            (s(cmd, Key::Y), Command::Redo),
            // 찾기 / 바꾸기 / 다음 찾기 / 이전 찾기
            (s(cmd, Key::F), Command::Find),
            (s(cmd, Key::H), Command::Replace),
            (s(Modifiers::NONE, Key::F3), Command::FindNext),
            (s(shift, Key::F3), Command::FindPrev),
            // 줄 이동 / 사이드바 / Markdown 미리보기 토글 / 종료
            (s(cmd, Key::G), Command::GoToLine),
            (s(cmd, Key::B), Command::ToggleSidebar),
            (s(cmd | shift, Key::V), Command::ToggleMdPreview),
            (s(alt, Key::F4), Command::Exit),
            // 글꼴 확대/축소/초기화
            (s(cmd, Key::Equals), Command::ZoomIn),
            (s(cmd, Key::Plus), Command::ZoomIn),
            (s(cmd, Key::Minus), Command::ZoomOut),
            (s(cmd, Key::Num0), Command::ZoomReset),
        ];

        for (shortcut, expected_cmd) in expected {
            let actual = simulate_shortcut(&km, shortcut);
            assert_eq!(
                actual,
                Some(expected_cmd),
                "Windows 단축키 매핑 불일치: {shortcut:?}"
            );
        }
    }

    #[test]
    fn macos_keymap_matches_plan_section_3_5_table() {
        let km = Keymap::for_os(true);
        let cmd = Modifiers::COMMAND;
        let ctrl = Modifiers::CTRL;
        let shift = Modifiers::SHIFT;
        let alt = Modifiers::ALT;
        let s = KeyboardShortcut::new;

        let expected = [
            (s(cmd, Key::N), Command::NewTab),
            (s(cmd, Key::O), Command::OpenFile),
            (s(cmd, Key::S), Command::Save),
            (s(cmd | shift, Key::S), Command::SaveAs),
            (s(cmd, Key::W), Command::CloseTab),
            // 다음 탭 (3가지)
            (s(ctrl, Key::Tab), Command::NextTab),
            (s(cmd | shift, Key::CloseBracket), Command::NextTab),
            (s(cmd | alt, Key::ArrowRight), Command::NextTab),
            // 이전 탭 (3가지)
            (s(ctrl | shift, Key::Tab), Command::PrevTab),
            (s(cmd | shift, Key::OpenBracket), Command::PrevTab),
            (s(cmd | alt, Key::ArrowLeft), Command::PrevTab),
            // 1~8번째 탭 / 마지막 탭
            (s(cmd, Key::Num1), Command::GotoTab(0)),
            (s(cmd, Key::Num2), Command::GotoTab(1)),
            (s(cmd, Key::Num3), Command::GotoTab(2)),
            (s(cmd, Key::Num4), Command::GotoTab(3)),
            (s(cmd, Key::Num5), Command::GotoTab(4)),
            (s(cmd, Key::Num6), Command::GotoTab(5)),
            (s(cmd, Key::Num7), Command::GotoTab(6)),
            (s(cmd, Key::Num8), Command::GotoTab(7)),
            (s(cmd, Key::Num9), Command::LastTab),
            // 탭 좌/우 이동
            (s(cmd | shift | alt, Key::ArrowLeft), Command::MoveTabLeft),
            (s(cmd | shift | alt, Key::ArrowRight), Command::MoveTabRight),
            // 모든 탭 닫기
            (s(cmd | shift, Key::W), Command::CloseAllTabs),
            // 실행 취소 / 다시 실행
            (s(cmd, Key::Z), Command::Undo),
            (s(cmd | shift, Key::Z), Command::Redo),
            // 찾기 / 바꾸기 / 다음 찾기 / 이전 찾기
            (s(cmd, Key::F), Command::Find),
            (s(cmd | alt, Key::F), Command::Replace),
            (s(cmd, Key::G), Command::FindNext),
            (s(cmd | shift, Key::G), Command::FindPrev),
            // 줄 이동 / 사이드바 / Markdown 미리보기 토글 / 종료
            (s(cmd, Key::L), Command::GoToLine),
            (s(cmd, Key::B), Command::ToggleSidebar),
            (s(cmd | shift, Key::V), Command::ToggleMdPreview),
            (s(cmd, Key::Q), Command::Exit),
            // 글꼴 확대/축소/초기화
            (s(cmd, Key::Equals), Command::ZoomIn),
            (s(cmd, Key::Plus), Command::ZoomIn),
            (s(cmd, Key::Minus), Command::ZoomOut),
            (s(cmd, Key::Num0), Command::ZoomReset),
        ];

        for (shortcut, expected_cmd) in expected {
            let actual = simulate_shortcut(&km, shortcut);
            assert_eq!(
                actual,
                Some(expected_cmd),
                "macOS 단축키 매핑 불일치: {shortcut:?}"
            );
        }
    }

    #[test]
    fn chord_k_then_v_dispatches_toggle_md_split_and_strips_text_event() {
        for is_macos in [false, true] {
            let km = Keymap::for_os(is_macos);
            let ctx = egui::Context::default();
            let mut dispatcher = KeyDispatcher::new();

            // 1프레임: Ctrl/Cmd+K 입력 -> 대기 상태 진입, 커맨드 미발행
            let mut raw1 = egui::RawInput::default();
            let mods_k = runtime_modifiers(Modifiers::COMMAND, is_macos);
            raw1.events.push(Event::Key {
                key: Key::K,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods_k,
            });
            let mut cmd1 = None;
            let mut out1 = ctx.run_ui(raw1, |ui| {
                cmd1 = dispatcher.dispatch(ui.ctx(), &km);
            });
            out1.textures_delta.clear();
            assert_eq!(cmd1, None);
            assert!(dispatcher.is_chord_k_pending());

            // 2프레임: V 키 + Event::Text("v") 동시 진입 -> ToggleMdSplit 발행 및 Text("v") 제거 확인
            let mut raw2 = egui::RawInput::default();
            raw2.events.push(Event::Key {
                key: Key::V,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            });
            raw2.events.push(Event::Text("v".to_owned()));

            let mut cmd2 = None;
            let mut remaining_text_events = 0;
            let mut out2 = ctx.run_ui(raw2, |ui| {
                let ctx = ui.ctx();
                cmd2 = dispatcher.dispatch(ctx, &km);
                remaining_text_events = ctx.input(|i| {
                    i.events
                        .iter()
                        .filter(|e| matches!(e, Event::Text(_)))
                        .count()
                });
            });
            out2.textures_delta.clear();
            assert_eq!(cmd2, Some(Command::ToggleMdSplit));
            assert!(!dispatcher.is_chord_k_pending());
            assert_eq!(
                remaining_text_events, 0,
                "Ctrl/Cmd+K 다음 V 입력 시 Event::Text(\"v\")가 제거되어야 합니다"
            );
        }
    }

    #[test]
    fn chord_k_then_v_in_hangul_mode_strips_ime_events() {
        for is_macos in [false, true] {
            // 한글 입력 상태: 논리 키는 V가 아닐 수 있고(물리 키 V), 'ㅍ' 조합 이벤트가 함께 들어온다.
            let (cmd, pending, remaining) = simulate_chord(
                is_macos,
                vec![
                    Event::Key {
                        key: Key::Questionmark,
                        physical_key: Some(Key::V),
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    },
                    Event::Ime(ImeEvent::Preedit {
                        text: "ㅍ".to_owned(),
                        active_range_chars: None,
                    }),
                    Event::Ime(ImeEvent::Commit("ㅍ".to_owned())),
                    Event::Text("ㅍ".to_owned()),
                ],
            );
            assert_eq!(cmd, Some(Command::ToggleMdSplit), "is_macos={is_macos}");
            assert!(!pending);
            assert_eq!(remaining, 0, "'ㅍ' 문자·IME 이벤트가 제거되어야 함");
        }
    }

    #[test]
    fn chord_k_then_v_as_macos_hangul_preedit_only_dispatches_and_strips() {
        // macOS 한글 입력기가 조합 중일 때: V 키 이벤트 없이 Preedit("ㅍ")만 들어온다 (P1-MAC 1-5c).
        let (cmd, pending, remaining) = simulate_chord(
            true,
            vec![Event::Ime(ImeEvent::Preedit {
                text: "ㅍ".to_owned(),
                active_range_chars: Some(1..1),
            })],
        );
        assert_eq!(cmd, Some(Command::ToggleMdSplit));
        assert!(!pending);
        assert_eq!(remaining, 0, "'ㅍ' 조합 이벤트가 제거되어야 함");
    }

    #[test]
    fn chord_k_then_other_hangul_preedit_cancels_and_keeps_composition() {
        let (cmd, pending, remaining) = simulate_chord(
            true,
            vec![Event::Ime(ImeEvent::Preedit {
                text: "ㅎ".to_owned(),
                active_range_chars: Some(1..1),
            })],
        );
        assert_eq!(cmd, None);
        assert!(!pending, "V가 아닌 조합이 시작되면 대기가 풀려야 함");
        assert_eq!(remaining, 1, "V가 아닌 조합은 그대로 남아야 함");
    }

    #[test]
    fn chord_k_then_other_key_cancels_and_keeps_text() {
        let (cmd, pending, remaining) = simulate_chord(
            false,
            vec![
                Event::Key {
                    key: Key::A,
                    physical_key: Some(Key::A),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
                Event::Text("a".to_owned()),
            ],
        );
        assert_eq!(cmd, None);
        assert!(!pending, "V가 아닌 키를 누르면 연속 입력 대기가 풀려야 함");
        assert_eq!(remaining, 1, "V가 아닌 키의 문자 입력은 그대로 남아야 함");
    }

    #[test]
    fn extra_modifiers_do_not_trigger_shortcuts() {
        let cmd = Modifiers::COMMAND;
        let ctrl = Modifiers::CTRL;
        let shift = Modifiers::SHIFT;
        let alt = Modifiers::ALT;
        let s = KeyboardShortcut::new;

        let windows = Keymap::for_os(false);
        for shortcut in [
            s(cmd | shift, Key::Z), // 실행 취소(Ctrl+Z)로 실행되면 안 됨
            s(cmd | shift, Key::N),
            s(cmd | shift, Key::F),
            s(cmd | alt, Key::Num7), // AltGr+7 (`{`, 독일어 배열)
            s(cmd | alt, Key::S),
            s(alt | shift, Key::F4),
            s(cmd | shift, Key::K), // 연속 입력 시작도 정확히 Ctrl+K만
        ] {
            assert_eq!(
                simulate_shortcut(&windows, shortcut),
                None,
                "Windows에서 실행되면 안 됨: {shortcut:?}"
            );
        }

        let macos = Keymap::for_os(true);
        for shortcut in [
            s(cmd | ctrl, Key::Z), // macOS: 추가로 눌린 Ctrl도 거부
            s(cmd | ctrl, Key::F),
            s(cmd | shift, Key::N),
            s(cmd | alt, Key::S),
            s(cmd, Key::Tab), // Ctrl+Tab만 다음 탭
        ] {
            assert_eq!(
                simulate_shortcut(&macos, shortcut),
                None,
                "macOS에서 실행되면 안 됨: {shortcut:?}"
            );
        }
    }

    #[test]
    fn shift_produced_symbols_still_match() {
        let windows = Keymap::for_os(false);
        // US 배열 Ctrl+Shift+= → 논리 키 `+`
        assert_eq!(
            simulate_shortcut(
                &windows,
                KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Plus)
            ),
            Some(Command::ZoomIn)
        );

        let macos = Keymap::for_os(true);
        let cmd_shift = runtime_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, true);
        // macOS Cmd+Shift+[ / ] → 논리 키 `{` / `}`
        assert_eq!(
            simulate_key(&macos, Key::OpenCurlyBracket, cmd_shift),
            Some(Command::PrevTab)
        );
        assert_eq!(
            simulate_key(&macos, Key::CloseCurlyBracket, cmd_shift),
            Some(Command::NextTab)
        );
        // Shift 없이 들어온 `{`는 인정하지 않는다
        let cmd_only = runtime_modifiers(Modifiers::COMMAND, true);
        assert_eq!(simulate_key(&macos, Key::OpenCurlyBracket, cmd_only), None);
    }
}
