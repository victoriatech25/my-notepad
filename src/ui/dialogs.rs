//! 모달 대화상자 (PLAN.md §3.6, ui-spec §3 대화상자 문구, §7 창 목록).
//!
//! - P1-10: 파일 열기·저장 오류 안내, 바이너리 파일 열기 확인
//! - P1-11: 미저장 확인(`저장 / 저장 안 함 / 취소`, 기본 버튼 `저장`, `Esc` = 취소)과
//!   일괄 닫기 시 "이어서 닫을 탭 N개" 안내, 창 목록(체크해서 여러 창 닫기·이동)
//! - P1-12: 표현 불가 문자 경고(`UTF-8로 저장` · `?`로 바꿔 저장 · 취소), 손상된 문서 저장 경고,
//!   수정된 문서를 인코딩으로 다시 읽기 전 확인
//! - UX-3 피드백: 모든 대화상자를 Notepad++ 형태 공통 모양(제목 띠 + `×` / 본문 / 버튼 띠)으로 통일

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui::{self, RichText};

use crate::core::encoding::{TextEncoding, UnmappableChar};
use crate::core::file_io::LoadedFile;
use crate::tabs::Tabs;
use crate::ui::tab_bar::{self, UNSAVED_PATH_HINT};
use crate::ui::theme::Palette;

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
    /// 변경 내용을 버리고 다시 읽기
    Reopen {
        tab_id: u64,
        encoding: TextEncoding,
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
        };
        egui::Id::unique(("app-dialog", kind))
    }

    pub fn message(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::Message {
            title: title.into(),
            body: body.into(),
        }
    }

    pub fn window_list() -> Self {
        Self::WindowList {
            checked: HashSet::new(),
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
                } => {
                    ui.set_width(440.0);
                    close_button = title_bar(ui, &palette, "변경 내용을 저장할까요?");
                    body_area(ui, |ui| {
                        ui.label(format!("\"{title}\"의 변경 내용이 저장되지 않았습니다."));
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
            }
        });

    // `×`·바깥 클릭·Esc: 미저장 확인에서는 취소, 그 밖에는 그냥 닫기
    if outcome.is_none() && !open_binary && (close_button || modal.should_close()) {
        outcome = Some(match current {
            Dialog::Unsaved { .. } => DialogOutcome::CancelClose,
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

/// Enter로 눌리는 기본 버튼 (굵게 + 강조 테두리 + 키보드 포커스).
///
/// 포커스가 대화상자 밖(뒤의 에디터 등)에 있거나 없으면 이 버튼으로 가져온다.
/// 대화상자 안에서 `Tab`으로 다른 버튼에 옮긴 포커스는 빼앗지 않는다.
fn default_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let stroke = egui::Stroke::new(1.0, ui.visuals().strong_text_color());
    let response = ui.add(
        egui::Button::new(RichText::new(label).strong())
            .min_size(BUTTON_MIN)
            .stroke(stroke),
    );
    let ctx = ui.ctx();
    let focus_in_dialog = ctx
        .memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|r| r.layer_id == ui.layer_id());
    if !focus_in_dialog {
        response.request_focus();
    }
    response
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
        })
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
            Dialog::Unsaved {
                tab_id: id,
                title: "cp949.txt".into(),
                remaining: 1,
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
