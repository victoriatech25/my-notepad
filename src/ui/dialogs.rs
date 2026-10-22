//! 모달 대화상자 (PLAN.md §3.6, ui-spec 대화상자 문구).
//!
//! - P1-10: 파일 열기·저장 오류 안내, 바이너리 파일 열기 확인
//! - P1-11: 미저장 확인(`저장 / 저장 안 함 / 취소`)과 여러 문서 순차 확인 추가 예정

use eframe::egui::{self, RichText};

use crate::core::file_io::LoadedFile;

/// 화면에 띄울 대화상자.
#[derive(Debug)]
pub enum Dialog {
    /// 확인 버튼만 있는 안내·오류.
    Message { title: String, body: String },
    /// "바이너리 파일입니다. 그래도 여시겠습니까?"
    ConfirmBinary(Box<LoadedFile>),
}

/// 대화상자를 닫으며 사용자가 고른 결과.
#[derive(Debug)]
pub enum DialogOutcome {
    Dismissed,
    OpenBinary(Box<LoadedFile>),
}

impl Dialog {
    pub fn message(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::Message {
            title: title.into(),
            body: body.into(),
        }
    }
}

/// 대화상자를 그린다. 닫히면 `dialog`를 비우고 결과를 돌려준다.
pub fn show(ctx: &egui::Context, dialog: &mut Option<Dialog>) -> Option<DialogOutcome> {
    let current = dialog.as_ref()?;
    let mut confirm = false;
    let mut dismiss = false;

    let modal = egui::Modal::new(egui::Id::unique("app-dialog")).show(ctx, |ui| {
        ui.set_width(420.0);
        match current {
            Dialog::Message { title, body } => {
                ui.heading(title);
                ui.add_space(4.0);
                ui.label(body);
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok = ui.button(RichText::new("확인").strong());
                    if ok.clicked() {
                        dismiss = true;
                    }
                    ok.request_focus();
                });
            }
            Dialog::ConfirmBinary(loaded) => {
                ui.heading("바이너리 파일입니다");
                ui.add_space(4.0);
                ui.label(format!(
                    "\"{}\"은(는) 텍스트 파일이 아닌 것 같습니다. 그래도 여시겠습니까?\n\
                     읽기 전용으로 열리며, 표시할 수 없는 바이트는 �로 보입니다.",
                    loaded.path.file_name().map_or_else(
                        || loaded.path.display().to_string(),
                        |n| { n.to_string_lossy().into_owned() }
                    )
                ));
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 실수로 바이너리를 열지 않도록 Enter 기본 버튼은 "취소"
                    let cancel = ui.button(RichText::new("취소").strong());
                    if cancel.clicked() {
                        dismiss = true;
                    }
                    cancel.request_focus();
                    if ui.button("읽기 전용으로 열기").clicked() {
                        confirm = true;
                    }
                });
            }
        }
    });
    if modal.should_close() {
        dismiss = true;
    }

    if confirm {
        match dialog.take() {
            Some(Dialog::ConfirmBinary(loaded)) => Some(DialogOutcome::OpenBinary(loaded)),
            _ => Some(DialogOutcome::Dismissed),
        }
    } else if dismiss {
        *dialog = None;
        Some(DialogOutcome::Dismissed)
    } else {
        None
    }
}
