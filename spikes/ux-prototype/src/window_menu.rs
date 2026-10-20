//! 창 메뉴·창 목록·탭 우클릭 메뉴·탭 이동 단축키 (UX-003).
//!
//! 탭 단위로 이동·정렬·닫기를 관리한다. 여러 탭을 한꺼번에 닫을 때는 닫기 대기열을 두고,
//! 수정된 탭마다 저장 확인을 받는다 (취소하면 남은 대기열도 취소).

use std::collections::HashSet;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText};

use crate::{App, Dialog, icons, theme};

#[derive(Clone, Copy)]
pub enum TabCommand {
    Next,
    Prev,
    Goto(usize),
    Last,
    MoveLeft,
    MoveRight,
    Close(usize),
    CloseOthers(usize),
    CloseRight(usize),
    CloseSaved,
    CloseAll,
    CopyPath(usize),
    WindowList,
}

/// 탭 관련 단축키. 한 기능에 여러 키를 둘 수 있으며 첫 번째가 메뉴에 표시된다.
pub struct TabShortcuts {
    pub next: Vec<KeyboardShortcut>,
    pub prev: Vec<KeyboardShortcut>,
    pub move_left: Vec<KeyboardShortcut>,
    pub move_right: Vec<KeyboardShortcut>,
    pub goto: Vec<KeyboardShortcut>,
    pub last: KeyboardShortcut,
    pub close_all: KeyboardShortcut,
}

impl TabShortcuts {
    pub fn for_os() -> Self {
        let cmd = Modifiers::COMMAND;
        let ctrl = Modifiers::CTRL;
        let shift = Modifiers::SHIFT;
        let alt = Modifiers::ALT;
        let mac = cfg!(target_os = "macos");
        let k = KeyboardShortcut::new;
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
        ];
        Self {
            next: if mac {
                vec![
                    k(ctrl, Key::Tab),
                    k(cmd | shift, Key::CloseBracket),
                    k(cmd | alt, Key::ArrowRight),
                ]
            } else {
                vec![k(ctrl, Key::Tab), k(ctrl, Key::PageDown)]
            },
            prev: if mac {
                vec![
                    k(ctrl | shift, Key::Tab),
                    k(cmd | shift, Key::OpenBracket),
                    k(cmd | alt, Key::ArrowLeft),
                ]
            } else {
                vec![k(ctrl | shift, Key::Tab), k(ctrl, Key::PageUp)]
            },
            move_left: if mac {
                vec![k(cmd | shift | alt, Key::ArrowLeft)]
            } else {
                vec![k(ctrl | shift, Key::PageUp)]
            },
            move_right: if mac {
                vec![k(cmd | shift | alt, Key::ArrowRight)]
            } else {
                vec![k(ctrl | shift, Key::PageDown)]
            },
            goto: digits.iter().map(|&d| k(cmd, d)).collect(),
            last: k(cmd, Key::Num9),
            close_all: k(cmd | shift, Key::W),
        }
    }
}

impl App {
    /// 탭 단축키를 다른 단축키보다 먼저 처리한다 (Shift 조합을 먼저 소비해야 함).
    pub(crate) fn handle_tab_shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            return;
        }
        let t = &self.tab_shortcuts;
        let any = |i: &mut egui::InputState, list: &[KeyboardShortcut]| {
            list.iter().any(|s| i.consume_shortcut(s))
        };
        let command = ctx.input_mut(|i| {
            if i.consume_shortcut(&t.close_all) {
                return Some(TabCommand::CloseAll);
            }
            if any(i, &t.move_left) {
                return Some(TabCommand::MoveLeft);
            }
            if any(i, &t.move_right) {
                return Some(TabCommand::MoveRight);
            }
            if any(i, &t.prev) {
                return Some(TabCommand::Prev);
            }
            if any(i, &t.next) {
                return Some(TabCommand::Next);
            }
            if i.consume_shortcut(&t.last) {
                return Some(TabCommand::Last);
            }
            t.goto
                .iter()
                .position(|s| i.consume_shortcut(s))
                .map(TabCommand::Goto)
        });
        if let Some(command) = command {
            self.run_tab_command(ctx, command);
        }
    }

    pub(crate) fn run_tab_command(&mut self, ctx: &egui::Context, command: TabCommand) {
        let n = self.docs.len();
        match command {
            TabCommand::Next => self.switch_to((self.active + 1) % n),
            TabCommand::Prev => self.switch_to((self.active + n - 1) % n),
            TabCommand::Goto(i) => {
                if i < n {
                    self.switch_to(i);
                }
            }
            TabCommand::Last => self.switch_to(n - 1),
            TabCommand::MoveLeft => {
                if self.active > 0 {
                    self.docs.swap(self.active, self.active - 1);
                    self.active -= 1;
                }
            }
            TabCommand::MoveRight => {
                if self.active + 1 < n {
                    self.docs.swap(self.active, self.active + 1);
                    self.active += 1;
                }
            }
            TabCommand::Close(i) => self.queue_close(vec![i]),
            TabCommand::CloseOthers(i) => self.queue_close((0..n).filter(|&j| j != i).collect()),
            TabCommand::CloseRight(i) => self.queue_close((i + 1..n).collect()),
            TabCommand::CloseSaved => {
                let saved: Vec<usize> = (0..n).filter(|&j| !self.docs[j].modified).collect();
                let count = saved.len();
                self.queue_close(saved);
                self.toast(ctx, format!("저장된 탭 {count}개를 닫습니다"));
            }
            TabCommand::CloseAll => self.queue_close((0..n).collect()),
            TabCommand::CopyPath(i) => {
                if let Some(path) = &self.docs[i].path {
                    ctx.copy_text(path.clone());
                    self.toast(ctx, format!("경로 복사됨: {path}"));
                } else {
                    self.toast(ctx, "저장되지 않은 새 문서라 경로가 없습니다");
                }
            }
            TabCommand::WindowList => {
                self.window_list_selected.clear();
                self.dialog = Some(Dialog::WindowList);
            }
        }
    }

    /// 닫을 탭을 대기열에 넣는다 (탭 번호가 바뀌므로 제목으로 기억).
    fn queue_close(&mut self, indices: Vec<usize>) {
        for i in indices {
            if let Some(doc) = self.docs.get(i) {
                self.close_queue.push(doc.title.clone());
            }
        }
    }

    /// 대화상자가 없을 때 매 프레임 호출: 대기열의 탭을 하나씩 닫고, 수정된 탭이면 저장 확인을 띄운다.
    pub(crate) fn process_close_queue(&mut self) {
        while self.dialog.is_none() && !self.close_queue.is_empty() {
            let title = self.close_queue.remove(0);
            let Some(index) = self.docs.iter().position(|d| d.title == title) else {
                continue;
            };
            if self.docs[index].modified {
                self.dialog = Some(Dialog::Unsaved {
                    doc: index,
                    then_quit: false,
                });
            } else {
                self.close_tab(index);
            }
        }
    }

    // ------------------------------------------------------------ 창 메뉴

    pub(crate) fn window_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let fmt = |s: &KeyboardShortcut| ctx.format_shortcut(s);
        let t = &self.tab_shortcuts;
        let mut command = None;
        let mut item =
            |ui: &mut egui::Ui, label: &str, shortcut: Option<String>, cmd: TabCommand| {
                let mut button = egui::Button::new(label);
                if let Some(s) = shortcut {
                    button = button.shortcut_text(s);
                }
                if ui.add(button).clicked() {
                    command = Some(cmd);
                }
            };

        item(ui, "다음 탭", Some(fmt(&t.next[0])), TabCommand::Next);
        item(ui, "이전 탭", Some(fmt(&t.prev[0])), TabCommand::Prev);
        item(
            ui,
            "탭을 왼쪽으로 이동",
            Some(fmt(&t.move_left[0])),
            TabCommand::MoveLeft,
        );
        item(
            ui,
            "탭을 오른쪽으로 이동",
            Some(fmt(&t.move_right[0])),
            TabCommand::MoveRight,
        );
        ui.separator();
        item(
            ui,
            "탭 닫기",
            Some(fmt(&self.shortcuts.close_tab)),
            TabCommand::Close(self.active),
        );
        item(
            ui,
            "다른 탭 모두 닫기",
            None,
            TabCommand::CloseOthers(self.active),
        );
        item(
            ui,
            "오른쪽 탭 모두 닫기",
            None,
            TabCommand::CloseRight(self.active),
        );
        item(ui, "저장된 탭 닫기", None, TabCommand::CloseSaved);
        item(
            ui,
            "모든 탭 닫기",
            Some(fmt(&t.close_all)),
            TabCommand::CloseAll,
        );
        ui.separator();
        item(ui, "창 목록…", None, TabCommand::WindowList);
        ui.separator();

        // 열린 탭 바로가기: 앞 8개는 Ctrl+1~8, 마지막 탭은 Ctrl+9
        let palette = theme::palette(ui.visuals());
        let last = self.docs.len() - 1;
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .min_scrolled_height(320.0)
            .show(ui, |ui| {
                for (i, doc) in self.docs.iter().enumerate() {
                    let shortcut = if i < 8 {
                        fmt(&t.goto[i])
                    } else if i == last {
                        fmt(&t.last)
                    } else {
                        String::new()
                    };
                    let mut text = RichText::new(format!("{}  {}", i + 1, doc.title));
                    if doc.modified {
                        text = text.color(palette.modified_marker);
                    }
                    let r = ui.add(
                        egui::Button::selectable(i == self.active, text).shortcut_text(shortcut),
                    );
                    if r.on_hover_text(doc.path.as_deref().unwrap_or("저장되지 않은 새 문서"))
                        .clicked()
                    {
                        command = Some(TabCommand::Goto(i));
                    }
                }
            });

        if let Some(command) = command {
            self.run_tab_command(ctx, command);
            ui.close();
        }
    }

    /// 탭 우클릭 메뉴. 고른 명령을 돌려준다.
    pub(crate) fn tab_context_menu(
        ui: &mut egui::Ui,
        index: usize,
        has_path: bool,
    ) -> Option<TabCommand> {
        let mut command = None;
        let mut item = |ui: &mut egui::Ui, label: &str, cmd: TabCommand| {
            if ui.button(label).clicked() {
                command = Some(cmd);
            }
        };
        item(ui, "닫기", TabCommand::Close(index));
        item(ui, "다른 탭 모두 닫기", TabCommand::CloseOthers(index));
        item(ui, "오른쪽 탭 모두 닫기", TabCommand::CloseRight(index));
        item(ui, "저장된 탭 닫기", TabCommand::CloseSaved);
        item(ui, "모든 탭 닫기", TabCommand::CloseAll);
        ui.separator();
        ui.add_enabled_ui(has_path, |ui| {
            item(ui, "경로 복사", TabCommand::CopyPath(index));
        });
        if command.is_some() {
            ui.close();
        }
        command
    }

    // ------------------------------------------------------------ 창 목록 대화상자

    /// 창 목록 대화상자 내용. 닫아야 하면 true.
    pub(crate) fn window_list_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let palette = theme::palette(ui.visuals());
        let mut close_dialog = false;
        let mut activate = None;
        ui.set_width(560.0);
        ui.heading("창 목록");
        ui.label(
            RichText::new("체크해서 여러 창을 한꺼번에 닫을 수 있습니다. 줄을 더블클릭하면 그 창으로 이동합니다.")
                .small()
                .weak(),
        );
        ui.add_space(4.0);

        let all_selected = !self.docs.is_empty()
            && self
                .docs
                .iter()
                .all(|d| self.window_list_selected.contains(&d.title));
        let mut toggle_all = all_selected;
        if ui.checkbox(&mut toggle_all, "전체 선택").changed() {
            if toggle_all {
                self.window_list_selected = self.docs.iter().map(|d| d.title.clone()).collect();
            } else {
                self.window_list_selected.clear();
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
                        for (i, doc) in self.docs.iter().enumerate() {
                            let mut checked = self.window_list_selected.contains(&doc.title);
                            if ui.checkbox(&mut checked, "").changed() {
                                if checked {
                                    self.window_list_selected.insert(doc.title.clone());
                                } else {
                                    self.window_list_selected.remove(&doc.title);
                                }
                            }
                            ui.horizontal(|ui| {
                                icons::show(ui, &doc.title, false, false, false);
                                let mut text = RichText::new(&doc.title);
                                if i == self.active {
                                    text = text.strong();
                                }
                                let r = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                                if r.double_clicked() {
                                    activate = Some(i);
                                }
                            });
                            if doc.modified {
                                ui.label(RichText::new("● 수정됨").color(palette.modified_marker));
                            } else {
                                ui.label("");
                            }
                            ui.label(
                                RichText::new(doc.path.as_deref().unwrap_or("(새 문서)"))
                                    .small()
                                    .weak(),
                            );
                            ui.end_row();
                        }
                    });
            });

        ui.separator();
        let selected: Vec<usize> = (0..self.docs.len())
            .filter(|&i| self.window_list_selected.contains(&self.docs[i].title))
            .collect();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(selected.len() == 1, egui::Button::new("이동"))
                .on_hover_text("체크한 창 하나로 이동")
                .clicked()
            {
                activate = selected.first().copied();
            }
            if ui
                .add_enabled(
                    !selected.is_empty(),
                    egui::Button::new(format!("선택한 창 닫기 ({})", selected.len())),
                )
                .clicked()
            {
                self.queue_close(selected.clone());
                close_dialog = true;
            }
            if ui.button("모든 창 닫기").clicked() {
                self.queue_close((0..self.docs.len()).collect());
                close_dialog = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("닫기").strong()).clicked() {
                    close_dialog = true;
                }
            });
        });

        if let Some(i) = activate {
            self.run_tab_command(ctx, TabCommand::Goto(i));
            close_dialog = true;
        }
        if close_dialog {
            self.window_list_selected = HashSet::new();
        }
        close_dialog
    }
}
