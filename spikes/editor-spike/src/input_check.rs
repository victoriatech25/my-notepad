//! P0-03~05 입력 검증 화면.
//!
//! - P0-03: 한글 IME 조합 입력 — 입력란 + IME/키/텍스트 이벤트 로그 (실패 시 원인 분석용)
//! - P0-04: 위젯보다 먼저 처리하는 전역 단축키 — 실행된 커맨드 로그, 입력란에 문자가 들어가면 실패
//! - P0-05: 창 닫기 가로채기 — 수정 상태에서 닫으면 취소 후 확인 대화상자

use std::collections::VecDeque;

use eframe::egui::{self, Event, Key, KeyboardShortcut, Modifiers, RichText, ViewportCommand};

const LOG_LIMIT: usize = 300;

const IME_GUIDE: &str = "\
아래 입력란에서 한글 입력을 확인합니다 (체크리스트는 WORKPLAN.md P0-03).
1) '한글 입력 테스트'  2) 조합 중 Enter / Space / 방향키 / 마우스 클릭  3) 조합 중 Backspace
4) 한/영 전환  5) 한자 키  6) 빠르게 긴 문장 입력  7) 조합 중 Ctrl+S 같은 단축키

";

struct Command {
    name: &'static str,
    shortcut: KeyboardShortcut,
}

fn commands() -> Vec<Command> {
    let cmd = Modifiers::COMMAND;
    let mac = cfg!(target_os = "macos");
    // 같은 키에서 Shift 조합을 먼저 검사해야 한다 (consume 순서).
    vec![
        Command {
            name: "Markdown 소스 ↔ 미리보기",
            shortcut: KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::V),
        },
        Command {
            name: "다른 이름으로 저장",
            shortcut: KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::S),
        },
        Command {
            name: "저장",
            shortcut: KeyboardShortcut::new(cmd, Key::S),
        },
        Command {
            name: "열기",
            shortcut: KeyboardShortcut::new(cmd, Key::O),
        },
        Command {
            name: "탭 닫기",
            shortcut: KeyboardShortcut::new(cmd, Key::W),
        },
        Command {
            name: "바꾸기",
            shortcut: if mac {
                KeyboardShortcut::new(cmd | Modifiers::ALT, Key::F)
            } else {
                KeyboardShortcut::new(cmd, Key::H)
            },
        },
        Command {
            name: "찾기",
            shortcut: KeyboardShortcut::new(cmd, Key::F),
        },
        Command {
            name: "이전 찾기",
            shortcut: if mac {
                KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::G)
            } else {
                KeyboardShortcut::new(Modifiers::SHIFT, Key::F3)
            },
        },
        Command {
            name: "다음 찾기",
            shortcut: if mac {
                KeyboardShortcut::new(cmd, Key::G)
            } else {
                KeyboardShortcut::new(Modifiers::NONE, Key::F3)
            },
        },
        Command {
            name: "사이드바",
            shortcut: KeyboardShortcut::new(cmd, Key::B),
        },
    ]
}

pub struct InputCheck {
    text: String,
    modified: bool,
    commands: Vec<Command>,
    chord_k_pending: bool,
    command_log: VecDeque<String>,
    event_log: VecDeque<String>,
    ime_only: bool,
    frame: u64,
    show_close_dialog: bool,
    allow_close: bool,
}

impl Default for InputCheck {
    fn default() -> Self {
        Self {
            text: IME_GUIDE.to_owned(),
            modified: false,
            commands: commands(),
            chord_k_pending: false,
            command_log: VecDeque::new(),
            event_log: VecDeque::new(),
            ime_only: false,
            frame: 0,
            show_close_dialog: false,
            allow_close: false,
        }
    }
}

fn push(log: &mut VecDeque<String>, line: String) {
    log.push_front(line);
    log.truncate(LOG_LIMIT);
}

impl InputCheck {
    /// 위젯보다 먼저 호출: 이벤트를 기록하고, 앱 단축키를 소비한다.
    pub fn before_widgets(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        let frame = self.frame;

        // 1) 이번 프레임의 입력 이벤트 기록 (소비 전 원본)
        let events = ctx.input(|i| i.events.clone());
        for event in &events {
            let line = match event {
                Event::Ime(ime) => Some(format!("#{frame:<6} IME  {ime:?}")),
                Event::Text(text) if !self.ime_only => Some(format!("#{frame:<6} TEXT {text:?}")),
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    repeat,
                    ..
                } if !self.ime_only => Some(format!(
                    "#{frame:<6} KEY  {key:?}{}{}",
                    fmt_modifiers(*modifiers),
                    if *repeat { " (repeat)" } else { "" }
                )),
                _ => None,
            };
            if let Some(line) = line {
                push(&mut self.event_log, line);
            }
        }

        // 2) Ctrl/Cmd+K 다음 V 코드
        if self.chord_k_pending {
            let (v, other) = ctx.input_mut(|i| {
                let v = i.consume_key(Modifiers::NONE, Key::V);
                if v {
                    // 키 이벤트와 함께 오는 문자 입력도 지워야 입력란에 'v'가 들어가지 않는다.
                    i.events
                        .retain(|e| !matches!(e, Event::Text(t) if t.eq_ignore_ascii_case("v")));
                }
                let other = i
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Key { pressed: true, .. }));
                (v, other)
            });
            if v {
                self.chord_k_pending = false;
                push(
                    &mut self.command_log,
                    format!(
                        "#{frame:<6} ▶ Markdown 분할 보기 ({} 다음 V)",
                        ctx.format_shortcut(&chord_k())
                    ),
                );
            } else if other {
                self.chord_k_pending = false;
                push(&mut self.command_log, format!("#{frame:<6}   (코드 취소)"));
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&chord_k())) {
            self.chord_k_pending = true;
            push(
                &mut self.command_log,
                format!(
                    "#{frame:<6}   {} … 다음 키 대기",
                    ctx.format_shortcut(&chord_k())
                ),
            );
        }

        // 3) 일반 단축키
        for command in &self.commands {
            if ctx.input_mut(|i| i.consume_shortcut(&command.shortcut)) {
                push(
                    &mut self.command_log,
                    format!(
                        "#{frame:<6} ▶ {} ({})",
                        command.name,
                        ctx.format_shortcut(&command.shortcut)
                    ),
                );
            }
        }
    }

    pub fn screen(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("logs")
            .resizable(true)
            .default_size(460.0)
            .show(ui, |ui| {
                ui.heading("실행된 커맨드 (P0-04)");
                ui.label(
                    RichText::new("단축키가 여기 찍히고 입력란에는 문자가 들어가지 않아야 합니다.")
                        .small()
                        .weak(),
                );
                egui::ScrollArea::vertical()
                    .id_salt("command-log")
                    .max_height(180.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for line in &self.command_log {
                            ui.label(RichText::new(line).monospace().small());
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("입력 이벤트 (P0-03)");
                    ui.checkbox(&mut self.ime_only, "IME만");
                    if ui.small_button("지우기").clicked() {
                        self.event_log.clear();
                        self.command_log.clear();
                    }
                    if ui
                        .small_button("복사")
                        .on_hover_text("로그를 클립보드로 복사")
                        .clicked()
                    {
                        let text: Vec<&str> =
                            self.event_log.iter().rev().map(String::as_str).collect();
                        ui.ctx().copy_text(text.join("\n"));
                    }
                });
                ui.label(
                    RichText::new(
                        "문제가 생기면 '복사'로 로그를 붙여 회신해 주세요. 위가 최신입니다.",
                    )
                    .small()
                    .weak(),
                );
                egui::ScrollArea::vertical()
                    .id_salt("event-log")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.event_log {
                            ui.label(RichText::new(line).monospace().small());
                        }
                    });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("한글 입력란");
            let output = egui::TextEdit::multiline(&mut self.text)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .desired_rows(24)
                .lock_focus(true)
                .show(ui);
            // 실행하자마자 바로 입력할 수 있도록 처음 몇 프레임 동안 입력란에 포커스를 준다.
            if self.frame <= 3 {
                output.response.request_focus();
            }
            if output.response.changed() {
                self.modified = true;
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "P0-05: 내용을 고친 뒤 창의 X 버튼이나 Alt+F4로 닫아 보세요. 확인 대화상자가 떠야 합니다.",
                )
                .small()
                .weak(),
            );
        });
    }

    pub fn status(&self, ui: &mut egui::Ui) {
        ui.label(format!("{}자", self.text.chars().count()));
        ui.separator();
        if self.modified {
            ui.label(RichText::new("● 수정됨").color(ui.visuals().warn_fg_color));
        } else {
            ui.label("저장됨");
        }
        if self.chord_k_pending {
            ui.separator();
            ui.label(RichText::new("코드 입력 중… (V)").strong());
        }
    }

    /// P0-05: 수정된 상태에서 창 닫기 요청이 오면 취소하고 확인 대화상자를 띄운다.
    pub fn close_dialog(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.modified && !self.allow_close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.show_close_dialog = true;
            push(
                &mut self.command_log,
                format!(
                    "#{:<6}   창 닫기 요청 → 취소하고 확인 대화상자 표시",
                    self.frame
                ),
            );
        }
        if !self.show_close_dialog {
            return;
        }
        let mut close_app = false;
        let mut dismiss = false;
        let modal = egui::Modal::new(egui::Id::unique("close-dialog")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading("변경 내용을 저장할까요?");
            ui.label("\"입력 검증\"의 변경 내용이 저장되지 않았습니다.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("저장").strong()).clicked() {
                    close_app = true;
                }
                if ui.button("저장 안 함").clicked() {
                    close_app = true;
                }
                if ui.button("취소").clicked() {
                    dismiss = true;
                }
            });
        });
        if modal.should_close() {
            dismiss = true;
        }
        if close_app {
            self.show_close_dialog = false;
            self.allow_close = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        } else if dismiss {
            self.show_close_dialog = false;
            push(
                &mut self.command_log,
                format!("#{:<6}   종료 취소됨", self.frame),
            );
        }
    }
}

fn chord_k() -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::COMMAND, Key::K)
}

fn fmt_modifiers(m: Modifiers) -> String {
    let mut parts = Vec::new();
    if m.ctrl {
        parts.push("Ctrl");
    }
    if m.mac_cmd {
        parts.push("Cmd");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", parts.join("+"))
    }
}
