//! Markdown 링크 대상 해석 (PLAN.md §3.8-5, P3-09).
//!
//! - `http(s)://`·`mailto:` 등 스킴이 있는 주소는 외부 링크 (OS 기본 프로그램으로 연다)
//! - `#앵커`만 있으면 같은 문서 안의 제목 — 앵커는 GitHub 슬러그로 비교하고, 슬러그가 아니면 슬러그로 바꿔 다시 찾는다
//! - 그 밖은 파일 링크: 문서 폴더 기준 상대 경로(퍼센트 인코딩 풀기)·절대 경로, `#앵커`는 그 파일의 제목.
//!   저장하지 않은 문서의 상대 경로는 대상을 알 수 없다.
//!
//! 파일이 있는지는 [`LinkTarget::File`]의 `exists`로 알려 준다 (없는 대상은 다른 색 + 클릭 시 안내).

use std::path::{Path, PathBuf};

use crate::core::image_load::{has_scheme, percent_decode};
use crate::core::markdown::{Document, slugify};

/// 링크 대상.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// 외부 주소 (웹 브라우저·메일 프로그램)
    External(String),
    /// 같은 문서의 제목 앵커 (`#` 뺀 원문)
    Anchor(String),
    /// 파일 (`anchor`는 그 파일의 제목 앵커)
    File {
        path: PathBuf,
        anchor: Option<String>,
        exists: bool,
    },
    /// 대상을 알 수 없음 — 사유
    Unresolved(String),
}

impl LinkTarget {
    /// 대상이 없어 다른 색으로 보여 줄 링크인지 (없는 파일·알 수 없는 대상).
    pub fn is_broken(&self) -> bool {
        matches!(
            self,
            LinkTarget::File { exists: false, .. } | LinkTarget::Unresolved(_)
        )
    }
}

/// `url`을 해석한다. `doc_dir`은 문서가 있는 폴더(저장하지 않은 문서면 `None`).
pub fn resolve(url: &str, doc_dir: Option<&Path>) -> LinkTarget {
    let url = url.trim();
    if url.is_empty() {
        return LinkTarget::Unresolved("빈 링크".to_owned());
    }
    if url.starts_with("//") {
        return LinkTarget::External(format!("https:{url}"));
    }
    if let Some(anchor) = url.strip_prefix('#') {
        return LinkTarget::Anchor(percent_decode(anchor));
    }
    let lower = url.to_ascii_lowercase();
    if has_scheme(url) && !lower.starts_with("file:") {
        return LinkTarget::External(url.to_owned());
    }
    let (path_part, anchor) = match url.split_once('#') {
        Some((p, a)) => (p, Some(percent_decode(a))),
        None => (url, None),
    };
    // `?쿼리`는 파일 링크에서 의미가 없으므로 버린다
    let path_part = path_part.split('?').next().unwrap_or(path_part);
    let path_part = if lower.starts_with("file://") {
        let rest = &path_part["file://".len()..];
        // file:///C:/a.md → C:/a.md
        let b = rest.as_bytes();
        if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
            &rest[1..]
        } else {
            rest
        }
    } else {
        path_part
    };
    let path = PathBuf::from(percent_decode(path_part));
    let path = if path.is_absolute() {
        path
    } else {
        match doc_dir {
            Some(dir) => dir.join(path),
            None => {
                return LinkTarget::Unresolved(
                    "저장하지 않은 문서라 상대 경로를 찾을 수 없음".to_owned(),
                );
            }
        }
    };
    let exists = path.exists();
    LinkTarget::File {
        path,
        anchor,
        exists,
    }
}

/// 앵커(`#` 뺀 원문)에 해당하는 제목의 소스 줄. 슬러그가 그대로 없으면 원문을 슬러그로 바꿔 찾는다.
pub fn anchor_line(doc: &Document, anchor: &str) -> Option<usize> {
    doc.anchor_line(anchor)
        .or_else(|| doc.anchor_line(&anchor.to_lowercase()))
        .or_else(|| doc.anchor_line(&slugify(anchor)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::markdown::parse;

    #[test]
    fn resolves_external_anchor_and_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("other.md"), "# 다른 문서\n").unwrap();
        let d = Some(dir.path());
        assert_eq!(
            resolve("https://example.com/a?b=1#c", d),
            LinkTarget::External("https://example.com/a?b=1#c".into())
        );
        assert_eq!(
            resolve("mailto:me@example.com", d),
            LinkTarget::External("mailto:me@example.com".into())
        );
        assert_eq!(
            resolve("#3.8-소스%20보기", d),
            LinkTarget::Anchor("3.8-소스 보기".into())
        );
        assert_eq!(
            resolve("other.md#다른-문서", d),
            LinkTarget::File {
                path: dir.path().join("other.md"),
                anchor: Some("다른-문서".into()),
                exists: true,
            }
        );
        let missing = resolve("없는%20파일.md", d);
        assert_eq!(
            missing,
            LinkTarget::File {
                path: dir.path().join("없는 파일.md"),
                anchor: None,
                exists: false,
            }
        );
        assert!(missing.is_broken());
        assert!(resolve("a.md", None).is_broken());
        assert!(!resolve("https://e.com", None).is_broken());
    }

    #[test]
    fn anchor_matches_slug_or_heading_text() {
        let doc = parse("# 개요\n\n## 3.8 Markdown 소스 보기\n\n## 개요\n");
        assert_eq!(anchor_line(&doc, "38-markdown-소스-보기"), Some(2));
        // 원문 제목을 그대로 써도 슬러그로 바꿔 찾는다
        assert_eq!(anchor_line(&doc, "3.8 Markdown 소스 보기"), Some(2));
        assert_eq!(anchor_line(&doc, "개요-1"), Some(4));
        assert_eq!(anchor_line(&doc, "없음"), None);
    }
}
