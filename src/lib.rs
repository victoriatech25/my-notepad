//! My-Notepad 라이브러리 루트 (PLAN.md §4.2).
//!
//! `src/main.rs` 바이너리와 `tests/*.rs` 통합 테스트에서 도메인 코어(`core`) 및 UI·앱 모듈을 공유한다.

pub mod app;
pub mod branding;
pub mod cli;
pub mod commands;
pub mod config;
pub mod core;
pub mod fonts;
pub mod logging;
pub mod single_instance;
pub mod tabs;
pub mod ui;
