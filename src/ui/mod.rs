//! UI 계층 모듈 (`egui` 의존).
//!
//! `core` 도메인 로직과 분리하여 화면 렌더링과 사용자 인터랙션만 담당한다 (PLAN.md §4.1).

pub mod dialogs;
pub mod doc_view;
pub mod editor;
pub mod help;
pub mod markdown_view;
pub mod menu_bar;
pub mod preview_images;
pub mod search_bar;
pub mod sidebar;
pub mod status_bar;
pub mod tab_bar;
pub mod theme;
pub mod viewer;
