//! 모달 대화상자 (PLAN.md §3.6, ui-spec §3 대화상자 문구, §7 창 목록).
//!
//! - P1-10: 파일 열기·저장 오류 안내, 바이너리 파일 열기 확인
//! - P1-11: 미저장 확인(`저장 / 저장 안 함 / 취소`, 기본 버튼 `저장`, `Esc` = 취소)과
//!   일괄 닫기 시 "이어서 닫을 탭 N개" 안내, 창 목록(체크해서 여러 창 닫기·이동)
//! - P1-12: 표현 불가 문자 경고(`UTF-8로 저장` · `?`로 바꿔 저장 · 취소), 손상된 문서 저장 경고,
//!   수정된 문서를 인코딩으로 다시 읽기 전 확인
//! - P1-14: 큰 파일(5MB 초과)을 편집 모드로 열기 전 경고 (기본 버튼 `읽기 전용 유지`)
//! - P2-02: 줄 이동 (줄 번호 입력 → `Enter`/`이동`, 범위 밖이면 대화상자 안에 오류 안내)
//! - P2-07: 외부 변경 확인 (`다시 불러오기` · `내 버전 유지`, 앱에서도 수정했으면 충돌 안내 + 기본 `내 버전 유지`),
//!   디스크에서 지워진 파일을 닫을 때 저장 확인에 삭제 안내
//! - UX-3 피드백: 모든 대화상자를 Notepad++ 형태 공통 모양(제목 띠 + `×` / 본문 / 버튼 띠)으로 통일
//! - P5-UX: 도움말 창 3개 — 단축키 목록(검색란 + 메뉴별 표), 정규식 문법, 정보(아이콘·버전·설정 파일 위치)
//! - P5-UX: 글꼴 — 편집기 글꼴(목록 + 찾기 + 고정폭만)·크기만 고른다 (Notepad++ 스타일 설정처럼 세부 항목은 두지 않음)

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, RichText};

use crate::branding;
use crate::commands::Keymap;
use crate::config::{DEFAULT_EDITOR_FONT_SIZE, EDITOR_FONT_SIZE_RANGE, EditorFont, Settings};
use crate::core::document::DiskMetadata;
use crate::core::encoding::{TextEncoding, UnmappableChar};
use crate::core::file_io::LoadedFile;
use crate::fonts::{self, FontEntry};
use crate::tabs::Tabs;
use crate::ui::help::{self, ShortcutSection};
use crate::ui::tab_bar::{self, UNSAVED_PATH_HINT};
use crate::ui::theme::Palette;
use crate::ui::viewer;

/// 화면에 띄울 대화상자.
#[derive(Debug)]
pub enum Dialog {
    /// 확인 버튼만 있는 안내·오류.
    Message { title: String, body: String },
    /// "텍스트 파일이 아닌 것 같습니다" — 읽기 전용으로 열기 / 취소
    ConfirmBinary(Box<LoadedFile>),
    /// "변경 내용을 저장할까요?" — 닫기 대기열에서 수정된 탭마다 띄운다.
    Unsaved {
        tab_id: u64,
        title: String,
        /// 이 탭 다음에 이어서 닫을 탭 수
        remaining: usize,
        /// 디스크에서 지워진 파일 (닫으면 내용이 사라짐)
        deleted: bool,
    },
    /// 창 목록 (체크한 탭 ID)
    WindowList { checked: HashSet<u64> },
    /// "EUC-KR로 저장할 수 없는 문자가 있습니다" — 저장을 막고 고르게 한다.
    Unmappable {
        tab_id: u64,
        /// 다른 이름으로 저장 중이었다면 그 경로
        target: Option<PathBuf>,
        encoding: TextEncoding,
        chars: Vec<UnmappableChar>,
    },
    /// "손상된 문서를 저장하려고 합니다"
    DamagedSave {
        tab_id: u64,
        target: Option<PathBuf>,
    },
    /// 수정된 문서를 인코딩으로 다시 읽기 전 확인 (변경 내용이 사라짐)
    ConfirmReopen {
        tab_id: u64,
        title: String,
        encoding: TextEncoding,
    },
    /// "큰 파일을 편집 모드로 열까요?" (뷰어 → 편집 위젯)
    ConfirmEditLarge {
        tab_id: u64,
        title: String,
        size_bytes: u64,
        /// 권장 편집 크기 (뷰어 경계값)
        limit_bytes: u64,
    },
    /// "파일이 다른 프로그램에서 변경되었습니다" (P2-07)
    ExternalChange {
        tab_id: u64,
        title: String,
        /// 앱에서도 수정한 상태 (충돌)
        conflict: bool,
        disk: DiskMetadata,
    },
    /// 줄 이동 (`Ctrl+G` / `Cmd+L`)
    GoToLine {
        tab_id: u64,
        line_count: usize,
        current_line: usize,
        input: String,
        error: Option<String>,
    },
    /// 도움말 → 단축키 목록 (P5-UX). 목록은 띄울 때 현재 키맵으로 만든다.
    Shortcuts {
        sections: Vec<ShortcutSection>,
        filter: String,
    },
    /// 도움말 → 정규식 문법 (P5-UX)
    RegexHelp,
    /// 도움말 → Vic Notepad 정보 (P5-UX)
    About {
        /// `버전 정보 복사`를 눌렀음 (버튼 문구로 알린다)
        copied: bool,
    },
    /// 보기 → 글꼴… (편집기 글꼴·크기, P5-UX)
    EditorFont(Box<FontChoice>),
}

/// 글꼴 대화상자 상태.
#[derive(Debug)]
pub struct FontChoice {
    /// 설치된 글꼴 (작업 스레드에서 읽는 중이면 `None`)
    fonts: Option<Vec<FontEntry>>,
    loading: Option<Receiver<Vec<FontEntry>>>,
    /// 고른 글꼴 (`None` = 기본)
    pub selected: Option<EditorFont>,
    pub size: f32,
    monospace_only: bool,
    filter: String,
    /// 미리보기 글꼴을 바꿔 달라는 요청 (앱이 `set_fonts`로 적용하고 비운다, `Some(None)` = 기본)
    pub preview_request: Option<Option<EditorFont>>,
    /// 목록을 다 읽으면 지금 글꼴이 보이게 한 번 스크롤한다
    reveal_selected: bool,
}

impl FontChoice {
    /// 지금 설정으로 연다. 글꼴 목록은 작업 스레드에서 읽는다.
    pub fn new(current: Option<EditorFont>, size: f32) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(fonts::scan_fonts(&fonts::font_dirs()));
        });
        Self {
            fonts: None,
            loading: Some(rx),
            preview_request: Some(current.clone()),
            selected: current,
            size,
            monospace_only: true,
            filter: String::new(),
            reveal_selected: true,
        }
    }

    fn select(&mut self, font: Option<EditorFont>) {
        if self.selected != font {
            self.preview_request = Some(font.clone());
            self.selected = font;
        }
    }
}

/// 대화상자를 닫으며 사용자가 고른 결과.
#[derive(Debug)]
pub enum DialogOutcome {
    Dismissed,
    OpenBinary(Box<LoadedFile>),
    /// 미저장 확인: 저장 후 닫기
    Save(u64),
    /// 미저장 확인: 저장하지 않고 닫기
    Discard(u64),
    /// 미저장 확인: 취소 (남은 닫기 대기열도 취소)
    CancelClose,
    /// 창 목록: 그 탭으로 이동
    Activate(u64),
    /// 창 목록: 체크한(또는 모든) 탭 닫기
    CloseTabs(Vec<u64>),
    /// 표현 불가 문자: UTF-8로 바꿔 저장
    SaveAsUtf8 {
        tab_id: u64,
        target: Option<PathBuf>,
    },
    /// 표현 불가 문자: 문서에서도 `?`로 바꾼 뒤 저장
    SaveReplacing {
        tab_id: u64,
        target: Option<PathBuf>,
    },
    /// 손상된 문서: 그래도 저장
    SaveDamaged {
        tab_id: u64,
        target: Option<PathBuf>,
    },
    /// 큰 파일을 편집 모드로 열기
    OpenEditMode(u64),
    /// 변경 내용을 버리고 다시 읽기
    Reopen {
        tab_id: u64,
        encoding: TextEncoding,
    },
    /// 글꼴: 편집기 글꼴·크기 적용
    SetEditorFont {
        font: Option<EditorFont>,
        size: f32,
    },
    /// 줄 이동: 1-기반 줄 번호
    GoToLine {
        tab_id: u64,
        line: usize,
    },
    /// 외부 변경: 디스크 버전을 다시 불러오기
    ReloadFromDisk(u64),
    /// 외부 변경: 지금 내용 유지 (이 디스크 버전은 다시 묻지 않음)
    KeepMine {
        tab_id: u64,
        disk: DiskMetadata,
    },
}

/// 표현 불가 문자 목록에서 한 번에 보여 줄 최대 줄 수.
const UNMAPPABLE_ROWS: usize = 12;

impl Dialog {
    /// 대화상자 영역 ID. 종류마다 달라야 앞서 띄운 다른 대화상자의 크기를 물려받지 않는다
    /// (예: 창 목록 다음에 뜬 저장 확인이 창 목록 크기로 그려지던 문제).
    pub fn area_id(&self) -> egui::Id {
        let kind = match self {
            Self::Message { .. } => "message",
            Self::ConfirmBinary(_) => "confirm-binary",
            Self::Unsaved { .. } => "unsaved",
            Self::WindowList { .. } => "window-list",
            Self::Unmappable { .. } => "unmappable",
            Self::DamagedSave { .. } => "damaged-save",
            Self::ConfirmReopen { .. } => "confirm-reopen",
            Self::ConfirmEditLarge { .. } => "confirm-edit-large",
            Self::GoToLine { .. } => "go-to-line",
            Self::ExternalChange { .. } => "external-change",
            Self::Shortcuts { .. } => "shortcuts",
            Self::RegexHelp => "regex-help",
            Self::About { .. } => "about",
            Self::EditorFont(_) => "editor-font",
        };
        egui::Id::unique(("app-dialog", kind))
    }

    pub fn message(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::Message {
            title: title.into(),
            body: body.into(),
        }
    }

    /// 줄 이동 대화상자 (`current_line`은 1-기반, 입력란 안내에 보인다).
    pub fn go_to_line(tab_id: u64, line_count: usize, current_line: usize) -> Self {
        Self::GoToLine {
            tab_id,
            line_count,
            current_line,
            input: String::new(),
            error: None,
        }
    }

    pub fn window_list() -> Self {
        Self::WindowList {
            checked: HashSet::new(),
        }
    }

    /// 단축키 목록 (현재 OS 키맵·표기로 만든다).
    pub fn shortcuts(keymap: &Keymap, ctx: &egui::Context) -> Self {
        Self::Shortcuts {
            sections: help::shortcut_sections(keymap, ctx),
            filter: String::new(),
        }
    }
}

/// 본문 안쪽 여백 (제목 띠·버튼 띠와 같은 좌우 여백).
const PAD: i8 = 16;
/// 버튼 최소 크기 (Notepad++ 대화상자 버튼과 비슷하게).
const BUTTON_MIN: egui::Vec2 = egui::vec2(88.0, 28.0);

/// 대화상자를 그린다. 닫히면 `dialog`를 비우고 결과를 돌려준다.
///
/// 모든 대화상자는 같은 모양이다 (ui-spec §3 공통 모양): ① 제목 띠(제목 + `×`) ② 본문 ③ 버튼 띠(오른쪽 정렬).
/// `×`와 `Esc`·바깥 클릭은 "취소"와 같다.
pub fn show(
    ctx: &egui::Context,
    dialog: &mut Option<Dialog>,
    tabs: &Tabs,
) -> Option<DialogOutcome> {
    let current = dialog.as_mut()?;
    let mut outcome = None;
    let mut open_binary = false;
    let mut close_button = false;

    let style = ctx.global_style();
    let frame = egui::Frame::window(&style)
        .inner_margin(0)
        .corner_radius(8.0);
    let modal = egui::Modal::new(current.area_id())
        .frame(frame)
        .show(ctx, |ui| {
            let palette = Palette::from_visuals(ui.visuals());
            ui.spacing_mut().item_spacing.y = 6.0;
            match current {
                Dialog::Message { title, body } => {
                    ui.set_width(440.0);
                    close_button = title_bar(ui, &palette, title);
                    body_area(ui, |ui| {
                        ui.label(body.as_str());
                    });
                    footer(ui, &palette, |ui| {
                        if default_button(ui, "확인").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                    });
                }
                Dialog::ConfirmBinary(loaded) => {
                    ui.set_width(440.0);
                    let name = loaded.path.file_name().map_or_else(
                        || loaded.path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    close_button = title_bar(ui, &palette, "텍스트 파일이 아닌 것 같습니다");
                    body_area(ui, |ui| {
                        ui.label(format!(
                            "\"{name}\"에 텍스트가 아닌 데이터가 있습니다. 열면 내용이 깨져 보이고, \
                             저장하면 파일이 손상될 수 있습니다."
                        ));
                    });
                    footer(ui, &palette, |ui| {
                        // 실수로 바이너리를 열지 않도록 Enter 기본 버튼은 "취소"
                        if default_button(ui, "취소").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                        if button(ui, "읽기 전용으로 열기").clicked() {
                            open_binary = true;
                        }
                    });
                }
                Dialog::Unsaved {
                    tab_id,
                    title,
                    remaining,
                    deleted,
                } => {
                    ui.set_width(440.0);
                    close_button = title_bar(ui, &palette, "변경 내용을 저장할까요?");
                    body_area(ui, |ui| {
                        if *deleted {
                            ui.label(format!(
                                "\"{title}\" 파일이 디스크에서 삭제되었습니다. 저장하지 않고 닫으면 내용이 사라집니다."
                            ));
                        } else {
                            ui.label(format!("\"{title}\"의 변경 내용이 저장되지 않았습니다."));
                        }
                        if *remaining > 0 {
                            ui.label(
                                RichText::new(format!(
                                    "이어서 닫을 탭 {remaining}개 — 취소하면 남은 탭은 닫지 않습니다."
                                ))
                                .small()
                                .weak(),
                            );
                        }
                    });
                    footer(ui, &palette, |ui| {
                        if button(ui, "취소").clicked() {
                            outcome = Some(DialogOutcome::CancelClose);
                        }
                        if button(ui, "저장 안 함").clicked() {
                            outcome = Some(DialogOutcome::Discard(*tab_id));
                        }
                        // 실수로 변경 내용을 잃지 않도록 Enter 기본 버튼은 "저장"
                        if default_button(ui, "저장").clicked() {
                            outcome = Some(DialogOutcome::Save(*tab_id));
                        }
                    });
                }
                Dialog::WindowList { checked } => {
                    ui.set_width(640.0);
                    close_button = title_bar(ui, &palette, "창 목록");
                    outcome = window_list(ui, &palette, checked, tabs);
                }
                Dialog::Unmappable {
                    tab_id,
                    target,
                    encoding,
                    chars,
                } => {
                    ui.set_width(480.0);
                    close_button = title_bar(
                        ui,
                        &palette,
                        &format!("{}로 저장할 수 없는 문자가 있습니다", short_name(*encoding)),
                    );
                    body_area(ui, |ui| {
                        ui.label(format!(
                            "아래 문자는 {}에 없습니다. 그대로 저장하면 `?`로 바뀝니다.",
                            encoding.label()
                        ));
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            egui::Grid::new("unmappable")
                                .striped(true)
                                .num_columns(2)
                                .spacing([16.0, 4.0])
                                .show(ui, |ui| {
                                    ui.strong("문자");
                                    ui.strong("위치");
                                    ui.end_row();
                                    for c in chars.iter().take(UNMAPPABLE_ROWS) {
                                        ui.label(format!("{} (U+{:04X})", c.ch, u32::from(c.ch)));
                                        ui.label(format!("Ln {}, Col {}", c.line, c.column));
                                        ui.end_row();
                                    }
                                });
                            if chars.len() > UNMAPPABLE_ROWS {
                                ui.label(
                                    RichText::new(format!("외 {}개", chars.len() - UNMAPPABLE_ROWS))
                                        .weak(),
                                );
                            }
                        });
                    });
                    footer(ui, &palette, |ui| {
                        if button(ui, "취소").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                        if button(ui, "`?`로 바꿔 저장").clicked() {
                            outcome = Some(DialogOutcome::SaveReplacing {
                                tab_id: *tab_id,
                                target: target.clone(),
                            });
                        }
                        // 문자를 잃지 않는 쪽이 기본 버튼
                        if default_button(ui, "UTF-8로 저장").clicked() {
                            outcome = Some(DialogOutcome::SaveAsUtf8 {
                                tab_id: *tab_id,
                                target: target.clone(),
                            });
                        }
                    });
                }
                Dialog::DamagedSave { tab_id, target } => {
                    ui.set_width(460.0);
                    close_button = title_bar(ui, &palette, "손상된 문서를 저장하려고 합니다");
                    body_area(ui, |ui| {
                        ui.label(
                            "이 문서는 열 때 해석하지 못한 바이트(�)가 있습니다. \
                             지금 저장하면 원본 바이트가 영구히 사라집니다.",
                        );
                        ui.label(
                            RichText::new("인코딩 → '다시 읽기'로 올바른 인코딩을 먼저 시도해 보세요.")
                                .weak(),
                        );
                    });
                    footer(ui, &palette, |ui| {
                        if button(ui, "그래도 저장").clicked() {
                            outcome = Some(DialogOutcome::SaveDamaged {
                                tab_id: *tab_id,
                                target: target.clone(),
                            });
                        }
                        if default_button(ui, "취소").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                    });
                }
                Dialog::ConfirmReopen {
                    tab_id,
                    title,
                    encoding,
                } => {
                    ui.set_width(460.0);
                    close_button = title_bar(ui, &palette, "변경 내용을 버리고 다시 읽을까요?");
                    body_area(ui, |ui| {
                        ui.label(format!(
                            "\"{title}\"을(를) {}(으)로 다시 읽으면 저장하지 않은 변경 내용이 사라집니다.",
                            encoding.label()
                        ));
                    });
                    footer(ui, &palette, |ui| {
                        if button(ui, "다시 읽기").clicked() {
                            outcome = Some(DialogOutcome::Reopen {
                                tab_id: *tab_id,
                                encoding: *encoding,
                            });
                        }
                        if default_button(ui, "취소").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                    });
                }
                Dialog::ConfirmEditLarge {
                    tab_id,
                    title,
                    size_bytes,
                    limit_bytes,
                } => {
                    ui.set_width(480.0);
                    close_button = title_bar(ui, &palette, "큰 파일을 편집 모드로 열까요?");
                    body_area(ui, |ui| {
                        ui.label(format!(
                            "\"{title}\" ({})는 권장 편집 크기({})를 넘습니다. 편집 모드로 열면 입력과 화면 반응이 \
                             느려지거나 메모리를 많이 쓸 수 있습니다.",
                            viewer::format_size(*size_bytes),
                            viewer::format_size(*limit_bytes)
                        ));
                    });
                    footer(ui, &palette, |ui| {
                        // 실수로 무거운 편집 위젯을 열지 않도록 Enter 기본 버튼은 "읽기 전용 유지"
                        if default_button(ui, "읽기 전용 유지").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                        if button(ui, "편집 모드로 열기").clicked() {
                            outcome = Some(DialogOutcome::OpenEditMode(*tab_id));
                        }
                    });
                }
                Dialog::ExternalChange {
                    tab_id,
                    title,
                    conflict,
                    disk,
                } => {
                    ui.set_width(480.0);
                    close_button =
                        title_bar(ui, &palette, "파일이 다른 프로그램에서 변경되었습니다");
                    body_area(ui, |ui| {
                        if *conflict {
                            ui.label(format!(
                                "\"{title}\"이(가) 디스크에서 변경되었고, 앱에서도 수정한 내용이 있습니다."
                            ));
                            ui.label(
                                RichText::new(
                                    "디스크 버전을 불러오면 앱에서 수정한 내용은 사라집니다.",
                                )
                                .color(palette.status_warning),
                            );
                        } else {
                            ui.label(format!(
                                "\"{title}\"이(가) 디스크에서 변경되었습니다. 다시 불러올까요?"
                            ));
                        }
                    });
                    footer(ui, &palette, |ui| {
                        let keep = DialogOutcome::KeepMine {
                            tab_id: *tab_id,
                            disk: *disk,
                        };
                        if *conflict {
                            // 앱에서 수정한 내용을 실수로 잃지 않도록 Enter 기본 버튼은 "내 버전 유지"
                            if default_button(ui, "내 버전 유지").clicked() {
                                outcome = Some(keep);
                            }
                            if button(ui, "디스크 버전 불러오기").clicked() {
                                outcome = Some(DialogOutcome::ReloadFromDisk(*tab_id));
                            }
                        } else {
                            if button(ui, "내 버전 유지").clicked() {
                                outcome = Some(keep);
                            }
                            if default_button(ui, "다시 불러오기").clicked() {
                                outcome = Some(DialogOutcome::ReloadFromDisk(*tab_id));
                            }
                        }
                    });
                }
                Dialog::GoToLine {
                    tab_id,
                    line_count,
                    current_line,
                    input,
                    error,
                } => {
                    ui.set_width(360.0);
                    close_button = title_bar(ui, &palette, "줄 이동");
                    let mut submit = false;
                    body_area(ui, |ui| {
                        ui.label(format!("줄 번호 (1 – {line_count})"));
                        let edit = ui.add(
                            egui::TextEdit::singleline(input)
                                .hint_text(format!("현재 {current_line}번째 줄"))
                                .desired_width(f32::INFINITY),
                        );
                        // Enter로 입력란이 포커스를 놓은 프레임에는 포커스를 다시 가져오지 않는다
                        // (가져오면 `lost_focus`가 거짓이 되어 Enter가 무시된다). 오류면 다음 프레임에 돌아온다.
                        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit = true;
                        } else {
                            focus_if_outside(ui, &edit);
                        }
                        if let Some(message) = error {
                            ui.label(RichText::new(message.as_str()).color(palette.status_error));
                        }
                    });
                    footer(ui, &palette, |ui| {
                        if button(ui, "취소").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                        if strong_button(ui, "이동").clicked() {
                            submit = true;
                        }
                    });
                    if submit {
                        match parse_line_number(input, *line_count) {
                            Ok(line) => {
                                outcome = Some(DialogOutcome::GoToLine {
                                    tab_id: *tab_id,
                                    line,
                                });
                            }
                            Err(message) => *error = Some(message),
                        }
                    }
                }
                Dialog::Shortcuts { sections, filter } => {
                    ui.set_width(560.0);
                    close_button = title_bar(ui, &palette, "단축키 목록");
                    body_area(ui, |ui| shortcuts_body(ui, sections, filter));
                    footer(ui, &palette, |ui| {
                        if strong_button(ui, "닫기").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                    });
                }
                Dialog::RegexHelp => {
                    ui.set_width(560.0);
                    close_button = title_bar(ui, &palette, "정규식 문법");
                    body_area(ui, regex_help_body);
                    footer(ui, &palette, |ui| {
                        if default_button(ui, "닫기").clicked() {
                            outcome = Some(DialogOutcome::Dismissed);
                        }
                    });
                }
                Dialog::About { copied } => {
                    ui.set_width(440.0);
                    close_button =
                        title_bar(ui, &palette, &format!("{} 정보", branding::APP_NAME));
                    body_area(ui, about_body);
                    let mut dismiss = false;
                    footer_with(
                        ui,
                        &palette,
                        |ui| {
                            let label = if *copied {
                                "복사했습니다"
                            } else {
                                "버전 정보 복사"
                            };
                            if button(ui, label)
                                .on_hover_text("버그를 알릴 때 붙여 넣을 한 줄 (제품명·버전·OS)")
                                .clicked()
                            {
                                ui.ctx().copy_text(help::version_line());
                                *copied = true;
                            }
                        },
                        |ui| dismiss = default_button(ui, "닫기").clicked(),
                    );
                    if dismiss {
                        outcome = Some(DialogOutcome::Dismissed);
                    }
                }
                Dialog::EditorFont(choice) => {
                    ui.set_width(520.0);
                    close_button = title_bar(ui, &palette, "글꼴");
                    body_area(ui, |ui| font_body(ui, &palette, choice));
                    let mut ok = false;
                    footer_with(
                        ui,
                        &palette,
                        |ui| {
                            if button(ui, "기본값으로")
                                .on_hover_text(format!(
                                    "{} · {}",
                                    fonts::DEFAULT_MONO_NAME,
                                    DEFAULT_EDITOR_FONT_SIZE
                                ))
                                .clicked()
                            {
                                choice.select(None);
                                choice.size = DEFAULT_EDITOR_FONT_SIZE;
                            }
                        },
                        |ui| {
                            if button(ui, "취소").clicked() {
                                outcome = Some(DialogOutcome::Dismissed);
                            }
                            ok = default_button(ui, "확인").clicked();
                        },
                    );
                    if ok {
                        outcome = Some(DialogOutcome::SetEditorFont {
                            font: choice.selected.clone(),
                            size: choice.size,
                        });
                    }
                }
            }
        });

    // `×`·바깥 클릭·Esc: 미저장 확인에서는 취소, 그 밖에는 그냥 닫기
    if outcome.is_none() && !open_binary && (close_button || modal.should_close()) {
        outcome = Some(match current {
            Dialog::Unsaved { .. } => DialogOutcome::CancelClose,
            // 외부 변경 확인을 그냥 닫으면 지금 내용 유지 (같은 디스크 버전은 다시 묻지 않음)
            Dialog::ExternalChange { tab_id, disk, .. } => DialogOutcome::KeepMine {
                tab_id: *tab_id,
                disk: *disk,
            },
            _ => DialogOutcome::Dismissed,
        });
    }

    // 바이너리 "읽기 전용으로 열기"는 대화상자가 가진 파일을 넘겨야 하므로 꺼내서 돌려준다.
    if open_binary && let Some(Dialog::ConfirmBinary(loaded)) = dialog.take() {
        return Some(DialogOutcome::OpenBinary(loaded));
    }
    if outcome.is_some() {
        *dialog = None;
    }
    outcome
}

/// 제목에 쓰는 짧은 인코딩 이름 (`EUC-KR (CP949)` → `EUC-KR`).
fn short_name(encoding: TextEncoding) -> &'static str {
    match encoding {
        TextEncoding::EucKr => "EUC-KR",
        other => other.label(),
    }
}

/// ① 제목 띠: 왼쪽 제목, 오른쪽 `×`. `×`를 눌렀으면 `true`.
fn title_bar(ui: &mut egui::Ui, palette: &Palette, title: &str) -> bool {
    let mut close = false;
    egui::Frame::new()
        .fill(palette.dialog_title_bg)
        .corner_radius(egui::CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        })
        .inner_margin(egui::Margin {
            left: PAD,
            right: 8,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(15.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let x = ui
                        .add(
                            egui::Button::new(RichText::new("✕").size(14.0))
                                .frame(false)
                                .min_size(egui::vec2(28.0, 24.0)),
                        )
                        .on_hover_text("닫기 (Esc)");
                    close = x.clicked();
                });
            });
        });
    close
}

/// ② 본문: 좌우·위아래 여백을 둔 영역.
fn body_area(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: PAD,
            right: PAD,
            top: 14,
            bottom: 18,
        })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui);
        });
}

/// ③ 버튼 띠: 색이 다른 바닥 영역에 버튼을 오른쪽 정렬로 둔다 (먼저 추가한 버튼이 가장 오른쪽).
fn footer(ui: &mut egui::Ui, palette: &Palette, add: impl FnOnce(&mut egui::Ui)) {
    footer_with(ui, palette, |_| {}, add);
}

/// 버튼 띠 (왼쪽 버튼 묶음 + 오른쪽 정렬 버튼 묶음).
fn footer_with(
    ui: &mut egui::Ui,
    palette: &Palette,
    left: impl FnOnce(&mut egui::Ui),
    right: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(palette.dialog_footer_bg)
        .corner_radius(egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 8,
            se: 8,
        })
        .inner_margin(egui::Margin::symmetric(PAD, 12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                left(ui);
                // `with_layout`은 남은 높이를 다 차지하므로 버튼 한 줄 높이만 할당한다.
                let size = egui::vec2(ui.available_width(), BUTTON_MIN.y);
                ui.allocate_ui_with_layout(
                    size,
                    egui::Layout::right_to_left(egui::Align::Center),
                    right,
                );
            });
        });
}

/// 테두리 버튼 (최소 크기 고정).
fn button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(label).min_size(BUTTON_MIN))
}

/// 굵게 + 강조 테두리 버튼 (주 동작 표시, 포커스는 가져오지 않음).
fn strong_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let stroke = egui::Stroke::new(1.0, ui.visuals().strong_text_color());
    ui.add(
        egui::Button::new(RichText::new(label).strong())
            .min_size(BUTTON_MIN)
            .stroke(stroke),
    )
}

/// Enter로 눌리는 기본 버튼 (굵게 + 강조 테두리 + 키보드 포커스).
fn default_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let response = strong_button(ui, label);
    focus_if_outside(ui, &response);
    response
}

/// 포커스가 대화상자 밖(뒤의 에디터 등)에 있거나 없으면 `response`로 가져온다.
/// 대화상자 안에서 `Tab`으로 다른 위젯에 옮긴 포커스는 빼앗지 않는다.
fn focus_if_outside(ui: &egui::Ui, response: &egui::Response) {
    let ctx = ui.ctx();
    let focus_in_dialog = ctx
        .memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|r| r.layer_id == ui.layer_id());
    if !focus_in_dialog {
        response.request_focus();
    }
}

/// 도움말 창 본문 스크롤 영역 높이 (작은 노트북 화면에서도 버튼 띠가 보이게).
const HELP_SCROLL_HEIGHT: f32 = 420.0;
/// 도움말 표 왼쪽 열 폭 (묶음이 달라도 열이 맞게).
const HELP_FIRST_COL: f32 = 230.0;

/// 단축키 목록: 검색란 + 메뉴별 표 (기능 · 단축키).
fn shortcuts_body(ui: &mut egui::Ui, sections: &[ShortcutSection], filter: &mut String) {
    let edit = ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("기능 이름이나 키로 찾기 (예: 탭, F3)")
            .desired_width(f32::INFINITY),
    );
    // 열자마자 입력하면 바로 걸러지게 검색란에 포커스 (닫기는 Esc)
    focus_if_outside(ui, &edit);
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .max_height(HELP_SCROLL_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let mut shown = 0;
            for section in sections {
                let rows: Vec<_> = section.rows.iter().filter(|r| r.matches(filter)).collect();
                if rows.is_empty() {
                    continue;
                }
                shown += rows.len();
                help_heading(ui, section.title);
                egui::Grid::new(("shortcuts", section.title))
                    .striped(true)
                    .num_columns(2)
                    .min_col_width(HELP_FIRST_COL)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for row in rows {
                            ui.label(row.action.as_str());
                            ui.label(RichText::new(row.keys.as_str()).monospace());
                            ui.end_row();
                        }
                    });
            }
            if shown == 0 {
                ui.add_space(8.0);
                ui.label(RichText::new("일치하는 단축키가 없습니다").weak());
            }
        });
}

/// 정규식 문법: 안내 + 묶음별 표 (문법 · 설명).
fn regex_help_body(ui: &mut egui::Ui) {
    ui.label("찾기 바에서 `.*`를 켜면 정규식으로 찾습니다. `Aa`를 끄면 대소문자를 무시합니다.");
    ui.label(
        RichText::new("^와 $는 줄 단위로 동작합니다. 문법은 Rust regex를 따릅니다.")
            .small()
            .weak(),
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .max_height(HELP_SCROLL_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (title, rows) in help::REGEX_SECTIONS {
                help_heading(ui, title);
                egui::Grid::new(("regex-help", *title))
                    .striped(true)
                    .num_columns(2)
                    .min_col_width(HELP_FIRST_COL - 60.0)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (syntax, meaning) in *rows {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                for part in syntax.split("  ") {
                                    ui.label(RichText::new(part).code());
                                }
                            });
                            ui.label(*meaning);
                            ui.end_row();
                        }
                    });
            }
        });
}

/// 정보: 아이콘 · 제품명 · 버전 · 설명 / 플랫폼 · 설정 파일 위치.
fn about_body(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if let Some(texture) = about_icon(ui.ctx()) {
            ui.add(egui::Image::new(&texture).fit_to_exact_size(egui::vec2(72.0, 72.0)));
            ui.add_space(8.0);
        }
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(branding::APP_NAME).size(20.0).strong());
            ui.label(format!("버전 {}", help::VERSION));
            ui.label(RichText::new(env!("CARGO_PKG_DESCRIPTION")).weak());
        });
    });
    ui.add_space(8.0);
    ui.separator();
    egui::Grid::new("about")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            ui.label(RichText::new("플랫폼").weak());
            ui.label(help::platform());
            ui.end_row();
            ui.label(RichText::new("설정 파일").weak());
            let path = Settings::default_path()
                .map_or_else(|| "(알 수 없음)".to_owned(), |p| p.display().to_string());
            ui.add(egui::Label::new(RichText::new(path).small()).wrap());
            ui.end_row();
        });
}

/// 정보 창 아이콘 텍스처 (처음 열 때 한 번 만들어 둔다).
fn about_icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::unique("about-icon");
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let icon = branding::window_icon()?;
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    );
    let texture = ctx.load_texture("about-icon", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

/// 글꼴 대화상자에서 고를 수 있는 크기 (목록 밖 크기는 위 입력란으로).
const FONT_SIZES: [f32; 15] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0, 32.0,
];
/// 글꼴·크기 목록 높이.
const FONT_LIST_HEIGHT: f32 = 220.0;
/// 미리보기 글.
const FONT_SAMPLE: &str = "The quick brown fox jumps over the lazy dog.\n0O 1lI {}[]() => != ;\n다람쥐 헌 쳇바퀴에 타고파. 한글 가나다 ABC";

/// 글꼴: [글꼴 목록 | 크기 목록] / 미리보기 / 안내.
fn font_body(ui: &mut egui::Ui, palette: &Palette, choice: &mut FontChoice) {
    if let Some(rx) = &choice.loading {
        match rx.try_recv() {
            Ok(list) => {
                // 지금 글꼴이 고정폭이 아니면 목록에 보이게 거르기를 끈다
                if let Some(current) = &choice.selected
                    && list.iter().any(|f| f.name == current.name && !f.monospace)
                {
                    choice.monospace_only = false;
                }
                choice.fonts = Some(list);
                choice.loading = None;
            }
            Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
            Err(mpsc::TryRecvError::Disconnected) => {
                choice.fonts = Some(Vec::new());
                choice.loading = None;
            }
        }
    }

    let mut picked: Option<Option<EditorFont>> = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        // 글꼴
        ui.vertical(|ui| {
            ui.set_width(320.0);
            ui.label("글꼴");
            ui.add(
                egui::TextEdit::singleline(&mut choice.filter)
                    .hint_text("글꼴 이름으로 찾기")
                    .desired_width(f32::INFINITY),
            );
            egui::Frame::new()
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .inner_margin(2)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("font-list")
                        .max_height(FONT_LIST_HEIGHT)
                        .min_scrolled_height(FONT_LIST_HEIGHT)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let default_label = format!("기본 ({})", fonts::DEFAULT_MONO_NAME);
                            if ui
                                .selectable_label(choice.selected.is_none(), default_label)
                                .clicked()
                            {
                                picked = Some(None);
                            }
                            let Some(list) = &choice.fonts else {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("글꼴 목록을 읽는 중…").weak());
                                });
                                return;
                            };
                            let filter = choice.filter.trim().to_lowercase();
                            let reveal = std::mem::take(&mut choice.reveal_selected);
                            for font in list.iter().filter(|f| {
                                (!choice.monospace_only || f.monospace)
                                    && (filter.is_empty()
                                        || f.name.to_lowercase().contains(&filter))
                            }) {
                                let selected = choice
                                    .selected
                                    .as_ref()
                                    .is_some_and(|s| s.name == font.name);
                                let row = ui.selectable_label(selected, font.name.as_str());
                                if selected && reveal {
                                    row.scroll_to_me(Some(egui::Align::Center));
                                }
                                if row.clicked() {
                                    picked = Some(Some(EditorFont {
                                        name: font.name.clone(),
                                        path: font.path.clone(),
                                        face: font.face,
                                    }));
                                }
                            }
                        });
                });
            ui.checkbox(&mut choice.monospace_only, "고정폭 글꼴만")
                .on_hover_text("글자 폭이 모두 같은 글꼴 (코드·표 맞춤에 좋음)");
        });
        // 크기
        ui.vertical(|ui| {
            ui.label("크기");
            ui.add(
                egui::DragValue::new(&mut choice.size)
                    .range(EDITOR_FONT_SIZE_RANGE)
                    .speed(0.2)
                    .fixed_decimals(0),
            );
            egui::Frame::new()
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .inner_margin(2)
                .show(ui, |ui| {
                    ui.set_width(80.0);
                    egui::ScrollArea::vertical()
                        .id_salt("font-size-list")
                        .max_height(FONT_LIST_HEIGHT)
                        .min_scrolled_height(FONT_LIST_HEIGHT)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for size in FONT_SIZES {
                                let selected = (choice.size - size).abs() < 0.01;
                                if ui
                                    .selectable_label(selected, format!("{size:.0}"))
                                    .clicked()
                                {
                                    choice.size = size;
                                }
                            }
                        });
                });
        });
    });
    if let Some(font) = picked {
        choice.select(font);
    }

    ui.add_space(8.0);
    ui.label("미리보기");
    egui::Frame::new()
        .fill(palette.dialog_footer_bg)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height(90.0);
            let font = egui::FontId::new(
                choice.size,
                egui::FontFamily::Name(fonts::FONT_PREVIEW_FAMILY.into()),
            );
            // 미리보기 패밀리는 앱이 다음 프레임에 등록한다 — 그전에는 기본 고정폭으로
            let font = if ui.fonts(|f| f.families().contains(&font.family)) {
                font
            } else {
                egui::FontId::monospace(choice.size)
            };
            egui::ScrollArea::horizontal()
                .id_salt("font-sample")
                .show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(FONT_SAMPLE).font(font)).extend());
                });
        });
    ui.label(
        RichText::new(
            "선택한 글꼴에 없는 글자(한글 등)는 기본 글꼴로 표시합니다. \
             확대/축소(Ctrl+= / Ctrl+-)는 이 크기에 곱해집니다.",
        )
        .small()
        .weak(),
    );
}

/// 도움말 표의 묶음 제목.
fn help_heading(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong());
    ui.add_space(2.0);
}

/// 줄 이동 입력을 1-기반 줄 번호로 읽는다. 범위 밖·숫자가 아니면 안내 문구.
pub fn parse_line_number(input: &str, line_count: usize) -> Result<usize, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("줄 번호를 입력하세요".to_owned());
    }
    let line: usize = input.parse().map_err(|_| "숫자로 입력하세요".to_owned())?;
    if (1..=line_count.max(1)).contains(&line) {
        Ok(line)
    } else {
        Err(format!(
            "1부터 {} 사이의 줄 번호를 입력하세요",
            line_count.max(1)
        ))
    }
}

/// 창 목록 대화상자 (ui-spec §7): 체크박스 · 아이콘 · 이름(현재 창 굵게) · 수정 여부 · 경로 /
/// 전체 선택 / `이동`(하나 체크 시) · `선택한 창 닫기 (N)` · `모든 창 닫기` · `닫기`, 줄 더블클릭으로 이동.
fn window_list(
    ui: &mut egui::Ui,
    palette: &Palette,
    checked: &mut HashSet<u64>,
    tabs: &Tabs,
) -> Option<DialogOutcome> {
    let mut outcome = None;

    body_area(ui, |ui| {
        ui.label(
            RichText::new(
                "체크해서 여러 창을 한꺼번에 닫을 수 있습니다. 줄을 더블클릭하면 그 창으로 이동합니다.",
            )
            .small()
            .weak(),
        );

        // 닫힌 탭의 체크는 버린다
        checked.retain(|id| tabs.index_of(*id).is_some());
        let mut all = checked.len() == tabs.len();
        if ui.checkbox(&mut all, "전체 선택").changed() {
            if all {
                checked.extend(tabs.iter().map(|t| t.id()));
            } else {
                checked.clear();
            }
        }
        ui.separator();

        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                egui::Grid::new("window-list")
                    .striped(true)
                    .num_columns(4)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        for (i, tab) in tabs.iter().enumerate() {
                            let id = tab.id();
                            let mut on = checked.contains(&id);
                            if ui.checkbox(&mut on, "").changed() {
                                if on {
                                    checked.insert(id);
                                } else {
                                    checked.remove(&id);
                                }
                            }
                            ui.horizontal(|ui| {
                                tab_bar::icon_label(ui, tab_bar::tab_icon(tabs, i));
                                let mut text = RichText::new(tab.doc.title());
                                if i == tabs.active_index() {
                                    text = text.strong();
                                }
                                let row = ui.add(
                                    egui::Label::new(text)
                                        .selectable(false)
                                        .sense(egui::Sense::click()),
                                );
                                if row.double_clicked() {
                                    outcome = Some(DialogOutcome::Activate(id));
                                }
                            });
                            if tab.doc.is_modified() {
                                ui.label(RichText::new("● 수정됨").color(palette.modified_marker));
                            } else {
                                ui.label("");
                            }
                            let path = tab.doc.path().map_or_else(
                                || UNSAVED_PATH_HINT.to_owned(),
                                |p| p.display().to_string(),
                            );
                            ui.label(RichText::new(path).small().weak());
                            ui.end_row();
                        }
                    });
            });
    });

    // 탭 순서대로 닫도록 목록 순서로 정렬
    let selected: Vec<u64> = tabs
        .iter()
        .map(|t| t.id())
        .filter(|id| checked.contains(id))
        .collect();
    let mut dismiss = false;
    footer_with(
        ui,
        palette,
        |ui| {
            if ui
                .add_enabled(
                    selected.len() == 1,
                    egui::Button::new("이동").min_size(BUTTON_MIN),
                )
                .on_hover_text("체크한 창 하나로 이동")
                .clicked()
            {
                outcome = selected.first().copied().map(DialogOutcome::Activate);
            }
            if ui
                .add_enabled(
                    !selected.is_empty(),
                    egui::Button::new(format!("선택한 창 닫기 ({})", selected.len()))
                        .min_size(BUTTON_MIN),
                )
                .clicked()
            {
                outcome = Some(DialogOutcome::CloseTabs(selected.clone()));
            }
            if button(ui, "모든 창 닫기").clicked() {
                outcome = Some(DialogOutcome::CloseTabs(
                    tabs.iter().map(|t| t.id()).collect(),
                ));
            }
        },
        |ui| {
            dismiss = default_button(ui, "닫기").clicked();
        },
    );
    if dismiss {
        outcome = Some(DialogOutcome::Dismissed);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Key, Modifiers, RawInput};

    fn frame(
        ctx: &egui::Context,
        dialog: &mut Option<Dialog>,
        tabs: &Tabs,
        events: Vec<Event>,
    ) -> Option<DialogOutcome> {
        let raw = RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut outcome = None;
        let mut full = ctx.run_ui(raw, |ui| {
            outcome = show(ui.ctx(), dialog, tabs);
        });
        full.textures_delta.clear();
        outcome
    }

    fn press(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn unsaved(tab_id: u64) -> Option<Dialog> {
        Some(Dialog::Unsaved {
            tab_id,
            title: "회의록.txt".into(),
            remaining: 2,
            deleted: false,
        })
    }

    fn external(conflict: bool) -> Option<Dialog> {
        Some(Dialog::ExternalChange {
            tab_id: 4,
            title: "server.log".into(),
            conflict,
            disk: DiskMetadata {
                modified_time: None,
                byte_len: 10,
            },
        })
    }

    #[test]
    fn external_change_default_reloads_unless_conflict() {
        let tabs = Tabs::new();
        // 수정하지 않은 탭: Enter = 다시 불러오기
        let ctx = egui::Context::default();
        let mut dialog = external(false);
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let out = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Enter)]);
        assert!(
            matches!(out, Some(DialogOutcome::ReloadFromDisk(4))),
            "{out:?}"
        );

        // 충돌: Enter = 내 버전 유지
        let ctx = egui::Context::default();
        let mut dialog = external(true);
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let out = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Enter)]);
        assert!(
            matches!(out, Some(DialogOutcome::KeepMine { tab_id: 4, .. })),
            "{out:?}"
        );

        // Esc = 내 버전 유지 (다시 묻지 않도록 디스크 버전을 넘긴다)
        let ctx = egui::Context::default();
        let mut dialog = external(false);
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let out = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Escape)]);
        assert!(
            matches!(out, Some(DialogOutcome::KeepMine { disk, .. }) if disk.byte_len == 10),
            "{out:?}"
        );
    }

    #[test]
    fn unsaved_enter_saves_and_escape_cancels() {
        let tabs = Tabs::new();

        let ctx = egui::Context::default();
        let mut dialog = unsaved(7);
        assert!(frame(&ctx, &mut dialog, &tabs, vec![]).is_none());
        let outcome = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Enter)]);
        assert!(
            matches!(outcome, Some(DialogOutcome::Save(7))),
            "{outcome:?}"
        );
        assert!(dialog.is_none());

        let ctx = egui::Context::default();
        let mut dialog = unsaved(7);
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let outcome = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Escape)]);
        assert!(
            matches!(outcome, Some(DialogOutcome::CancelClose)),
            "{outcome:?}"
        );
        assert!(dialog.is_none());
    }

    #[test]
    fn unmappable_enter_saves_as_utf8_and_damaged_enter_cancels() {
        let tabs = Tabs::new();
        let ctx = egui::Context::default();
        let mut dialog = Some(Dialog::Unmappable {
            tab_id: 3,
            target: None,
            encoding: TextEncoding::EucKr,
            chars: Vec::new(),
        });
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let outcome = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Enter)]);
        assert!(
            matches!(
                outcome,
                Some(DialogOutcome::SaveAsUtf8 {
                    tab_id: 3,
                    target: None
                })
            ),
            "{outcome:?}"
        );

        let ctx = egui::Context::default();
        let mut dialog = Some(Dialog::DamagedSave {
            tab_id: 3,
            target: None,
        });
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let outcome = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Enter)]);
        assert!(
            matches!(outcome, Some(DialogOutcome::Dismissed)),
            "{outcome:?}"
        );
    }

    /// 대화상자가 내용 크기만큼만 그려지는지. 실제 앱처럼 하나의 화면에서 큰 창 목록을 먼저 띄운 뒤
    /// 다른 대화상자를 띄워도 앞 대화상자 크기를 물려받지 않아야 한다.
    #[test]
    fn dialogs_fit_their_content_after_larger_dialog() {
        let mut tabs = Tabs::new();
        for _ in 0..15 {
            tabs.new_untitled();
        }
        let id = tabs.active().id();
        let ctx = egui::Context::default();
        let dialogs = [
            Dialog::window_list(),
            Dialog::message("제목", "본문"),
            Dialog::ConfirmEditLarge {
                tab_id: id,
                title: "huge.log".into(),
                size_bytes: 85 * 1_048_576,
                limit_bytes: 5 * 1_048_576,
            },
            Dialog::Unsaved {
                tab_id: id,
                title: "cp949.txt".into(),
                remaining: 1,
                deleted: false,
            },
            Dialog::DamagedSave {
                tab_id: id,
                target: None,
            },
            Dialog::ConfirmReopen {
                tab_id: id,
                title: "cp949.txt".into(),
                encoding: TextEncoding::Utf8,
            },
            Dialog::Unmappable {
                tab_id: id,
                target: None,
                encoding: TextEncoding::EucKr,
                chars: Vec::new(),
            },
        ];
        for dialog in dialogs {
            let is_list = matches!(dialog, Dialog::WindowList { .. });
            let name = format!("{dialog:?}");
            let area = dialog.area_id();
            let mut dialog = Some(dialog);
            for _ in 0..3 {
                frame(&ctx, &mut dialog, &tabs, vec![]);
            }
            let rect = ctx.memory(|m| m.area_rect(area)).expect("대화상자 영역");
            if !is_list {
                assert!(rect.height() < 200.0, "{name}: 높이 {}", rect.height());
                assert!(rect.width() < 520.0, "{name}: 폭 {}", rect.width());
            }
        }
    }

    #[test]
    fn go_to_line_validates_and_submits_with_enter() {
        let tabs = Tabs::new();
        let ctx = egui::Context::default();
        let mut dialog = Some(Dialog::go_to_line(5, 120, 1));
        frame(&ctx, &mut dialog, &tabs, vec![]);
        // 범위 밖 → 대화상자 유지 + 오류 안내
        let out = frame(
            &ctx,
            &mut dialog,
            &tabs,
            vec![Event::Text("500".into()), press(Key::Enter)],
        );
        assert!(out.is_none());
        assert!(matches!(
            &dialog,
            Some(Dialog::GoToLine { error: Some(e), .. }) if e.contains("1부터 120")
        ));
        // 입력란에 포커스가 돌아와 고쳐 쓸 수 있다
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let out = frame(
            &ctx,
            &mut dialog,
            &tabs,
            vec![
                press(Key::Backspace),
                press(Key::Backspace),
                press(Key::Backspace),
                Event::Text("42".into()),
                press(Key::Enter),
            ],
        );
        assert!(
            matches!(
                out,
                Some(DialogOutcome::GoToLine {
                    tab_id: 5,
                    line: 42
                })
            ),
            "{out:?}"
        );
        assert!(dialog.is_none());
    }

    #[test]
    fn help_dialogs_close_with_enter_or_escape_and_filter_shortcuts() {
        let tabs = Tabs::new();

        // 정보: Enter = 닫기
        let ctx = egui::Context::default();
        let mut dialog = Some(Dialog::About { copied: false });
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let out = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Enter)]);
        assert!(matches!(out, Some(DialogOutcome::Dismissed)), "{out:?}");

        // 정규식 문법: Esc = 닫기
        let ctx = egui::Context::default();
        let mut dialog = Some(Dialog::RegexHelp);
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let out = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Escape)]);
        assert!(matches!(out, Some(DialogOutcome::Dismissed)), "{out:?}");

        // 단축키 목록: 열자마자 입력하면 검색란에 들어가고, Esc로 닫힌다
        let ctx = crate::ui::help::tests::started_context();
        let mut dialog = Some(Dialog::shortcuts(&Keymap::current(), &ctx));
        frame(&ctx, &mut dialog, &tabs, vec![]);
        frame(&ctx, &mut dialog, &tabs, vec![Event::Text("탭".into())]);
        assert!(matches!(&dialog, Some(Dialog::Shortcuts { filter, .. }) if filter == "탭"));
        let out = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Escape)]);
        assert!(matches!(out, Some(DialogOutcome::Dismissed)), "{out:?}");
    }

    #[test]
    fn font_dialog_enter_applies_choice_and_escape_cancels() {
        let tabs = Tabs::new();
        let current = EditorFont {
            name: "D2Coding".into(),
            path: "D2Coding.ttc".into(),
            face: 0,
        };

        // 열 때 지금 글꼴을 미리보기로 요청하고, Enter = 확인 (고른 글꼴·크기 그대로)
        let ctx = egui::Context::default();
        let mut choice = FontChoice::new(Some(current.clone()), 15.0);
        assert_eq!(choice.preview_request.take(), Some(Some(current.clone())));
        let mut dialog = Some(Dialog::EditorFont(Box::new(choice)));
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let out = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Enter)]);
        assert!(
            matches!(&out, Some(DialogOutcome::SetEditorFont { font: Some(f), size })
                if *f == current && *size == 15.0),
            "{out:?}"
        );

        // 기본값으로 = 기본 글꼴·크기, 미리보기도 기본으로
        let mut choice = FontChoice::new(Some(current), 20.0);
        choice.preview_request = None;
        choice.select(None);
        assert_eq!(choice.preview_request, Some(None));
        choice.select(None);
        assert_eq!(
            choice.preview_request,
            Some(None),
            "같은 선택은 다시 요청하지 않음"
        );

        // Esc = 취소
        let ctx = egui::Context::default();
        let mut dialog = Some(Dialog::EditorFont(Box::new(FontChoice::new(None, 13.0))));
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let out = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Escape)]);
        assert!(matches!(out, Some(DialogOutcome::Dismissed)), "{out:?}");
    }

    #[test]
    fn line_number_parsing() {
        assert_eq!(parse_line_number(" 7 ", 10), Ok(7));
        assert_eq!(parse_line_number("10", 10), Ok(10));
        assert!(parse_line_number("0", 10).is_err());
        assert!(parse_line_number("11", 10).is_err());
        assert!(parse_line_number("abc", 10).unwrap_err().contains("숫자"));
        assert!(parse_line_number("", 10).is_err());
        assert_eq!(parse_line_number("1", 0), Ok(1), "빈 문서도 1번 줄은 있다");
    }

    #[test]
    fn binary_enter_cancels() {
        let tabs = Tabs::new();
        let ctx = egui::Context::default();
        let mut dialog = Some(Dialog::ConfirmBinary(Box::new(LoadedFile {
            path: "image.dat".into(),
            bytes: vec![0, 1, 2],
            metadata: crate::core::document::DiskMetadata {
                modified_time: None,
                byte_len: 3,
            },
            readonly: false,
            open_mode: crate::core::file_io::OpenMode::Edit,
        })));
        frame(&ctx, &mut dialog, &tabs, vec![]);
        let outcome = frame(&ctx, &mut dialog, &tabs, vec![press(Key::Enter)]);
        assert!(
            matches!(outcome, Some(DialogOutcome::Dismissed)),
            "{outcome:?}"
        );
    }
}
