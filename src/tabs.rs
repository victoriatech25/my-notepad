//! 열린 탭 목록 (`Tabs` — PLAN.md §3.1, P1-10).
//!
//! 각 탭은 독립된 [`Document`]와 [`EditorState`]를 가진다. 커서·스크롤·Undo 이력은 문서와 위젯 상태에
//! 탭별로 남으므로, 탭을 오가도 그대로 유지된다.
//!
//! - 새 문서 번호는 비어 있는 가장 작은 `Untitled-N`을 쓴다 (`Untitled-1`을 닫으면 다음 새 문서가 다시 1번).
//! - 같은 파일은 한 번만 연다. 경로는 [`std::fs::canonicalize`] 결과로 비교한다 (Windows 대소문자·`..`·심볼릭 링크).
//! - 파일을 열 때 지금 탭이 손대지 않은 빈 새 문서면 그 탭을 바꿔 쓴다 (첫 실행 직후 열기).
//! - 마지막 탭을 닫으면 빈 `Untitled-1`을 새로 만든다 (탭이 0개인 상태는 없다).

use std::path::{Path, PathBuf};

use crate::commands::ViewMode;
use crate::core::document::Document;
use crate::ui::editor::EditorState;

/// 열린 탭 하나.
#[derive(Debug)]
pub struct Tab {
    id: u64,
    pub doc: Document,
    pub editor: EditorState,
    /// Markdown 보기 모드 (탭 단위 상태 — ui-spec 탭 바).
    pub view_mode: ViewMode,
    /// 중복 열기 판정용 정규화 경로.
    path_key: Option<PathBuf>,
    /// 파일 시스템의 읽기 전용 속성 (저장 시 다른 이름으로 저장 유도).
    pub file_readonly: bool,
}

impl Tab {
    fn new(id: u64, doc: Document) -> Self {
        let mut editor = EditorState::new(id);
        editor.restore_view(&doc);
        editor.request_focus();
        let path_key = doc.path().map(path_key);
        // `.md` 기본 보기 모드는 분할 (ui-spec §5, UX-1 승인)
        let view_mode = if doc.is_markdown() {
            ViewMode::Split
        } else {
            ViewMode::Source
        };
        Self {
            id,
            doc,
            editor,
            view_mode,
            path_key,
            file_readonly: false,
        }
    }

    /// 탭 고유 번호 (위젯 ID용, 순서와 무관).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// 다른 이름으로 저장 후 경로가 바뀌었을 때 중복 열기 판정 경로를 갱신한다.
    pub fn refresh_path_key(&mut self) {
        self.path_key = self.doc.path().map(path_key);
    }

    /// 같은 탭에서 문서를 바꾼다 (인코딩으로 다시 읽기). 커서는 새 문서 범위 안으로 맞추고 스크롤은 유지한다.
    pub fn replace_document(&mut self, mut doc: Document) {
        let len = doc.text().chars().count();
        let cursor = self.doc.cursor_char_range();
        doc.set_cursor_char_range(cursor.start.min(len)..cursor.end.min(len));
        doc.set_scroll_offset_y(self.doc.scroll_offset_y());
        self.doc = doc;
        self.editor.invalidate();
        self.editor.restore_view(&self.doc);
        self.refresh_path_key();
    }

    /// 손대지 않은 빈 새 문서인지 (파일을 열면 이 탭을 바꿔 쓴다).
    fn is_pristine_untitled(&self) -> bool {
        self.doc.path().is_none() && !self.doc.is_modified() && self.doc.text().is_empty()
    }

    fn untitled_number(&self) -> Option<usize> {
        if self.doc.path().is_some() {
            return None;
        }
        self.doc.title().strip_prefix("Untitled-")?.parse().ok()
    }
}

/// 중복 열기 판정에 쓰는 정규화 경로 (파일이 없으면 받은 경로 그대로).
pub fn path_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 열린 탭 목록과 활성 탭.
#[derive(Debug)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

impl Tabs {
    /// `Untitled-1` 하나로 시작한다.
    pub fn new() -> Self {
        let mut tabs = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
        };
        tabs.new_untitled();
        tabs
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn get(&self, index: usize) -> Option<&Tab> {
        self.tabs.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Tab> {
        self.tabs.get_mut(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter()
    }

    /// 새 빈 문서 탭을 활성 탭 바로 오른쪽이 아니라 맨 끝에 만들고 활성화한다.
    pub fn new_untitled(&mut self) -> usize {
        let mut number = 1;
        while self
            .tabs
            .iter()
            .any(|t| t.untitled_number() == Some(number))
        {
            number += 1;
        }
        self.push(Document::new_untitled(number))
    }

    /// 열린 문서를 탭으로 추가하고 활성화한다. 활성 탭이 손대지 않은 빈 새 문서면 그 자리를 바꿔 쓴다.
    pub fn open_document(&mut self, doc: Document, file_readonly: bool) -> usize {
        let index = if self
            .tabs
            .get(self.active)
            .is_some_and(Tab::is_pristine_untitled)
        {
            let id = self.alloc_id();
            self.tabs[self.active] = Tab::new(id, doc);
            self.active
        } else {
            self.push(doc)
        };
        self.tabs[index].file_readonly = file_readonly;
        index
    }

    /// 이미 열린 파일의 탭 번호.
    pub fn find_path(&self, path: &Path) -> Option<usize> {
        let key = path_key(path);
        self.tabs
            .iter()
            .position(|t| t.path_key.as_ref() == Some(&key))
    }

    /// `index` 탭을 활성화한다. 범위를 벗어나면 아무것도 하지 않고 `false`.
    pub fn activate(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        tab.editor.restore_view(&tab.doc);
        tab.editor.request_focus();
        self.active = index;
        true
    }

    /// 다음 탭 (끝에서는 처음으로).
    pub fn next(&mut self) {
        self.activate((self.active + 1) % self.tabs.len());
    }

    /// 이전 탭 (처음에서는 끝으로).
    pub fn prev(&mut self) {
        self.activate((self.active + self.tabs.len() - 1) % self.tabs.len());
    }

    pub fn last(&mut self) {
        self.activate(self.tabs.len() - 1);
    }

    /// 활성 탭을 왼쪽(`-1`) 또는 오른쪽(`+1`)으로 옮긴다. 끝에서는 움직이지 않는다.
    pub fn move_active(&mut self, right: bool) {
        let target = if right {
            self.active + 1
        } else {
            self.active.wrapping_sub(1)
        };
        if target < self.tabs.len() {
            self.tabs.swap(self.active, target);
            self.active = target;
        }
    }

    /// 탭 ID로 현재 탭 번호를 찾는다 (닫기 대기열처럼 번호가 바뀌는 동안 탭을 기억할 때).
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// 탭 번호 목록을 탭 ID 목록으로 바꾼다 (범위 밖 번호는 무시).
    pub fn ids_of(&self, indices: &[usize]) -> Vec<u64> {
        indices
            .iter()
            .filter_map(|&i| self.tabs.get(i).map(|t| t.id))
            .collect()
    }

    /// 수정되지 않은 탭만 닫고, 닫지 않은(수정된) 탭 수를 돌려준다 (`저장된 탭 닫기`).
    pub fn close_unmodified(&mut self, indices: &[usize]) -> usize {
        let skipped = indices
            .iter()
            .filter(|&&i| self.tabs.get(i).is_some_and(|t| t.doc.is_modified()))
            .count();
        self.remove_where(|index, tab| indices.contains(&index) && !tab.doc.is_modified());
        skipped
    }

    /// 지정한 탭을 수정 여부와 상관없이 닫는다. 저장 확인은 호출하는 쪽(닫기 대기열)이 끝낸 뒤여야 한다.
    pub fn close_ids(&mut self, ids: &[u64]) {
        self.remove_where(|_, tab| ids.contains(&tab.id));
    }

    /// 조건에 맞는 탭을 닫는다. 활성 탭이 닫히면 그 자리에 온 탭(없으면 왼쪽)으로 옮기고,
    /// 모두 닫히면 빈 `Untitled-1`을 만든다.
    fn remove_where(&mut self, mut remove: impl FnMut(usize, &Tab) -> bool) {
        let active_id = self.tabs[self.active].id;
        let old_active = self.active;
        let mut removed_before_active = 0;
        let mut index = 0;
        self.tabs.retain(|tab| {
            let r = remove(index, tab);
            if r && index < old_active {
                removed_before_active += 1;
            }
            index += 1;
            !r
        });

        if self.tabs.is_empty() {
            self.active = 0;
            self.new_untitled();
        } else if let Some(i) = self.index_of(active_id) {
            self.active = i;
        } else {
            let fallback = (old_active - removed_before_active).min(self.tabs.len() - 1);
            self.activate(fallback);
        }
    }

    fn push(&mut self, doc: Document) -> usize {
        let id = self.alloc_id();
        self.tabs.push(Tab::new(id, doc));
        let index = self.tabs.len() - 1;
        self.active = index;
        index
    }

    fn alloc_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(tabs: &Tabs) -> Vec<String> {
        tabs.iter().map(|t| t.doc.title()).collect()
    }

    fn file_doc(path: &Path) -> Document {
        Document::from_bytes(Some(path.to_path_buf()), b"text", false, None).unwrap()
    }

    #[test]
    fn untitled_numbers_reuse_smallest_free() {
        let mut tabs = Tabs::new();
        tabs.new_untitled();
        tabs.new_untitled();
        assert_eq!(titles(&tabs), ["Untitled-1", "Untitled-2", "Untitled-3"]);
        assert_eq!(tabs.active_index(), 2);

        assert_eq!(tabs.close_unmodified(&[0]), 0);
        tabs.new_untitled();
        assert_eq!(titles(&tabs), ["Untitled-2", "Untitled-3", "Untitled-1"]);
    }

    #[test]
    fn opening_replaces_pristine_untitled_and_dedupes_paths() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();

        let mut tabs = Tabs::new();
        assert_eq!(tabs.open_document(file_doc(&a), false), 0);
        assert_eq!(titles(&tabs), ["a.txt"]);

        tabs.open_document(file_doc(&b), true);
        assert_eq!(titles(&tabs), ["a.txt", "b.md"]);
        assert!(tabs.active().file_readonly);

        // 같은 파일을 다른 표기로 찾아도 기존 탭
        let alias = dir.path().join(".").join("a.txt");
        assert_eq!(tabs.find_path(&alias), Some(0));
        assert_eq!(tabs.find_path(&dir.path().join("c.txt")), None);

        // 수정된 새 문서는 바꿔 쓰지 않는다
        tabs.new_untitled();
        tabs.active_mut().doc.set_text("메모");
        tabs.open_document(file_doc(&dir.path().join("c.txt")), false);
        assert_eq!(tabs.len(), 4);
    }

    #[test]
    fn navigation_wraps_and_move_stays_in_bounds() {
        let mut tabs = Tabs::new();
        tabs.new_untitled();
        tabs.new_untitled();
        tabs.activate(0);

        tabs.prev();
        assert_eq!(tabs.active_index(), 2);
        tabs.next();
        assert_eq!(tabs.active_index(), 0);
        assert!(!tabs.activate(5));
        tabs.last();
        assert_eq!(tabs.active_index(), 2);

        tabs.move_active(true);
        assert_eq!(tabs.active_index(), 2);
        tabs.move_active(false);
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(titles(&tabs), ["Untitled-1", "Untitled-3", "Untitled-2"]);
        tabs.activate(0);
        tabs.move_active(false);
        assert_eq!(tabs.active_index(), 0);
    }

    #[test]
    fn closing_keeps_modified_tabs_and_never_leaves_zero_tabs() {
        let mut tabs = Tabs::new();
        tabs.new_untitled();
        tabs.new_untitled();
        tabs.get_mut(1).unwrap().doc.set_text("수정됨");
        tabs.activate(2);

        // 모두 닫기: 수정된 탭 1개는 남는다
        assert_eq!(tabs.close_unmodified(&[0, 1, 2]), 1);
        assert_eq!(titles(&tabs), ["Untitled-2"]);
        assert_eq!(tabs.active_index(), 0);

        tabs.get_mut(0).unwrap().doc.undo();
        assert_eq!(tabs.close_unmodified(&[0]), 0);
        assert_eq!(titles(&tabs), ["Untitled-1"]);
    }

    #[test]
    fn close_ids_closes_modified_tabs_by_identity() {
        let mut tabs = Tabs::new();
        tabs.new_untitled();
        tabs.new_untitled();
        tabs.get_mut(1).unwrap().doc.set_text("수정됨");
        let ids = tabs.ids_of(&[1, 2, 9]);
        assert_eq!(ids.len(), 2);

        // 앞쪽 탭을 먼저 닫아 번호가 바뀌어도 ID로 찾는다
        tabs.close_ids(&ids[1..]);
        assert_eq!(tabs.index_of(ids[0]), Some(1));
        tabs.close_ids(&ids[..1]);
        assert_eq!(titles(&tabs), ["Untitled-1"]);
        assert_eq!(tabs.index_of(ids[0]), None);
    }

    #[test]
    fn markdown_defaults_to_split_and_replace_keeps_tab() {
        let mut tabs = Tabs::new();
        let md = Document::from_bytes(Some("notes.md".into()), b"# a", false, None).unwrap();
        tabs.open_document(md, false);
        assert_eq!(tabs.active().view_mode, ViewMode::Split);
        tabs.new_untitled();
        assert_eq!(tabs.active().view_mode, ViewMode::Source);

        // 다시 읽기: 같은 탭(ID)에서 문서만 바뀌고 커서는 새 길이 안으로
        tabs.activate(0);
        let id = tabs.active().id();
        tabs.active_mut().doc.set_cursor_char_range(3..3);
        let shorter = Document::from_bytes(Some("notes.md".into()), b"#", false, None).unwrap();
        tabs.active_mut().replace_document(shorter);
        assert_eq!(tabs.active().id(), id);
        assert_eq!(tabs.active().doc.text(), "#");
        assert_eq!(tabs.active().doc.cursor_char_range(), 1..1);
    }

    #[test]
    fn closing_active_tab_moves_to_neighbor() {
        let mut tabs = Tabs::new();
        tabs.new_untitled();
        tabs.new_untitled();
        tabs.new_untitled();
        tabs.activate(1);
        tabs.close_unmodified(&[1]);
        assert_eq!(titles(&tabs), ["Untitled-1", "Untitled-3", "Untitled-4"]);
        assert_eq!(tabs.active_index(), 1);

        tabs.activate(2);
        tabs.close_unmodified(&[2]);
        assert_eq!(tabs.active_index(), 1);

        // 활성 탭 왼쪽을 닫으면 활성 탭은 그대로 (번호만 당겨짐)
        tabs.new_untitled();
        tabs.close_unmodified(&[0]);
        assert_eq!(tabs.active().doc.title(), "Untitled-2");
    }
}
