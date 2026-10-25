//! 도메인 코어 계층 (`egui` 비의존 순수 Rust 로직 — PLAN.md §4.1).
//!
//! 인코딩, 줄바꿈, 텍스트 버퍼, 문서 모델, 파일 입출력, 검색, 하이라이트,
//! 폴더 트리, 파일 아이콘 규칙, Markdown 블록 모델을 UI와 분리해 단위 테스트 가능하게 유지한다.

pub mod buffer;
pub mod document;
pub mod encoding;
pub mod file_icons;
pub mod file_io;
pub mod fs_tree;
pub mod highlight;
pub mod image_load;
pub mod line_ending;
pub mod markdown;
pub mod search;
