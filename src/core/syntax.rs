//! 코드 문법 강조 — 자체 경량 토크나이저 (PLAN.md v0.7 §5 Phase 6, P6-A).
//!
//! - 외부 문법 엔진(`syntect`·`tree-sitter`) 없이 **주석·문자열·숫자·키워드·키(이름)·구분 기호**만 칠한다.
//!   의미 분석(함수·변수·타입 구분)과 한 파일 안의 언어 섞임(HTML 안 `<script>` 등)은 하지 않는다.
//! - 언어는 데이터 표([`Def`])로 정의한다: 주석 기호, 문자열 구분자, 키워드 목록, 키 규칙 등. 언어 추가 = 표 한 줄.
//!   로그·diff만 줄 단위 전용 규칙([`log_spans`]·[`diff_spans`])을 쓴다.
//! - 문서 처음부터 한 번 훑는다. 여러 줄 주석·문자열만 다음 줄로 이어진다. 결과는 바이트 범위
//!   (오름차순·겹치지 않음)이고, 경계는 모두 ASCII 구분자 위치나 줄 끝이라 항상 글자 경계다.
//! - 언어 판정은 파일 이름·확장자로만 한다 ([`detect`]). `.md`는 Markdown 구문 색상(P3-10)이 따로 맡는다.

use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

/// 칠하는 토큰 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Token {
    Comment,
    String,
    Number,
    Keyword,
    /// 설정 키·속성 이름·변수 (`name:`·`key =`·`$var`·nginx 지시어)
    Key,
    /// 태그 기호 (`<`·`</`·`>`)
    Punct,
    /// 로그 오류 수준 (`ERROR`·`FATAL`…)
    Error,
    /// 로그 경고 수준 (`WARN`…)
    Warning,
    /// diff 추가 줄 (`+`)
    Added,
    /// diff 삭제 줄 (`-`)
    Removed,
}

/// 토큰 구간: 바이트 범위 (오름차순, 겹치지 않음).
pub type TokenSpans = Vec<(Range<usize>, Token)>;

/// 문법 강조 언어.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Json,
    Yaml,
    Toml,
    Conf,
    Xml,
    Html,
    Css,
    JavaScript,
    TypeScript,
    Shell,
    PowerShell,
    Batch,
    Python,
    Sql,
    Dockerfile,
    Makefile,
    Rust,
    C,
    CSharp,
    Java,
    Kotlin,
    Go,
    Log,
    Diff,
}

impl Language {
    /// 메뉴 순서 (PLAN.md 대상 언어 표의 묶음 순서).
    pub const ALL: [Self; 24] = [
        Self::Json,
        Self::Yaml,
        Self::Toml,
        Self::Conf,
        Self::Xml,
        Self::Html,
        Self::Css,
        Self::JavaScript,
        Self::TypeScript,
        Self::Shell,
        Self::PowerShell,
        Self::Batch,
        Self::Python,
        Self::Sql,
        Self::Dockerfile,
        Self::Makefile,
        Self::Rust,
        Self::C,
        Self::CSharp,
        Self::Java,
        Self::Kotlin,
        Self::Go,
        Self::Log,
        Self::Diff,
    ];

    /// 상태 표시줄·메뉴에 보이는 이름.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Conf => "설정 (INI·conf)",
            Self::Xml => "XML",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Shell => "Shell",
            Self::PowerShell => "PowerShell",
            Self::Batch => "Batch",
            Self::Python => "Python",
            Self::Sql => "SQL",
            Self::Dockerfile => "Dockerfile",
            Self::Makefile => "Makefile",
            Self::Rust => "Rust",
            Self::C => "C/C++",
            Self::CSharp => "C#",
            Self::Java => "Java",
            Self::Kotlin => "Kotlin",
            Self::Go => "Go",
            Self::Log => "로그",
            Self::Diff => "Diff",
        }
    }
}

/// 탭의 문법 강조 선택 (`보기 → 문법 강조`). 파일마다 기억하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SyntaxChoice {
    /// 파일 이름·확장자로 자동 판정
    #[default]
    Auto,
    /// 칠하지 않음
    Off,
    /// 이 언어로
    Lang(Language),
}

/// 파일 이름·확장자로 언어를 고른다. 모르면 `None` (일반 텍스트).
pub fn detect(path: &Path) -> Option<Language> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let by_name = match name.as_str() {
        "dockerfile" | "containerfile" => Some(Language::Dockerfile),
        "makefile" | "gnumakefile" => Some(Language::Makefile),
        ".gitignore" | ".gitattributes" | ".editorconfig" | ".npmrc" | ".env" => {
            Some(Language::Conf)
        }
        _ if name.starts_with("dockerfile.") || name.ends_with(".dockerfile") => {
            Some(Language::Dockerfile)
        }
        _ if name.starts_with(".env.") => Some(Language::Conf),
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    let ext = name.rsplit_once('.')?.1;
    Some(match ext {
        "json" | "jsonc" | "json5" => Language::Json,
        "yml" | "yaml" => Language::Yaml,
        "toml" => Language::Toml,
        "ini" | "cfg" | "conf" | "properties" | "env" | "editorconfig" => Language::Conf,
        "xml" | "csproj" | "vbproj" | "props" | "targets" | "svg" | "xaml" | "plist" => {
            Language::Xml
        }
        "html" | "htm" | "xhtml" => Language::Html,
        "css" | "scss" | "less" => Language::Css,
        "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
        "ts" | "mts" | "cts" | "tsx" => Language::TypeScript,
        "sh" | "bash" | "zsh" => Language::Shell,
        "ps1" | "psm1" | "psd1" => Language::PowerShell,
        "bat" | "cmd" => Language::Batch,
        "py" | "pyw" => Language::Python,
        "sql" => Language::Sql,
        "mk" => Language::Makefile,
        "rs" => Language::Rust,
        "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx" | "ino" => Language::C,
        "cs" => Language::CSharp,
        "java" => Language::Java,
        "kt" | "kts" => Language::Kotlin,
        "go" => Language::Go,
        "log" => Language::Log,
        "diff" | "patch" => Language::Diff,
        _ => return None,
    })
}

/// 문자열 구분자.
#[derive(Debug, Clone, Copy)]
struct Str {
    open: &'static str,
    close: &'static str,
    /// `\`로 다음 글자를 건너뛴다
    escape: bool,
    /// 줄을 넘어 이어진다 (닫지 않으면 줄 끝에서 끝나지 않음)
    multiline: bool,
}

const fn s(open: &'static str, close: &'static str, escape: bool, multiline: bool) -> Str {
    Str {
        open,
        close,
        escape,
        multiline,
    }
}

const DQ: Str = s("\"", "\"", true, false);
const SQ: Str = s("'", "'", true, false);
const SQ_RAW: Str = s("'", "'", false, false);
const BACKTICK: Str = s("`", "`", true, true);
const DQ_RAW: Str = s("\"", "\"", false, false);
const BACKTICK_RAW: Str = s("`", "`", false, true);
const TRIPLE_DQ: Str = s("\"\"\"", "\"\"\"", true, true);
const TRIPLE_DQ_RAW: Str = s("\"\"\"", "\"\"\"", false, true);
const TRIPLE_SQ: Str = s("'''", "'''", true, true);
const TRIPLE_SQ_RAW: Str = s("'''", "'''", false, true);
const PS_HERE_DQ: Str = s("@\"", "\"@", false, true);
const PS_HERE_SQ: Str = s("@'", "'@", false, true);

/// 키(이름)를 찾는 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyRule {
    None,
    /// JSON: `:` 앞의 문자열
    StringBeforeColon,
    /// YAML: 줄 앞 `key:` (`- key:` 포함)
    LineBeforeColon,
    /// TOML·INI: 줄 앞 `key =`
    BeforeEquals,
    /// conf: `key = …`이면 `=` 앞, 아니면 줄의 첫 낱말 (nginx 지시어)
    Conf,
    /// Makefile: 줄 앞 `대상:`·`변수 =`
    Make,
    /// CSS: `{}` 안의 `속성:`
    CssProperty,
}

/// 언어 정의 (데이터 표 한 줄).
#[derive(Debug)]
struct Def {
    /// 어디서든 시작하는 줄 주석
    line_comments: &'static [&'static str],
    /// 줄 처음(공백 뒤)에서만 시작하는 줄 주석
    start_comments: &'static [&'static str],
    block_comments: &'static [(&'static str, &'static str)],
    /// 긴 구분자를 먼저 (`"""`가 `"`보다 앞)
    strings: &'static [Str],
    /// 키워드 (`ignore_case`면 소문자로 적는다)
    keywords: &'static [&'static str],
    ignore_case: bool,
    key: KeyRule,
    /// `$name`·`${name}`·`$(name)` 변수 → 키
    dollar_vars: bool,
    /// `%name%` 변수 → 키 (Batch)
    percent_vars: bool,
    /// `<태그>` → 구분 기호·키워드, 속성 이름 → 키
    tags: bool,
    /// `[section]` 줄 → 키워드
    sections: bool,
    /// 줄 처음 `#낱말` → 키워드 (C 전처리기)
    preprocessor: bool,
    /// `'`는 짧은 글자 상수일 때만 문자열 (Rust 수명 `'a` 구분)
    char_literal: bool,
    /// 낱말에 들어가는 추가 글자 (CSS·YAML 키의 `-` 등)
    word_extra: &'static [u8],
}

const BASE: Def = Def {
    line_comments: &[],
    start_comments: &[],
    block_comments: &[],
    strings: &[],
    keywords: &[],
    ignore_case: false,
    key: KeyRule::None,
    dollar_vars: false,
    percent_vars: false,
    tags: false,
    sections: false,
    preprocessor: false,
    char_literal: false,
    word_extra: &[],
};

const C_COMMENTS: &[(&str, &str)] = &[("/*", "*/")];

const JS_KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];
const TS_KEYWORDS: &[&str] = &[
    "abstract",
    "any",
    "as",
    "async",
    "await",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "namespace",
    "never",
    "new",
    "null",
    "number",
    "object",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "set",
    "static",
    "string",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unknown",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

const fn def(lang: Language) -> Def {
    use Language as L;
    match lang {
        L::Json => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[DQ],
            keywords: &["false", "null", "true"],
            key: KeyRule::StringBeforeColon,
            ..BASE
        },
        L::Yaml => Def {
            line_comments: &["#"],
            strings: &[DQ, SQ_RAW],
            keywords: &["false", "no", "null", "off", "on", "true", "yes", "~"],
            key: KeyRule::LineBeforeColon,
            word_extra: b"-",
            ..BASE
        },
        L::Toml => Def {
            line_comments: &["#"],
            strings: &[TRIPLE_DQ, TRIPLE_SQ_RAW, DQ, SQ_RAW],
            keywords: &["false", "inf", "nan", "true"],
            key: KeyRule::BeforeEquals,
            sections: true,
            ..BASE
        },
        L::Conf => Def {
            line_comments: &["#"],
            start_comments: &[";"],
            strings: &[DQ, SQ_RAW],
            keywords: &["false", "no", "off", "on", "true", "yes"],
            ignore_case: true,
            key: KeyRule::Conf,
            dollar_vars: true,
            sections: true,
            ..BASE
        },
        L::Xml | L::Html => Def {
            block_comments: &[("<!--", "-->"), ("<![CDATA[", "]]>")],
            strings: &[DQ, SQ_RAW],
            tags: true,
            word_extra: b"-:",
            ..BASE
        },
        L::Css => Def {
            block_comments: C_COMMENTS,
            line_comments: &["//"],
            strings: &[DQ, SQ],
            keywords: &["!important", "auto", "inherit", "initial", "none", "unset"],
            key: KeyRule::CssProperty,
            dollar_vars: true,
            word_extra: b"-",
            ..BASE
        },
        L::JavaScript => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[DQ, SQ, BACKTICK],
            keywords: JS_KEYWORDS,
            tags: true,
            ..BASE
        },
        L::TypeScript => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[DQ, SQ, BACKTICK],
            keywords: TS_KEYWORDS,
            tags: true,
            ..BASE
        },
        L::Shell => Def {
            line_comments: &["#"],
            strings: &[DQ, SQ_RAW, BACKTICK],
            keywords: &[
                "case", "do", "done", "elif", "else", "esac", "exit", "export", "fi", "for",
                "function", "if", "in", "local", "readonly", "return", "select", "set", "shift",
                "source", "then", "until", "while",
            ],
            dollar_vars: true,
            ..BASE
        },
        L::PowerShell => Def {
            line_comments: &["#"],
            block_comments: &[("<#", "#>")],
            strings: &[PS_HERE_DQ, PS_HERE_SQ, DQ_RAW, SQ_RAW],
            keywords: &[
                "begin", "break", "catch", "class", "continue", "do", "else", "elseif", "end",
                "enum", "exit", "filter", "finally", "for", "foreach", "function", "if", "in",
                "param", "process", "return", "switch", "throw", "trap", "try", "until", "using",
                "while",
            ],
            ignore_case: true,
            dollar_vars: true,
            word_extra: b"-",
            ..BASE
        },
        L::Batch => Def {
            start_comments: &["rem ", "::", "@rem "],
            strings: &[DQ_RAW],
            keywords: &[
                "call",
                "cd",
                "defined",
                "do",
                "echo",
                "else",
                "endlocal",
                "equ",
                "errorlevel",
                "exist",
                "exit",
                "for",
                "geq",
                "goto",
                "gtr",
                "if",
                "in",
                "leq",
                "lss",
                "neq",
                "not",
                "off",
                "on",
                "pause",
                "set",
                "setlocal",
                "shift",
                "start",
            ],
            ignore_case: true,
            percent_vars: true,
            ..BASE
        },
        L::Python => Def {
            line_comments: &["#"],
            strings: &[TRIPLE_DQ, TRIPLE_SQ, DQ, SQ],
            keywords: &[
                "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
                "continue", "def", "del", "elif", "else", "except", "finally", "for", "from",
                "global", "if", "import", "in", "is", "lambda", "match", "nonlocal", "not", "or",
                "pass", "raise", "return", "self", "try", "while", "with", "yield",
            ],
            ..BASE
        },
        L::Sql => Def {
            line_comments: &["--"],
            block_comments: C_COMMENTS,
            strings: &[SQ_RAW, DQ_RAW],
            keywords: &[
                "add",
                "all",
                "alter",
                "and",
                "as",
                "asc",
                "begin",
                "between",
                "by",
                "case",
                "commit",
                "constraint",
                "create",
                "cross",
                "database",
                "default",
                "delete",
                "desc",
                "distinct",
                "drop",
                "else",
                "end",
                "exists",
                "false",
                "foreign",
                "from",
                "full",
                "group",
                "having",
                "if",
                "in",
                "index",
                "inner",
                "insert",
                "into",
                "is",
                "join",
                "key",
                "left",
                "like",
                "limit",
                "not",
                "null",
                "offset",
                "on",
                "or",
                "order",
                "outer",
                "primary",
                "references",
                "right",
                "rollback",
                "select",
                "set",
                "table",
                "then",
                "true",
                "union",
                "unique",
                "update",
                "values",
                "view",
                "when",
                "where",
                "with",
            ],
            ignore_case: true,
            ..BASE
        },
        L::Dockerfile => Def {
            line_comments: &["#"],
            strings: &[DQ, SQ_RAW],
            keywords: &[
                "add",
                "arg",
                "as",
                "cmd",
                "copy",
                "entrypoint",
                "env",
                "expose",
                "from",
                "healthcheck",
                "label",
                "maintainer",
                "onbuild",
                "run",
                "shell",
                "stopsignal",
                "user",
                "volume",
                "workdir",
            ],
            ignore_case: true,
            dollar_vars: true,
            ..BASE
        },
        L::Makefile => Def {
            line_comments: &["#"],
            strings: &[DQ, SQ_RAW],
            keywords: &[
                "define", "else", "endef", "endif", "export", "ifdef", "ifeq", "ifndef", "ifneq",
                "include", "override", "unexport",
            ],
            key: KeyRule::Make,
            dollar_vars: true,
            ..BASE
        },
        L::Rust => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[DQ, SQ],
            keywords: &[
                "Self", "as", "async", "await", "break", "const", "continue", "crate", "dyn",
                "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop",
                "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct",
                "super", "trait", "true", "type", "unsafe", "use", "where", "while",
            ],
            char_literal: true,
            ..BASE
        },
        L::C => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[DQ, SQ],
            keywords: &[
                "auto",
                "bool",
                "break",
                "case",
                "catch",
                "char",
                "class",
                "const",
                "constexpr",
                "continue",
                "default",
                "delete",
                "do",
                "double",
                "else",
                "enum",
                "explicit",
                "extern",
                "false",
                "float",
                "for",
                "friend",
                "goto",
                "if",
                "inline",
                "int",
                "long",
                "namespace",
                "new",
                "noexcept",
                "nullptr",
                "operator",
                "override",
                "private",
                "protected",
                "public",
                "return",
                "short",
                "signed",
                "sizeof",
                "static",
                "struct",
                "switch",
                "template",
                "this",
                "throw",
                "true",
                "try",
                "typedef",
                "typename",
                "union",
                "unsigned",
                "using",
                "virtual",
                "void",
                "volatile",
                "while",
            ],
            preprocessor: true,
            ..BASE
        },
        L::CSharp => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[DQ, SQ],
            keywords: &[
                "abstract",
                "as",
                "async",
                "await",
                "base",
                "bool",
                "break",
                "byte",
                "case",
                "catch",
                "char",
                "class",
                "const",
                "continue",
                "decimal",
                "default",
                "do",
                "double",
                "else",
                "enum",
                "event",
                "false",
                "finally",
                "float",
                "for",
                "foreach",
                "get",
                "if",
                "in",
                "init",
                "int",
                "interface",
                "internal",
                "is",
                "long",
                "namespace",
                "new",
                "null",
                "object",
                "out",
                "override",
                "private",
                "protected",
                "public",
                "readonly",
                "record",
                "ref",
                "return",
                "sealed",
                "set",
                "static",
                "string",
                "struct",
                "switch",
                "this",
                "throw",
                "true",
                "try",
                "typeof",
                "using",
                "var",
                "virtual",
                "void",
                "while",
                "yield",
            ],
            preprocessor: true,
            ..BASE
        },
        L::Java => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[TRIPLE_DQ, DQ, SQ],
            keywords: &[
                "abstract",
                "boolean",
                "break",
                "byte",
                "case",
                "catch",
                "char",
                "class",
                "const",
                "continue",
                "default",
                "do",
                "double",
                "else",
                "enum",
                "extends",
                "false",
                "final",
                "finally",
                "float",
                "for",
                "if",
                "implements",
                "import",
                "instanceof",
                "int",
                "interface",
                "long",
                "new",
                "null",
                "package",
                "private",
                "protected",
                "public",
                "record",
                "return",
                "short",
                "static",
                "super",
                "switch",
                "this",
                "throw",
                "throws",
                "true",
                "try",
                "var",
                "void",
                "while",
            ],
            ..BASE
        },
        L::Kotlin => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[TRIPLE_DQ_RAW, DQ, SQ],
            keywords: &[
                "as",
                "break",
                "class",
                "companion",
                "const",
                "continue",
                "data",
                "do",
                "else",
                "enum",
                "false",
                "for",
                "fun",
                "if",
                "import",
                "in",
                "interface",
                "is",
                "lateinit",
                "null",
                "object",
                "open",
                "override",
                "package",
                "private",
                "protected",
                "public",
                "return",
                "sealed",
                "super",
                "this",
                "throw",
                "true",
                "try",
                "typealias",
                "val",
                "var",
                "when",
                "while",
            ],
            ..BASE
        },
        L::Go => Def {
            line_comments: &["//"],
            block_comments: C_COMMENTS,
            strings: &[DQ, SQ, BACKTICK_RAW],
            keywords: &[
                "break",
                "case",
                "chan",
                "const",
                "continue",
                "default",
                "defer",
                "else",
                "fallthrough",
                "false",
                "for",
                "func",
                "go",
                "goto",
                "if",
                "import",
                "interface",
                "iota",
                "map",
                "nil",
                "package",
                "range",
                "return",
                "select",
                "struct",
                "switch",
                "true",
                "type",
                "var",
            ],
            ..BASE
        },
        // 로그·diff는 줄 단위 전용 규칙 ([`log_spans`]·[`diff_spans`])
        L::Log | L::Diff => BASE,
    }
}

/// 언어마다 한 번 만드는 정렬된 키워드 목록 (이진 탐색 — 낱말마다 해시하는 것보다 빠르다).
fn keyword_list(lang: Language) -> &'static [&'static str] {
    static LISTS: OnceLock<Vec<Vec<&'static str>>> = OnceLock::new();
    let lists = LISTS.get_or_init(|| {
        Language::ALL
            .iter()
            .map(|&l| {
                let mut list = def(l).keywords.to_vec();
                list.sort_unstable();
                list.dedup();
                list
            })
            .collect()
    });
    let index = Language::ALL.iter().position(|&l| l == lang).unwrap_or(0);
    &lists[index]
}

/// 토큰이 시작할 수 있는 첫 바이트 표 — 나머지 바이트는 검사 없이 지나간다.
fn start_bytes(def: &Def) -> [bool; 256] {
    let mut table = [false; 256];
    for b in 0..=255u8 {
        table[b as usize] = b.is_ascii_alphanumeric() || matches!(b, b'_' | b'\n' | b'{' | b'}');
    }
    let mut mark = |pat: &str| {
        if let Some(&first) = pat.as_bytes().first() {
            table[first as usize] = true;
            table[first.to_ascii_lowercase() as usize] = true;
            table[first.to_ascii_uppercase() as usize] = true;
        }
    };
    def.line_comments.iter().for_each(|p| mark(p));
    def.block_comments.iter().for_each(|(p, _)| mark(p));
    def.strings.iter().for_each(|st| mark(st.open));
    if def.tags {
        mark("<");
        mark(">");
        mark("/");
        mark("?");
    }
    if def.dollar_vars {
        mark("$");
    }
    if def.percent_vars {
        mark("%");
    }
    if def.key == KeyRule::CssProperty {
        mark("!");
    }
    table
}

/// 텍스트의 토큰 구간을 계산한다 (문서 처음부터 한 번 훑기).
pub fn tokenize(lang: Language, text: &str) -> TokenSpans {
    match lang {
        Language::Log => log_spans(text),
        Language::Diff => diff_spans(text),
        _ => {
            let def = def(lang);
            Scanner {
                starts: start_bytes(&def),
                def,
                keywords: keyword_list(lang),
                text,
                bytes: text.as_bytes(),
                out: TokenSpans::new(),
            }
            .run()
        }
    }
}

fn is_word_byte(b: u8, extra: &[u8]) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || extra.contains(&b)
}

struct Scanner<'a> {
    def: Def,
    starts: [bool; 256],
    keywords: &'static [&'static str],
    text: &'a str,
    bytes: &'a [u8],
    out: TokenSpans,
}

impl Scanner<'_> {
    fn push(&mut self, range: Range<usize>, token: Token) {
        if range.start < range.end {
            self.out.push((range, token));
        }
    }

    fn at(&self, i: usize, pat: &str) -> bool {
        let p = pat.as_bytes();
        let Some(slice) = self.bytes.get(i..i + p.len()) else {
            return false;
        };
        if self.def.ignore_case {
            slice.eq_ignore_ascii_case(p)
        } else {
            slice == p
        }
    }

    fn line_end(&self, i: usize) -> usize {
        self.bytes[i..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(self.bytes.len(), |p| i + p)
    }

    fn skip_blank(&self, mut i: usize) -> usize {
        while i < self.bytes.len() && matches!(self.bytes[i], b' ' | b'\t') {
            i += 1;
        }
        i
    }

    fn word_end(&self, mut i: usize) -> usize {
        while i < self.bytes.len() && is_word_byte(self.bytes[i], self.def.word_extra) {
            i += 1;
        }
        i
    }

    fn is_keyword(&self, word: &str) -> bool {
        if self.keywords.is_empty() {
            return false;
        }
        if self.def.ignore_case {
            // 소문자로 바꿔 찾는다 (할당 없이 스택 버퍼에)
            let mut buf = [0u8; 32];
            let Some(dst) = buf.get_mut(..word.len()) else {
                return false;
            };
            dst.copy_from_slice(word.as_bytes());
            dst.make_ascii_lowercase();
            let lower = std::str::from_utf8(dst).unwrap_or_default();
            self.keywords.binary_search(&lower).is_ok()
        } else {
            self.keywords.binary_search(&word).is_ok()
        }
    }

    fn run(mut self) -> TokenSpans {
        let n = self.bytes.len();
        let mut i = 0;
        let mut line_start = true;
        let mut depth = 0_i32;
        let mut in_tag = false;
        while i < n {
            if line_start {
                line_start = false;
                if let Some(next) = self.line_rules(i) {
                    i = next;
                    continue;
                }
            }
            let c = self.bytes[i];
            if !self.starts[c as usize] {
                i += 1;
                continue;
            }
            if c == b'\n' {
                line_start = true;
                i += 1;
                continue;
            }
            if let Some(end) = self.block_comment(i) {
                self.push(i..end, Token::Comment);
                i = end;
                continue;
            }
            if self.line_comment(i) {
                let end = self.line_end(i);
                self.push(i..end, Token::Comment);
                i = end;
                continue;
            }
            if let Some(end) = self.string(i) {
                let token = if self.def.key == KeyRule::StringBeforeColon
                    && self.bytes.get(self.skip_blank(end)) == Some(&b':')
                {
                    Token::Key
                } else {
                    Token::String
                };
                self.push(i..end, token);
                i = end;
                continue;
            }
            if self.def.tags {
                if let Some(end) = self.tag_open(i) {
                    in_tag = true;
                    i = end;
                    continue;
                }
                if in_tag && (c == b'>' || self.at(i, "/>") || self.at(i, "?>")) {
                    let end = if c == b'>' { i + 1 } else { i + 2 };
                    self.push(i..end, Token::Punct);
                    in_tag = false;
                    i = end;
                    continue;
                }
            }
            if let Some(end) = self.variable(i) {
                self.push(i..end, Token::Key);
                i = end;
                continue;
            }
            let prev_word = i > 0 && is_word_byte(self.bytes[i - 1], self.def.word_extra);
            if c.is_ascii_digit() && !prev_word {
                let mut end = i + 1;
                while end < n
                    && (self.bytes[end].is_ascii_alphanumeric() || b"_.".contains(&self.bytes[end]))
                {
                    end += 1;
                }
                self.push(i..end, Token::Number);
                i = end;
                continue;
            }
            if (c.is_ascii_alphabetic()
                || c == b'_'
                || (c == b'!' && self.def.key == KeyRule::CssProperty))
                && !prev_word
            {
                let end = self.word_end(i + 1);
                let next = self.bytes.get(self.skip_blank(end)).copied();
                let word = &self.text[i..end];
                if (in_tag && next == Some(b'='))
                    || (self.def.key == KeyRule::CssProperty && depth > 0 && next == Some(b':'))
                {
                    self.push(i..end, Token::Key);
                } else if self.is_keyword(word) {
                    self.push(i..end, Token::Keyword);
                }
                i = end;
                continue;
            }
            match c {
                b'{' => depth += 1,
                b'}' => depth = (depth - 1).max(0),
                _ => {}
            }
            i += 1;
        }
        self.out
    }

    /// 줄 처음 규칙 (줄 주석·섹션·전처리기·키). 처리했으면 다음 위치.
    fn line_rules(&mut self, line: usize) -> Option<usize> {
        let start = self.skip_blank(line);
        let end = self.line_end(line);
        if start >= end {
            return None;
        }
        for &prefix in self.def.start_comments {
            if self.at(start, prefix)
                || self.at(start, prefix.trim_end()) && start + prefix.trim_end().len() == end
            {
                self.push(start..end, Token::Comment);
                return Some(end);
            }
        }
        let first = self.bytes[start];
        if self.def.sections
            && first == b'['
            && let Some(close) = self.bytes[start..end].iter().position(|&b| b == b']')
        {
            self.push(start..start + close + 1, Token::Keyword);
            return Some(start + close + 1);
        }
        if self.def.preprocessor && first == b'#' {
            let word = self.word_end(self.skip_blank(start + 1));
            self.push(start..word, Token::Keyword);
            return Some(word);
        }
        let key_end = match self.def.key {
            KeyRule::LineBeforeColon => self.yaml_key(start, end),
            KeyRule::BeforeEquals => self.key_before_equals(start, end),
            KeyRule::Conf => self.key_before_equals(start, end).or_else(|| {
                let word = self.word_end(start);
                (word > start && first.is_ascii_alphabetic()).then_some(word)
            }),
            KeyRule::Make => self.key_before_equals(start, end).or_else(|| {
                // 대상 줄: 들여쓰기 없이 `이름:` (`:=`는 위에서 변수로)
                let colon = start + self.bytes[start..end].iter().position(|&b| b == b':')?;
                (start == line && colon > start && !self.at(colon, ":=")).then_some(colon)
            }),
            _ => None,
        };
        let key_end = key_end?;
        let key_start = if self.def.key == KeyRule::LineBeforeColon && self.at(start, "- ") {
            self.skip_blank(start + 2)
        } else {
            start
        };
        self.push(key_start..key_end, Token::Key);
        Some(key_end)
    }

    /// `key =`의 키 끝 (뒤 공백 제외). 주석·따옴표·괄호로 시작하면 키가 아니다.
    fn key_before_equals(&self, start: usize, end: usize) -> Option<usize> {
        let first = self.bytes[start];
        if matches!(
            first,
            b'#' | b';' | b'[' | b'"' | b'\'' | b'{' | b'}' | b'='
        ) {
            return None;
        }
        let line = &self.bytes[start..end];
        let eq = line.iter().position(|&b| b == b'=')?;
        let before = &line[..eq];
        if before
            .iter()
            .any(|&b| matches!(b, b'"' | b'\'' | b'#' | b'(' | b'{' | b'$'))
        {
            return None;
        }
        // `:=`·`?=`·`+=` (Makefile)도 기호 앞까지
        let mut key_end = start + eq;
        while key_end > start
            && matches!(
                self.bytes[key_end - 1],
                b' ' | b'\t' | b':' | b'?' | b'+' | b'!'
            )
        {
            key_end -= 1;
        }
        (key_end > start).then_some(key_end)
    }

    /// YAML `key:` (`- key:` 포함)의 키 끝.
    fn yaml_key(&self, start: usize, end: usize) -> Option<usize> {
        let mut i = if self.at(start, "- ") {
            self.skip_blank(start + 2)
        } else {
            start
        };
        let key_start = i;
        if i >= end
            || matches!(
                self.bytes[i],
                b'#' | b'"' | b'\'' | b'{' | b'[' | b'-' | b'&' | b'*' | b'|' | b'>'
            )
        {
            return None;
        }
        while i < end {
            match self.bytes[i] {
                b':' if i + 1 == end || matches!(self.bytes[i + 1], b' ' | b'\t' | b'\r') => {
                    let mut key_end = i;
                    while key_end > key_start && matches!(self.bytes[key_end - 1], b' ' | b'\t') {
                        key_end -= 1;
                    }
                    return (key_end > key_start).then_some(key_end);
                }
                b'#' => return None,
                _ => i += 1,
            }
        }
        None
    }

    fn block_comment(&self, i: usize) -> Option<usize> {
        let (open, close) = self
            .def
            .block_comments
            .iter()
            .find(|(open, _)| self.at(i, open))?;
        let from = i + open.len();
        Some(
            self.text[from..]
                .find(close)
                .map_or(self.bytes.len(), |p| from + p + close.len()),
        )
    }

    fn line_comment(&self, i: usize) -> bool {
        self.def.line_comments.iter().any(|&prefix| {
            self.at(i, prefix)
                // `#`은 낱말 안·`$#`에서는 주석이 아니다 (셸 `$#`, `a#b`)
                && !(prefix == "#"
                    && i > 0
                    && (self.bytes[i - 1].is_ascii_alphanumeric() || self.bytes[i - 1] == b'$'))
        })
    }

    fn string(&self, i: usize) -> Option<usize> {
        let n = self.bytes.len();
        for st in self.def.strings {
            if !self.at(i, st.open) {
                continue;
            }
            // Rust 수명 `'a`: 짧은 글자 상수(`'x'`·`'\n'`·`'\u{1F600}'`)만 문자열로
            if self.def.char_literal && st.open == "'" {
                let limit = (i + 12).min(n);
                let mut j = i + 1;
                let mut closed = None;
                while j < limit && self.bytes[j] != b'\n' {
                    if self.bytes[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if self.bytes[j] == b'\'' {
                        closed = Some(j + 1);
                        break;
                    }
                    j += 1;
                }
                // `'a'`처럼 닫혀도 안에 글자가 하나 이상이어야 한다
                return closed.filter(|&end| end > i + 2);
            }
            let mut j = i + st.open.len();
            while j < n {
                if st.escape && self.bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if self.at(j, st.close) {
                    return Some(j + st.close.len());
                }
                if self.bytes[j] == b'\n' && !st.multiline {
                    return Some(j);
                }
                j += 1;
            }
            return Some(n);
        }
        None
    }

    /// `<태그`·`</태그`·`<!DOCTYPE`·`<?xml` → 기호 + 태그 이름. 끝 위치.
    fn tag_open(&mut self, i: usize) -> Option<usize> {
        if self.bytes[i] != b'<' {
            return None;
        }
        let mut j = i + 1;
        if matches!(self.bytes.get(j), Some(b'/' | b'!' | b'?')) {
            j += 1;
        }
        if !self.bytes.get(j)?.is_ascii_alphabetic() {
            return None;
        }
        self.push(i..j, Token::Punct);
        let end = self.word_end(j);
        self.push(j..end, Token::Keyword);
        Some(end)
    }

    /// `$name`·`${…}`·`$(…)`·`%name%` 변수의 끝.
    fn variable(&self, i: usize) -> Option<usize> {
        let n = self.bytes.len();
        let c = self.bytes[i];
        if self.def.dollar_vars && c == b'$' && i + 1 < n {
            let close = match self.bytes[i + 1] {
                b'{' => Some(b'}'),
                b'(' => Some(b')'),
                _ => None,
            };
            if let Some(close) = close {
                let end = self.line_end(i);
                let p = self.bytes[i + 2..end].iter().position(|&b| b == close)?;
                return Some(i + 2 + p + 1);
            }
            let end = self.word_end(i + 1);
            return (end > i + 1).then_some(end);
        }
        if self.def.percent_vars && c == b'%' {
            let end = self.word_end(i + 1);
            if end > i + 1 && self.bytes.get(end) == Some(&b'%') {
                return Some(end + 1);
            }
            // `%%i`·`%1` (for 변수·인자)
            if matches!(self.bytes.get(i + 1), Some(b'%' | b'0'..=b'9' | b'~')) {
                return Some(self.word_end(i + 2).max(i + 2));
            }
        }
        None
    }
}

/// 로그: 날짜·시각·숫자 → 숫자, 수준 낱말(`ERROR`·`WARN`·`INFO`…) → 오류·경고·키워드, 따옴표 → 문자열.
fn log_spans(text: &str) -> TokenSpans {
    const ERRORS: &[&str] = &[
        "crit",
        "critical",
        "err",
        "error",
        "exception",
        "fail",
        "failed",
        "failure",
        "fatal",
        "panic",
        "panicked",
        "severe",
    ];
    const WARNINGS: &[&str] = &["warn", "warning"];
    const LEVELS: &[&str] = &["debug", "info", "notice", "trace", "verbose"];
    let b = text.as_bytes();
    let n = b.len();
    let mut out = TokenSpans::new();
    let mut i = 0;
    while i < n {
        let c = b[i];
        let prev_word = i > 0 && b[i - 1].is_ascii_alphanumeric();
        if c.is_ascii_digit() && !prev_word {
            let mut end = i + 1;
            while end < n
                && (b[end].is_ascii_digit()
                    || (matches!(
                        b[end],
                        b':' | b'-' | b'.' | b'/' | b'T' | b',' | b'Z' | b'+'
                    ) && b.get(end + 1).is_some_and(u8::is_ascii_digit)))
            {
                end += 1;
            }
            out.push((i..end, Token::Number));
            i = end;
        } else if c == b'"' {
            let end = b[i + 1..]
                .iter()
                .position(|&x| x == b'"' || x == b'\n')
                .map_or(n, |p| i + 1 + p + usize::from(b[i + 1 + p] == b'"'));
            out.push((i..end, Token::String));
            i = end;
        } else if c.is_ascii_alphabetic() && !prev_word {
            let mut end = i + 1;
            while end < n && b[end].is_ascii_alphanumeric() {
                end += 1;
            }
            let word = text[i..end].to_ascii_lowercase();
            let token = if ERRORS.contains(&word.as_str()) {
                Some(Token::Error)
            } else if WARNINGS.contains(&word.as_str()) {
                Some(Token::Warning)
            } else if LEVELS.contains(&word.as_str()) {
                Some(Token::Keyword)
            } else {
                None
            };
            if let Some(token) = token {
                out.push((i..end, token));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// diff·patch: 머리 줄(`diff`·`index`·`---`·`+++`) → 키워드, `@@` → 키, `+` → 추가, `-` → 삭제.
fn diff_spans(text: &str) -> TokenSpans {
    let mut out = TokenSpans::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.trim_end_matches(['\n', '\r']).len();
        let token = if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("index ")
        {
            Some(Token::Keyword)
        } else if line.starts_with("@@") {
            Some(Token::Key)
        } else if line.starts_with('+') {
            Some(Token::Added)
        } else if line.starts_with('-') {
            Some(Token::Removed)
        } else {
            None
        };
        if let Some(token) = token
            && start < end
        {
            out.push((start..end, token));
        }
        start += line.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// (조각, 토큰) 목록으로 본다 — 테스트 가독성용
    fn tokens(lang: Language, text: &str) -> Vec<(&str, Token)> {
        tokenize(lang, text)
            .into_iter()
            .map(|(r, t)| (&text[r], t))
            .collect()
    }

    fn has(list: &[(&str, Token)], piece: &str, token: Token) -> bool {
        list.contains(&(piece, token))
    }

    #[test]
    fn spans_are_sorted_non_overlapping_and_on_char_boundaries() {
        let samples = [
            (
                Language::Rust,
                "fn 한글() { let s = \"가\\\"나\"; // 주석\n}\n'a' 'ab",
            ),
            (Language::Json, "{\"키\": [1, true, \"값\"]} /* 끝"),
            (Language::Yaml, "이름: 값 # 주석\n- key: \"x\"\n"),
            (Language::Html, "<div class=\"가\">본문<!-- 주석 --></div><"),
            (Language::Python, "'''여러\n줄'''\nx = f\"{y}\" # 끝"),
            (Language::Log, "2026-10-27 10:00:01 ERROR 한글 \"따옴"),
            (Language::Diff, "+추가\n-삭제\n@@ -1 +1 @@\n"),
            (Language::Batch, "rem 주석\nset X=%PATH%\necho \"가"),
        ];
        for (lang, text) in samples {
            let spans = tokenize(lang, text);
            let mut pos = 0;
            for (range, _) in &spans {
                assert!(
                    range.start >= pos && range.start < range.end,
                    "{lang:?} {spans:?}"
                );
                assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
                pos = range.end;
            }
            assert!(pos <= text.len());
        }
    }

    #[test]
    fn c_like_languages() {
        let t = tokens(
            Language::Rust,
            "// 한 줄\nfn main() { let x = 0x1F; /* 블록\n주석 */ \"문자\\\"열\" 'c' &'a str }",
        );
        assert!(has(&t, "// 한 줄", Token::Comment));
        assert!(has(&t, "fn", Token::Keyword));
        assert!(has(&t, "let", Token::Keyword));
        assert!(has(&t, "0x1F", Token::Number));
        assert!(has(&t, "/* 블록\n주석 */", Token::Comment));
        assert!(has(&t, "\"문자\\\"열\"", Token::String));
        assert!(has(&t, "'c'", Token::String));
        assert!(
            !t.iter().any(|(p, _)| p.starts_with("'a")),
            "수명은 문자열 아님: {t:?}"
        );
        assert!(!has(&t, "main", Token::Keyword));

        let t = tokens(Language::C, "#include <stdio.h>\nint x1 = 3;");
        assert!(has(&t, "#include", Token::Keyword));
        assert!(has(&t, "int", Token::Keyword));
        assert!(
            !t.iter().any(|(p, _)| *p == "1"),
            "낱말 안 숫자는 숫자 아님"
        );
    }

    #[test]
    fn data_and_config_languages() {
        let t = tokens(
            Language::Json,
            "{\"name\": \"vic\", \"n\": -1.5e3, \"ok\": true}",
        );
        assert!(has(&t, "\"name\"", Token::Key));
        assert!(has(&t, "\"vic\"", Token::String));
        assert!(has(&t, "1.5e3", Token::Number));
        assert!(has(&t, "true", Token::Keyword));

        let t = tokens(
            Language::Yaml,
            "name: CI\non:\n  push:\n    branches:\n      - main\n  - uses: actions/checkout@v7 # 주석\n",
        );
        assert!(has(&t, "name", Token::Key));
        assert!(has(&t, "on", Token::Key), "{t:?}");
        assert!(has(&t, "branches", Token::Key));
        assert!(has(&t, "uses", Token::Key));
        assert!(has(&t, "# 주석", Token::Comment));

        let t = tokens(
            Language::Toml,
            "[package]\nname = \"x\" # 주석\nedition.workspace = true\n",
        );
        assert!(has(&t, "[package]", Token::Keyword));
        assert!(has(&t, "name", Token::Key));
        assert!(has(&t, "edition.workspace", Token::Key));
        assert!(has(&t, "true", Token::Keyword));

        // nginx (스크린샷의 default.conf)
        let t = tokens(
            Language::Conf,
            "server {\n    listen 80;\n    # 주석\n    proxy_set_header Host $host;\n    rewrite ^/api(.*)$ $1 break;\n}\n",
        );
        assert!(has(&t, "server", Token::Key));
        assert!(has(&t, "listen", Token::Key));
        assert!(has(&t, "80", Token::Number));
        assert!(has(&t, "# 주석", Token::Comment));
        assert!(has(&t, "proxy_set_header", Token::Key));
        assert!(has(&t, "$host", Token::Key));

        let t = tokens(Language::Conf, "; ini 주석\n[core]\nautocrlf = false\n");
        assert!(has(&t, "; ini 주석", Token::Comment));
        assert!(has(&t, "[core]", Token::Keyword));
        assert!(has(&t, "autocrlf", Token::Key));
        assert!(has(&t, "false", Token::Keyword));
    }

    #[test]
    fn markup_and_style() {
        let t = tokens(
            Language::Html,
            "<!DOCTYPE html>\n<a href=\"/x\">링크</a><!-- c -->",
        );
        assert!(has(&t, "<!", Token::Punct));
        assert!(has(&t, "DOCTYPE", Token::Keyword));
        assert!(has(&t, "a", Token::Keyword));
        assert!(has(&t, "href", Token::Key));
        assert!(has(&t, "\"/x\"", Token::String));
        assert!(has(&t, "</", Token::Punct));
        assert!(has(&t, "<!-- c -->", Token::Comment));
        assert!(
            !t.iter().any(|(p, _)| p.contains("링크")),
            "본문은 칠하지 않음"
        );

        let t = tokens(
            Language::Css,
            "a:hover { color: #fff; margin: 0 4px !important; }",
        );
        assert!(has(&t, "color", Token::Key));
        assert!(has(&t, "margin", Token::Key));
        assert!(has(&t, "4px", Token::Number));
        assert!(has(&t, "!important", Token::Keyword));
        assert!(!has(&t, "a", Token::Key), "선택자는 키가 아님");

        let t = tokens(
            Language::TypeScript,
            "const x: string = `a${b}`;\nreturn <Box key={1} />;",
        );
        assert!(has(&t, "const", Token::Keyword));
        assert!(has(&t, "string", Token::Keyword));
        assert!(has(&t, "`a${b}`", Token::String));
        assert!(has(&t, "Box", Token::Keyword));
        assert!(has(&t, "key", Token::Key));
        assert!(has(&t, "/>", Token::Punct));
    }

    #[test]
    fn scripts() {
        let t = tokens(
            Language::Shell,
            "if [ $# -gt 0 ]; then echo \"$HOME\" ${X} # c\nfi",
        );
        assert!(has(&t, "if", Token::Keyword));
        assert!(has(&t, "# c", Token::Comment));
        assert!(has(&t, "${X}", Token::Key));
        assert!(
            !t.iter()
                .any(|(p, t)| *t == Token::Comment && p.starts_with("# -gt")),
            "$#는 주석 아님"
        );

        let t = tokens(
            Language::PowerShell,
            "<# 블록 #>\nForEach ($f in $files) { Write-Host 'x' }",
        );
        assert!(has(&t, "<# 블록 #>", Token::Comment));
        assert!(has(&t, "ForEach", Token::Keyword), "대소문자 무시");
        assert!(has(&t, "$files", Token::Key));

        let t = tokens(
            Language::Batch,
            "@echo off\nREM 주석\n:: 주석2\nset P=%PATH%\nfor %%i in (*) do echo %%i",
        );
        assert!(has(&t, "REM 주석", Token::Comment));
        assert!(has(&t, ":: 주석2", Token::Comment));
        assert!(has(&t, "%PATH%", Token::Key));
        assert!(has(&t, "%%i", Token::Key));
        assert!(has(&t, "echo", Token::Keyword));

        let t = tokens(
            Language::Python,
            "def f():\n    \"\"\"문서\n    \"\"\"\n    return None  # 끝",
        );
        assert!(has(&t, "def", Token::Keyword));
        assert!(has(&t, "\"\"\"문서\n    \"\"\"", Token::String));
        assert!(has(&t, "None", Token::Keyword));

        let t = tokens(Language::Sql, "SELECT id FROM t WHERE name = 'a' -- c");
        assert!(has(&t, "SELECT", Token::Keyword));
        assert!(has(&t, "WHERE", Token::Keyword));
        assert!(has(&t, "'a'", Token::String));
        assert!(has(&t, "-- c", Token::Comment));

        let t = tokens(
            Language::Dockerfile,
            "FROM node:24 AS build\nRUN pnpm i # c\nENV A=${B}",
        );
        assert!(has(&t, "FROM", Token::Keyword));
        assert!(has(&t, "RUN", Token::Keyword));
        assert!(has(&t, "${B}", Token::Key));

        let t = tokens(
            Language::Makefile,
            "CC := gcc\nall: main.o\n\t$(CC) -o app $(OBJ)\nifeq ($(X),1)\nendif",
        );
        assert!(has(&t, "CC", Token::Key));
        assert!(has(&t, "all", Token::Key));
        assert!(has(&t, "$(CC)", Token::Key));
        assert!(has(&t, "ifeq", Token::Keyword));
    }

    #[test]
    fn log_and_diff() {
        let t = tokens(
            Language::Log,
            "2026-10-27 10:00:01.123 [INFO] 시작\n2026-10-27 10:00:02 ERROR 실패 \"x\"\nwarning: unused",
        );
        assert!(has(&t, "2026-10-27", Token::Number));
        assert!(has(&t, "10:00:01.123", Token::Number));
        assert!(has(&t, "INFO", Token::Keyword));
        assert!(has(&t, "ERROR", Token::Error));
        assert!(has(&t, "warning", Token::Warning));
        assert!(has(&t, "\"x\"", Token::String));

        let t = tokens(
            Language::Diff,
            "diff --git a b\n--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n same\n",
        );
        assert!(has(&t, "--- a", Token::Keyword));
        assert!(has(&t, "@@ -1 +1 @@", Token::Key));
        assert!(has(&t, "-old", Token::Removed));
        assert!(has(&t, "+new", Token::Added));
        assert_eq!(t.len(), 6);
    }

    #[test]
    fn detects_language_from_file_name() {
        let cases = [
            ("ci.yml", Some(Language::Yaml)),
            ("package.json", Some(Language::Json)),
            ("Cargo.toml", Some(Language::Toml)),
            ("default.conf", Some(Language::Conf)),
            (".gitignore", Some(Language::Conf)),
            (".env.local", Some(Language::Conf)),
            ("Dockerfile", Some(Language::Dockerfile)),
            ("api.Dockerfile", Some(Language::Dockerfile)),
            ("Makefile", Some(Language::Makefile)),
            ("Inventory.tsx", Some(Language::TypeScript)),
            ("app.test.ts", Some(Language::TypeScript)),
            ("main.RS", Some(Language::Rust)),
            ("x.hpp", Some(Language::C)),
            ("build.ps1", Some(Language::PowerShell)),
            ("change.log", Some(Language::Log)),
            ("fix.patch", Some(Language::Diff)),
            ("README.md", None),
            ("메모.txt", None),
            ("noext", None),
        ];
        for (name, expected) in cases {
            assert_eq!(detect(Path::new(name)), expected, "{name}");
        }
    }

    /// 예제 파일(`cargo xtask try syntax`)마다 언어가 판정되고, 칠한 구간이 있으며 주석·문자열·키워드·키 중
    /// 둘 이상이 나온다 (한 종류만 칠해지면 규칙이 어긋난 것).
    #[test]
    fn fixture_samples_are_detected_and_colored() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/syntax");
        let mut seen = HashSet::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let lang =
                detect(&path).unwrap_or_else(|| panic!("언어 판정 실패: {}", path.display()));
            seen.insert(lang);
            let text = std::fs::read_to_string(&path).unwrap();
            let kinds: HashSet<Token> = tokenize(lang, &text).into_iter().map(|(_, t)| t).collect();
            assert!(kinds.len() >= 2, "{}: {kinds:?}", path.display());
        }
        assert_eq!(seen.len(), Language::ALL.len(), "언어마다 예제 하나 이상");
    }

    #[test]
    fn keyword_lists_are_well_formed() {
        for lang in Language::ALL {
            let d = def(lang);
            for kw in d.keywords {
                assert!(!kw.is_empty());
                if d.ignore_case {
                    assert_eq!(
                        *kw,
                        kw.to_ascii_lowercase(),
                        "{lang:?}: 대소문자 무시 목록은 소문자"
                    );
                }
            }
            assert_eq!(
                keyword_list(lang).len(),
                d.keywords.len(),
                "{lang:?}: 중복 키워드"
            );
        }
    }

    /// 1MB 상태 훑기 시간 (PLAN 기준 ≤ 10ms) — `cargo test --release --lib -- --ignored --nocapture tokenize_1mb`
    #[test]
    #[ignore = "측정용: 릴리스 빌드로 실행"]
    fn tokenize_1mb_time() {
        let unit = include_str!("syntax.rs");
        let mut text = String::new();
        while text.len() + unit.len() <= 1 << 20 {
            text.push_str(unit);
        }
        for lang in [
            Language::Rust,
            Language::TypeScript,
            Language::Json,
            Language::Log,
        ] {
            let started = std::time::Instant::now();
            let mut spans = 0;
            for _ in 0..5 {
                spans = tokenize(lang, &text).len();
            }
            println!(
                "{lang:?}: {:.2} ms / {} B, 구간 {spans}개",
                started.elapsed().as_secs_f64() * 1000.0 / 5.0,
                text.len()
            );
        }
    }
}
