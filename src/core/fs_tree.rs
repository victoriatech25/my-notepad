//! 좌측 사이드바 폴더 트리 모델 (PLAN.md §3.1-2·§3.1-3, P2-05). `egui`에 의존하지 않는다.
//!
//! - **지연 로딩**: 폴더를 펼칠 때 그 단계만 읽는다 (큰 폴더에서 UI 멈춤 방지). 처음에는 열린 폴더의 1단계만 읽음
//! - **숨김 목록**: gitignore 문법 패턴([`DEFAULT_HIDDEN`] — `.git/`, `node_modules/` 등)에 걸리면 트리에 넣지 않는다
//! - **`.gitignore` 대상 흐리게**: 열린 폴더 안의 `.gitignore`(하위 폴더 것 포함, 깊은 쪽 우선)에 걸리는 항목은
//!   숨기지 않고 [`Node::ignored`]로 표시한다. 무시된 폴더의 하위 항목도 무시된 것으로 본다 (git과 같은 규칙).
//!   `target`은 숨김 목록이 아니라 이 규칙으로 흐리게 보인다 (§3.1-3, P2-06 확인 항목)
//! - **심볼릭 링크 순환 방지**: 펼치려는 폴더의 실제 경로가 자기 자신이나 조상과 같으면 읽지 않고 오류로 표시
//! - **정렬**: 폴더 먼저, 그다음 이름(대소문자 무시)
//! - **읽기 실패**(권한 없음 등)는 그 폴더의 [`Node::error`]로 남기고 트리는 계속 쓸 수 있다
//! - **새로고침**: 트리를 다시 읽되 펼쳐 둔 폴더는 다시 펼친다 (실시간 감시는 범위 밖)

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// 기본 숨김 목록 (gitignore 문법, 어느 깊이든 이름이 같으면 숨김).
pub const DEFAULT_HIDDEN: &[&str] = &[
    ".git/",
    "node_modules/",
    ".svn/",
    ".hg/",
    ".DS_Store",
    "Thumbs.db",
];

/// 트리 노드 번호 (같은 트리 안에서만 유효, 새로고침하면 바뀐다).
pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
}

/// 트리 노드 (파일 또는 폴더).
#[derive(Debug)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub kind: NodeKind,
    /// 열린 폴더 바로 아래가 0
    pub depth: usize,
    pub parent: Option<NodeId>,
    /// 자식 노드 (`None`이면 아직 읽지 않은 폴더)
    pub children: Option<Vec<NodeId>>,
    pub expanded: bool,
    /// `.gitignore` 대상 (흐리게 표시)
    pub ignored: bool,
    /// 심볼릭 링크(또는 Windows 연결 지점)인지 여부
    pub symlink: bool,
    /// 폴더를 읽지 못한 이유 (권한 없음, 순환 링크 등)
    pub error: Option<String>,
    /// 폴더의 실제 경로 (펼칠 때 계산, 순환 판정용)
    canonical: Option<PathBuf>,
    /// 이 폴더에 있는 `.gitignore` 규칙
    gitignore: Option<Arc<Gitignore>>,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.kind == NodeKind::Dir
    }
}

/// 폴더 트리.
#[derive(Debug)]
pub struct FsTree {
    nodes: Vec<Node>,
    hidden_patterns: Vec<String>,
    hidden: Gitignore,
}

impl FsTree {
    /// `root` 폴더를 열고 1단계를 읽는다. 숨김 패턴은 gitignore 문법.
    pub fn open(root: &Path, hidden_patterns: &[&str]) -> io::Result<Self> {
        if !fs::metadata(root)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "폴더가 아닙니다",
            ));
        }
        let mut builder = GitignoreBuilder::new(root);
        for pattern in hidden_patterns {
            builder
                .add_line(None, pattern)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        }
        let hidden = builder
            .build()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        let name = root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let mut tree = Self {
            nodes: vec![Node {
                name,
                path: root.to_path_buf(),
                kind: NodeKind::Dir,
                depth: 0,
                parent: None,
                children: None,
                expanded: false,
                ignored: false,
                symlink: false,
                error: None,
                canonical: None,
                gitignore: None,
            }],
            hidden_patterns: hidden_patterns.iter().map(|p| (*p).to_owned()).collect(),
            hidden,
        };
        tree.expand(tree.root());
        Ok(tree)
    }

    /// 열린 폴더(루트) 노드.
    pub fn root(&self) -> NodeId {
        0
    }

    pub fn root_path(&self) -> &Path {
        &self.nodes[0].path
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// 폴더를 펼친다 (처음이면 그 단계를 읽는다). 파일이면 아무것도 하지 않는다.
    pub fn expand(&mut self, id: NodeId) {
        if !self.nodes[id].is_dir() {
            return;
        }
        if self.nodes[id].children.is_none() {
            self.load_children(id);
        }
        self.nodes[id].expanded = true;
    }

    pub fn collapse(&mut self, id: NodeId) {
        self.nodes[id].expanded = false;
    }

    /// 펼침 ↔ 접힘.
    pub fn toggle(&mut self, id: NodeId) {
        if self.nodes[id].expanded {
            self.collapse(id);
        } else {
            self.expand(id);
        }
    }

    /// 화면에 보이는 노드 (루트 아래부터, 펼친 폴더만 따라 내려간 순서).
    pub fn visible_rows(&self) -> Vec<NodeId> {
        let mut rows = Vec::new();
        let mut stack: Vec<NodeId> = self
            .children_of(self.root())
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(id) = stack.pop() {
            rows.push(id);
            if self.nodes[id].expanded {
                stack.extend(self.children_of(id).iter().rev());
            }
        }
        rows
    }

    /// 읽어 둔 자식 (읽지 않았으면 빈 목록).
    pub fn children_of(&self, id: NodeId) -> &[NodeId] {
        self.nodes[id].children.as_deref().unwrap_or(&[])
    }

    /// 이미 읽어 둔 노드 중 경로가 같은 것.
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        self.nodes.iter().position(|n| n.path == path)
    }

    /// 디스크에서 다시 읽는다. 펼쳐 둔 폴더는 (아직 있으면) 다시 펼친다.
    pub fn refresh(&mut self) -> io::Result<()> {
        let mut expanded: Vec<(usize, PathBuf)> = self
            .nodes
            .iter()
            .skip(1)
            .filter(|n| n.expanded)
            .map(|n| (n.depth, n.path.clone()))
            .collect();
        expanded.sort_by_key(|(depth, _)| *depth);
        let patterns: Vec<&str> = self.hidden_patterns.iter().map(String::as_str).collect();
        let mut fresh = Self::open(self.root_path(), &patterns)?;
        for (_, path) in expanded {
            if let Some(id) = fresh.find(&path) {
                fresh.expand(id);
            }
        }
        *self = fresh;
        Ok(())
    }

    fn load_children(&mut self, id: NodeId) {
        let path = self.nodes[id].path.clone();
        let depth = if id == self.root() {
            0
        } else {
            self.nodes[id].depth + 1
        };
        self.nodes[id].children = Some(Vec::new());

        // 심볼릭 링크 순환: 실제 경로가 자기 조상과 같으면 읽지 않는다.
        match fs::canonicalize(&path) {
            Ok(canonical) => {
                let mut ancestor = self.nodes[id].parent;
                while let Some(a) = ancestor {
                    if self.nodes[a].canonical.as_ref() == Some(&canonical) {
                        self.nodes[id].error = Some("순환하는 링크라 펼치지 않습니다".to_owned());
                        return;
                    }
                    ancestor = self.nodes[a].parent;
                }
                self.nodes[id].canonical = Some(canonical);
            }
            Err(e) => {
                self.nodes[id].error = Some(describe_error(&e));
                return;
            }
        }

        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) => {
                self.nodes[id].error = Some(describe_error(&e));
                return;
            }
        };
        self.nodes[id].error = None;
        self.nodes[id].gitignore = load_gitignore(&path);

        let parent_ignored = self.nodes[id].ignored;
        let mut children: Vec<Node> = entries
            .flatten()
            .filter_map(|entry| {
                let entry_path = entry.path();
                let file_type = entry.file_type().ok()?;
                let symlink = file_type.is_symlink();
                // 링크는 가리키는 대상이 폴더인지로 판단 (끊어진 링크는 파일로 보여 준다)
                let is_dir = if symlink {
                    fs::metadata(&entry_path).is_ok_and(|m| m.is_dir())
                } else {
                    file_type.is_dir()
                };
                if self.hidden.matched(&entry_path, is_dir).is_ignore() {
                    return None;
                }
                let ignored = parent_ignored || self.gitignore_matches(id, &entry_path, is_dir);
                Some(Node {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    path: entry_path,
                    kind: if is_dir {
                        NodeKind::Dir
                    } else {
                        NodeKind::File
                    },
                    depth,
                    parent: Some(id),
                    children: None,
                    expanded: false,
                    ignored,
                    symlink,
                    error: None,
                    canonical: None,
                    gitignore: None,
                })
            })
            .collect();
        children.sort_by(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });

        let first = self.nodes.len();
        let ids = (first..first + children.len()).collect();
        self.nodes.extend(children);
        self.nodes[id].children = Some(ids);
    }

    /// `dir` 폴더부터 조상 쪽으로 `.gitignore`를 확인한다. 깊은 쪽 규칙이 먼저 결정한다.
    fn gitignore_matches(&self, dir: NodeId, path: &Path, is_dir: bool) -> bool {
        let mut current = Some(dir);
        while let Some(id) = current {
            if let Some(gitignore) = &self.nodes[id].gitignore {
                match gitignore.matched(path, is_dir) {
                    Match::Ignore(_) => return true,
                    Match::Whitelist(_) => return false,
                    Match::None => {}
                }
            }
            current = self.nodes[id].parent;
        }
        false
    }
}

/// 폴더의 `.gitignore`를 읽는다 (없거나 읽을 수 없으면 `None`, 잘못된 줄은 건너뜀).
fn load_gitignore(dir: &Path) -> Option<Arc<Gitignore>> {
    let file = dir.join(".gitignore");
    if !file.is_file() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(dir);
    if let Some(e) = builder.add(&file) {
        log::warn!("{} 일부 규칙을 읽지 못함: {e}", file.display());
    }
    builder.build().ok().map(Arc::new)
}

fn describe_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => "권한이 없어 열 수 없습니다".to_owned(),
        io::ErrorKind::NotFound => "폴더가 없습니다".to_owned(),
        _ => format!("열 수 없습니다: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"").unwrap();
    }

    fn names(tree: &FsTree, ids: &[NodeId]) -> Vec<String> {
        ids.iter().map(|&id| tree.node(id).name.clone()).collect()
    }

    fn child(tree: &FsTree, parent: NodeId, name: &str) -> NodeId {
        *tree
            .children_of(parent)
            .iter()
            .find(|&&id| tree.node(id).name == name)
            .unwrap_or_else(|| panic!("{name} 없음"))
    }

    #[test]
    fn loads_lazily_and_sorts_folders_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("b.txt"));
        touch(&root.join("A.md"));
        touch(&root.join("zeta/inner/deep.rs"));
        touch(&root.join("Alpha/x.txt"));

        let mut tree = FsTree::open(root, DEFAULT_HIDDEN).unwrap();
        let top = tree.children_of(tree.root()).to_vec();
        assert_eq!(names(&tree, &top), ["Alpha", "zeta", "A.md", "b.txt"]);

        // 펼치기 전에는 하위 단계를 읽지 않는다
        let zeta = child(&tree, tree.root(), "zeta");
        assert!(tree.node(zeta).children.is_none());
        assert_eq!(tree.visible_rows().len(), 4);

        tree.expand(zeta);
        let inner = child(&tree, zeta, "inner");
        assert!(tree.node(inner).children.is_none(), "한 단계만 읽음");
        assert_eq!(tree.node(inner).depth, 1);
        assert_eq!(
            names(&tree, &tree.visible_rows()),
            ["Alpha", "zeta", "inner", "A.md", "b.txt"]
        );
        tree.collapse(zeta);
        assert_eq!(tree.visible_rows().len(), 4);
        tree.toggle(zeta);
        assert_eq!(tree.visible_rows().len(), 5);
    }

    #[test]
    fn hidden_patterns_are_not_listed_but_target_is() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join(".git/HEAD"));
        touch(&root.join("node_modules/pkg/index.js"));
        touch(&root.join("sub/node_modules/x.js"));
        touch(&root.join("sub/.DS_Store"));
        touch(&root.join("target/debug/app.exe"));
        touch(&root.join(".gitignore"));
        touch(&root.join("src/main.rs"));

        let mut tree = FsTree::open(root, DEFAULT_HIDDEN).unwrap();
        let top = tree.children_of(tree.root()).to_vec();
        assert_eq!(names(&tree, &top), ["src", "sub", "target", ".gitignore"]);
        let sub = child(&tree, tree.root(), "sub");
        tree.expand(sub);
        assert!(
            tree.children_of(sub).is_empty(),
            "하위 폴더의 숨김 대상도 숨김"
        );

        // 사용자 패턴 (gitignore 문법)
        let tree = FsTree::open(root, &["*.rs", "sub/"]).unwrap();
        let top = tree.children_of(tree.root()).to_vec();
        assert!(names(&tree, &top).contains(&".git".to_owned()));
        assert!(!names(&tree, &top).contains(&"sub".to_owned()));
    }

    #[test]
    fn gitignore_rules_mark_entries_dimmed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "target/\n*.log\n!keep.log\n").unwrap();
        touch(&root.join("target/debug/app.exe"));
        touch(&root.join("app.log"));
        touch(&root.join("keep.log"));
        touch(&root.join("main.rs"));
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/.gitignore"), "secret.txt\n!important.log\n").unwrap();
        touch(&root.join("sub/secret.txt"));
        touch(&root.join("sub/other.log"));
        touch(&root.join("sub/important.log"));
        touch(&root.join("other/secret.txt"));

        let mut tree = FsTree::open(root, DEFAULT_HIDDEN).unwrap();
        let ignored = |tree: &FsTree, parent: NodeId, name: &str| {
            tree.node(child(tree, parent, name)).ignored
        };
        let r = tree.root();
        assert!(ignored(&tree, r, "target"), "`target/`");
        assert!(ignored(&tree, r, "app.log"), "`*.log`");
        assert!(!ignored(&tree, r, "keep.log"), "부정 규칙 `!keep.log`");
        assert!(!ignored(&tree, r, "main.rs"));

        // 무시된 폴더의 하위 항목도 무시
        let target = child(&tree, r, "target");
        tree.expand(target);
        assert!(ignored(&tree, target, "debug"));

        // 하위 폴더의 `.gitignore`: 그 폴더 안에서만, 깊은 쪽이 먼저 결정
        let sub = child(&tree, r, "sub");
        tree.expand(sub);
        assert!(ignored(&tree, sub, "secret.txt"));
        assert!(ignored(&tree, sub, "other.log"), "상위 `*.log` 적용");
        assert!(
            !ignored(&tree, sub, "important.log"),
            "하위 부정 규칙이 우선"
        );
        let other = child(&tree, r, "other");
        tree.expand(other);
        assert!(
            !ignored(&tree, other, "secret.txt"),
            "다른 폴더에는 적용 안 됨"
        );
    }

    #[test]
    fn unreadable_folder_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("gone/a.txt"));
        touch(&root.join("ok.txt"));
        let mut tree = FsTree::open(root, DEFAULT_HIDDEN).unwrap();
        let gone = child(&tree, tree.root(), "gone");
        fs::remove_dir_all(root.join("gone")).unwrap();
        tree.expand(gone);
        assert!(tree.node(gone).error.is_some());
        assert!(tree.children_of(gone).is_empty());
        assert_eq!(tree.visible_rows().len(), 2, "나머지 트리는 그대로");

        assert!(FsTree::open(&root.join("ok.txt"), DEFAULT_HIDDEN).is_err());
        assert!(FsTree::open(&root.join("nope"), DEFAULT_HIDDEN).is_err());
    }

    /// 권한 없는 폴더 (Unix: 권한 000 — Windows는 ACL을 바꿔야 해 macOS CI에서 확인)
    #[cfg(unix)]
    #[test]
    fn permission_denied_folder_shows_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        touch(&locked.join("a.txt"));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let mut tree = FsTree::open(dir.path(), DEFAULT_HIDDEN).unwrap();
        let id = child(&tree, tree.root(), "locked");
        tree.expand(id);
        let error = tree.node(id).error.clone();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        // root로 실행하면 권한 검사를 건너뛰므로 오류가 없을 수 있다
        if let Some(error) = error {
            assert!(error.contains("권한"), "{error}");
        }
    }

    #[test]
    fn symlink_cycle_is_not_expanded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("a/file.txt"));
        let link = root.join("a/back");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(root, &link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(root, &link);
        if let Err(e) = made {
            // Windows에서는 개발자 모드·관리자 권한이 없으면 링크를 만들 수 없다 (macOS CI에서 확인)
            eprintln!("심볼릭 링크를 만들 수 없어 건너뜀: {e}");
            return;
        }
        let mut tree = FsTree::open(root, DEFAULT_HIDDEN).unwrap();
        let a = child(&tree, tree.root(), "a");
        tree.expand(a);
        let back = child(&tree, a, "back");
        assert!(tree.node(back).symlink);
        assert!(tree.node(back).is_dir());
        tree.expand(back);
        assert!(tree.node(back).error.as_deref().unwrap().contains("순환"));
        assert!(tree.children_of(back).is_empty());
    }

    #[test]
    fn refresh_keeps_expanded_folders_and_finds_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("src/main.rs"));
        touch(&root.join("src/ui/editor.rs"));
        let mut tree = FsTree::open(root, DEFAULT_HIDDEN).unwrap();
        let src = child(&tree, tree.root(), "src");
        tree.expand(src);
        let ui = child(&tree, src, "ui");
        tree.expand(ui);

        touch(&root.join("src/ui/sidebar.rs"));
        touch(&root.join("new.txt"));
        tree.refresh().unwrap();
        assert_eq!(
            names(&tree, &tree.visible_rows()),
            ["src", "ui", "editor.rs", "sidebar.rs", "main.rs", "new.txt"]
        );
    }

    /// 10,000개 파일 폴더의 1단계 읽기 시간 (WORKPLAN P2-05 🤖, 기록용 출력)
    #[test]
    fn opens_folder_with_10000_files_quickly() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10_000 {
            fs::write(dir.path().join(format!("file-{i:05}.txt")), b"").unwrap();
        }
        let started = Instant::now();
        let tree = FsTree::open(dir.path(), DEFAULT_HIDDEN).unwrap();
        let elapsed = started.elapsed();
        println!(
            "10,000개 파일 1단계 읽기: {:.1}ms",
            elapsed.as_secs_f64() * 1000.0
        );
        assert_eq!(tree.visible_rows().len(), 10_000);
        assert_eq!(tree.node(tree.visible_rows()[0]).name, "file-00000.txt");
        assert!(elapsed.as_secs_f64() < 2.0, "{elapsed:?}");
    }
}
