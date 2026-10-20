//! 프로토타입용 가짜 문서와 폴더 트리.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Source,
    Split,
    Preview,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Source => "소스",
            ViewMode::Split => "분할",
            ViewMode::Preview => "미리보기",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EncodingState {
    Normal,
    /// 자동 감지가 추정한 결과
    Guessed,
    /// 디코딩 중 잘못된 바이트가 있었음
    Damaged,
}

pub struct Doc {
    pub title: String,
    pub path: Option<String>,
    pub text: String,
    pub modified: bool,
    pub encoding: &'static str,
    pub encoding_state: EncodingState,
    pub line_ending: &'static str,
    pub mixed_line_endings: bool,
    pub is_markdown: bool,
    pub view_mode: ViewMode,
}

impl Doc {
    fn new(title: &str, path: Option<&str>, text: &str) -> Self {
        Self {
            title: title.to_owned(),
            path: path.map(str::to_owned),
            text: text.to_owned(),
            modified: false,
            encoding: "UTF-8",
            encoding_state: EncodingState::Normal,
            line_ending: "LF",
            mixed_line_endings: false,
            is_markdown: title.ends_with(".md"),
            view_mode: if title.ends_with(".md") {
                ViewMode::Split
            } else {
                ViewMode::Source
            },
        }
    }

    pub fn untitled(n: usize) -> Self {
        let mut doc = Self::new(&format!("Untitled-{n}"), None, "");
        doc.line_ending = if cfg!(windows) { "CRLF" } else { "LF" };
        doc
    }

    pub fn placeholder(name: &str, path: &str) -> Self {
        Self::new(
            name,
            Some(path),
            &format!("(프로토타입) \"{name}\"의 내용 예시입니다.\n실제 파일은 읽지 않습니다.\n"),
        )
    }
}

pub fn sample_docs() -> Vec<Doc> {
    let mut readme = Doc::new("README.md", Some(r"C:\devel\my-notepad\README.md"), README);
    readme.view_mode = ViewMode::Split;

    let mut minutes = Doc::new(
        "회의록.txt",
        Some(r"C:\Users\me\Documents\회의록.txt"),
        MINUTES,
    );
    minutes.modified = true;
    minutes.line_ending = "CRLF";

    let mut cp949 = Doc::new(
        "cp949_sample.txt",
        Some(r"C:\devel\my-notepad\tests\fixtures\encoding\cp949_sample.txt"),
        CP949,
    );
    cp949.encoding = "EUC-KR (CP949)";
    cp949.encoding_state = EncodingState::Guessed;
    cp949.line_ending = "CRLF";

    let mut log = Doc::new("server.log", Some(r"D:\logs\server.log"), SERVER_LOG);
    log.line_ending = "CRLF";
    log.mixed_line_endings = true;

    let mut broken = Doc::new(
        "broken.txt",
        Some(r"C:\devel\my-notepad\tests\fixtures\encoding\broken.txt"),
        BROKEN,
    );
    broken.encoding = "EUC-KR (CP949)";
    broken.encoding_state = EncodingState::Damaged;

    vec![readme, minutes, cp949, log, broken, Doc::untitled(1)]
}

pub fn many_extra_docs() -> Vec<Doc> {
    (1..=12)
        .map(|i| {
            Doc::placeholder(
                &format!("note-{i:02}.txt"),
                &format!(r"C:\Users\me\notes\note-{i:02}.txt"),
            )
        })
        .collect()
}

pub struct TreeNode {
    pub name: &'static str,
    pub path: &'static str,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    pub fn is_dir(&self) -> bool {
        !self.children.is_empty()
    }
}

fn file(name: &'static str, path: &'static str) -> TreeNode {
    TreeNode {
        name,
        path,
        children: Vec::new(),
    }
}

fn dir(name: &'static str, path: &'static str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode {
        name,
        path,
        children,
    }
}

/// 실제 저장소 구조를 흉내 낸 예시 트리 (`target`은 .gitignore 대상).
pub fn sample_tree() -> TreeNode {
    const R: &str = r"C:\devel\my-notepad";
    dir(
        "my-notepad",
        R,
        vec![
            dir(
                ".cargo",
                r"C:\devel\my-notepad\.cargo",
                vec![file(
                    "config.toml",
                    r"C:\devel\my-notepad\.cargo\config.toml",
                )],
            ),
            dir(
                "docs",
                r"C:\devel\my-notepad\docs",
                vec![
                    dir(
                        "ux",
                        r"C:\devel\my-notepad\docs\ux",
                        vec![
                            dir(
                                "screens",
                                r"C:\devel\my-notepad\docs\ux\screens",
                                vec![file(
                                    "p0-ux1-markdown-split.png",
                                    r"C:\devel\my-notepad\docs\ux\screens\p0-ux1-markdown-split.png",
                                )],
                            ),
                            file("ui-spec.md", r"C:\devel\my-notepad\docs\ux\ui-spec.md"),
                        ],
                    ),
                    file("PLAN.md", r"C:\devel\my-notepad\docs\PLAN.md"),
                    file("WORKPLAN.md", r"C:\devel\my-notepad\docs\WORKPLAN.md"),
                ],
            ),
            dir(
                "src",
                r"C:\devel\my-notepad\src",
                vec![
                    file("main.rs", r"C:\devel\my-notepad\src\main.rs"),
                    file("app.rs", r"C:\devel\my-notepad\src\app.rs"),
                ],
            ),
            dir(
                "target",
                r"C:\devel\my-notepad\target",
                vec![dir(
                    "release",
                    r"C:\devel\my-notepad\target\release",
                    vec![
                        file(
                            "my-notepad.exe",
                            r"C:\devel\my-notepad\target\release\my-notepad.exe",
                        ),
                        file(
                            "my_notepad.pdb",
                            r"C:\devel\my-notepad\target\release\my_notepad.pdb",
                        ),
                        file(
                            ".rustc_info.json",
                            r"C:\devel\my-notepad\target\.rustc_info.json",
                        ),
                    ],
                )],
            ),
            dir(
                "tests",
                r"C:\devel\my-notepad\tests",
                vec![dir(
                    "fixtures",
                    r"C:\devel\my-notepad\tests\fixtures",
                    vec![
                        file(
                            "cp949_sample.txt",
                            r"C:\devel\my-notepad\tests\fixtures\encoding\cp949_sample.txt",
                        ),
                        file(
                            "broken.txt",
                            r"C:\devel\my-notepad\tests\fixtures\encoding\broken.txt",
                        ),
                        file(
                            "server.log",
                            r"C:\devel\my-notepad\tests\fixtures\server.log",
                        ),
                    ],
                )],
            ),
            dir(
                "xtask",
                r"C:\devel\my-notepad\xtask",
                vec![file("Cargo.toml", r"C:\devel\my-notepad\xtask\Cargo.toml")],
            ),
            file(".editorconfig", r"C:\devel\my-notepad\.editorconfig"),
            file(".gitattributes", r"C:\devel\my-notepad\.gitattributes"),
            file(".gitignore", r"C:\devel\my-notepad\.gitignore"),
            file("Cargo.lock", r"C:\devel\my-notepad\Cargo.lock"),
            file("Cargo.toml", r"C:\devel\my-notepad\Cargo.toml"),
            file("README.md", r"C:\devel\my-notepad\README.md"),
            file(
                "rust-toolchain.toml",
                r"C:\devel\my-notepad\rust-toolchain.toml",
            ),
        ],
    )
}

/// 프로토타입용 .gitignore 판정 (실제 구현은 `ignore` 크레이트, P2-05).
pub fn is_ignored(node: &TreeNode) -> bool {
    node.name == "target"
}

/// 처음에 펼쳐 둘 폴더
pub fn initially_open_dirs() -> std::collections::HashSet<&'static str> {
    [
        r"C:\devel\my-notepad",
        r"C:\devel\my-notepad\.cargo",
        r"C:\devel\my-notepad\docs",
        r"C:\devel\my-notepad\docs\ux",
        r"C:\devel\my-notepad\target",
    ]
    .into_iter()
    .collect()
}

const README: &str = "\
# My-Notepad

가볍고 빠른 **Notepad++ 스타일** 텍스트 에디터입니다.

## 주요 기능

- 멀티 탭과 폴더 탐색 사이드바
- 한글 인코딩 자동 감지 (UTF-8, EUC-KR)
- Markdown 소스 보기 / 미리보기 / 분할 보기
- 정규식 찾기와 바꾸기

## 빠른 시작

```
cargo xtask try prototype
```

## 단축키

| 기능 | Windows | macOS |
| :--- | :--- | :--- |
| 찾기 | Ctrl+F | Cmd+F |
| 미리보기 전환 | Ctrl+Shift+V | Cmd+Shift+V |

> 이 문서는 프로토타입용 예시입니다. 왼쪽 소스를 고치면 오른쪽 미리보기에 반영됩니다.
";

const MINUTES: &str = "\
2026-10-20 주간 회의록

참석: 기획, 개발, 디자인

1. 인코딩 자동 감지 결과 공유
   - EUC-KR 파일 감지율 개선 필요
   - 인코딩 변환 저장 시 경고 문구 검토
2. Markdown 미리보기 일정
   - 분할 보기 스크롤 동기화 우선
3. 다음 회의: 2026-10-27

회의 분위기: 좋음 👍 (이모지는 EUC-KR로 저장할 수 없습니다 — 인코딩 → 변환 저장 → EUC-KR로 확인)

(이 탭은 수정된 상태입니다 — 탭을 닫아 저장 확인 대화상자를 확인해 보세요)
";

const CP949: &str = "\
메모장에서 ANSI(CP949)로 저장한 파일 예시입니다.
상태 표시줄의 인코딩이 'EUC-KR (CP949) · 추정됨'으로 표시됩니다.
인코딩을 클릭해 '다시 읽기'와 '변환 저장'을 확인해 보세요.
";

const SERVER_LOG: &str = "\
2026-10-20 09:00:01 INFO  server started on port 8080
2026-10-20 09:00:02 INFO  loading config from config.toml
2026-10-20 09:01:15 WARN  slow request: GET /api/files (1203 ms)
2026-10-20 09:02:40 ERROR failed to read file: encoding error at byte 128
2026-10-20 09:02:41 INFO  retrying with encoding EUC-KR
2026-10-20 09:02:41 INFO  file loaded: 회의록.txt
2026-10-20 09:05:00 ERROR connection reset by peer
2026-10-20 09:05:01 INFO  reconnected
2026-10-20 09:10:12 WARN  encoding guess low confidence: 0.42
2026-10-20 09:15:30 ERROR disk full while saving backup

(단어 'ERROR'나 'encoding'을 더블클릭하면 같은 단어가 모두 강조됩니다.
이 파일은 CRLF와 LF가 섞여 있어 상태 표시줄에 Mixed가 표시됩니다.)
";

const BROKEN: &str = "\
이 파일은 일부 바이트가 깨져 있습니다: 정상 문장 \u{FFFD}\u{FFFD} 다음 문장
상태 표시줄에 '손상'이 표시되고, 저장하면 원본 바이트가 사라진다는 경고가 나옵니다.
";
