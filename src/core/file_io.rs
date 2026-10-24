//! 파일 입출력 (원자적 저장, 파일 크기 판정, 바이너리 확인, 읽기 전용/권한 오류 — PLAN.md §3.6, ADR 0001, P1-08).
//!
//! 설계 원칙:
//! - **원자적 저장**: 대상과 같은 폴더의 임시 파일에 모두 쓰고 디스크에 플러시한 뒤 rename으로 교체한다.
//!   쓰는 도중 실패하면 임시 파일만 지워지고 원본은 그대로 남는다.
//! - **크기 판정** (ADR 0001): 경계값 이하 → 편집, 경계값 초과 ~ 뷰어 경계값 이하 → 편집 + 안내 막대,
//!   뷰어 경계값 초과 → 읽기 전용 뷰어. 경계값은 [`SizePolicy`]로 바꿀 수 있다.
//! - **바이너리 확인**: 인코딩 감지(P1-04)가 바이너리로 판정하면 바로 열지 않고 [`OpenOutcome::Binary`]를
//!   돌려준다. UI가 "그래도 여시겠습니까?"를 확인한 뒤 [`open_binary_anyway`]로 읽기 전용 문서를 만든다.
//! - **읽기 전용/권한**: 읽기 전용 파일은 읽기 전용 문서로 열고, 그 경로로 저장하면 [`FileIoErrorKind::ReadOnly`]를
//!   돌려줘 UI가 "다른 이름으로 저장"을 유도하게 한다.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::core::document::{DiskMetadata, Document};
use crate::core::encoding::{
    DetectionOutcome, TextEncoding, UnmappableError, decode_with_encoding, detect_encoding,
};
use crate::core::line_ending::LineEnding;

/// 이 크기(바이트)까지는 일반 편집으로 연다 (ADR 0001, 1MB).
pub const DEFAULT_NOTICE_ABOVE_BYTES: u64 = 1024 * 1024;

/// 이 크기(바이트)를 넘으면 읽기 전용 뷰어로 연다 (ADR 0001, 5MB).
pub const DEFAULT_VIEWER_ABOVE_BYTES: u64 = 5 * 1024 * 1024;

/// 파일 크기에 따른 열기 방식 (ADR 0001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// 일반 편집 (방식 A 편집 위젯).
    Edit,
    /// 편집 허용 + 상단 안내 막대 ("큰 파일이라 입력이 느릴 수 있습니다 · 읽기 전용으로 보기").
    EditWithNotice,
    /// 읽기 전용 뷰어 (보이는 줄만 그림, "편집 모드로 열기"는 경고 후 허용 — P1-14).
    Viewer,
}

/// 열기 방식을 정하는 파일 크기 경계값 (설정으로 변경 가능).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizePolicy {
    notice_above: u64,
    viewer_above: u64,
}

impl Default for SizePolicy {
    fn default() -> Self {
        Self {
            notice_above: DEFAULT_NOTICE_ABOVE_BYTES,
            viewer_above: DEFAULT_VIEWER_ABOVE_BYTES,
        }
    }
}

impl SizePolicy {
    /// 경계값을 지정해 만든다. `viewer_above`가 `notice_above`보다 작으면 `notice_above`로 올린다
    /// (안내 구간이 없어질 뿐 구간 순서는 뒤집히지 않는다).
    pub fn new(notice_above: u64, viewer_above: u64) -> Self {
        Self {
            notice_above,
            viewer_above: viewer_above.max(notice_above),
        }
    }

    /// 이 크기를 넘으면 안내 막대를 표시한다.
    pub fn notice_above(&self) -> u64 {
        self.notice_above
    }

    /// 이 크기를 넘으면 읽기 전용 뷰어로 연다.
    pub fn viewer_above(&self) -> u64 {
        self.viewer_above
    }

    /// 파일 크기(바이트)로 열기 방식을 판정한다. 경계값과 같은 크기는 아래 구간에 속한다.
    pub fn classify(&self, byte_len: u64) -> OpenMode {
        if byte_len <= self.notice_above {
            OpenMode::Edit
        } else if byte_len <= self.viewer_above {
            OpenMode::EditWithNotice
        } else {
            OpenMode::Viewer
        }
    }
}

/// 파일 입출력 오류 종류 (UI가 대화상자 문구를 고르는 기준).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIoErrorKind {
    /// 파일이 없음.
    NotFound,
    /// 운영체제 권한 부족 (폴더 쓰기 권한 없음, 다른 프로그램이 잠금 등).
    PermissionDenied,
    /// 읽기 전용 파일에 저장하려 함 → "다른 이름으로 저장" 유도.
    ReadOnly,
    /// 경로가 폴더임.
    IsDirectory,
    /// 그 밖의 입출력 오류.
    Other,
}

/// 파일 입출력 오류.
#[derive(Debug)]
pub struct FileIoError {
    kind: FileIoErrorKind,
    path: PathBuf,
    source: Option<io::Error>,
}

impl FileIoError {
    fn new(kind: FileIoErrorKind, path: &Path) -> Self {
        Self {
            kind,
            path: path.to_path_buf(),
            source: None,
        }
    }

    fn from_io(path: &Path, err: io::Error) -> Self {
        let kind = match err.kind() {
            io::ErrorKind::NotFound => FileIoErrorKind::NotFound,
            io::ErrorKind::PermissionDenied => FileIoErrorKind::PermissionDenied,
            io::ErrorKind::IsADirectory => FileIoErrorKind::IsDirectory,
            io::ErrorKind::ReadOnlyFilesystem => FileIoErrorKind::ReadOnly,
            _ => FileIoErrorKind::Other,
        };
        Self {
            kind,
            path: path.to_path_buf(),
            source: Some(err),
        }
    }

    /// 오류 종류.
    pub fn kind(&self) -> FileIoErrorKind {
        self.kind
    }

    /// 오류가 난 파일 경로.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for FileIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.display();
        match self.kind {
            FileIoErrorKind::NotFound => write!(f, "파일을 찾을 수 없습니다: {path}"),
            FileIoErrorKind::PermissionDenied => write!(f, "파일에 접근할 권한이 없습니다: {path}"),
            FileIoErrorKind::ReadOnly => write!(f, "읽기 전용 파일입니다: {path}"),
            FileIoErrorKind::IsDirectory => {
                write!(f, "폴더는 파일로 열거나 저장할 수 없습니다: {path}")
            }
            FileIoErrorKind::Other => write!(f, "파일 입출력 오류: {path}"),
        }?;
        if let Some(source) = &self.source {
            write!(f, " ({source})")?;
        }
        Ok(())
    }
}

impl std::error::Error for FileIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e as _)
    }
}

/// 열어 둔 파일의 디스크 상태 변화 (외부 변경 감지, PLAN.md §3.6, P2-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskChange {
    /// 기록해 둔 수정 시각·크기와 같다
    Unchanged,
    /// 다른 프로그램이 바꿨다 (지금 디스크 메타데이터)
    Modified(DiskMetadata),
    /// 파일이 없어졌다
    Deleted,
}

/// 파일의 지금 디스크 메타데이터를 열기·저장 때 기록한 `known`과 비교한다.
///
/// 수정 시각이나 크기가 다르면 바뀐 것으로 본다. 기록이 없으면(새 문서 등) 비교하지 않는다.
/// 권한 등으로 메타데이터를 읽지 못하면 바뀌지 않은 것으로 본다 (잘못된 경고보다 조용히 넘어가기).
pub fn check_disk_change(path: &Path, known: Option<DiskMetadata>) -> DiskChange {
    let Some(known) = known else {
        return DiskChange::Unchanged;
    };
    match DiskMetadata::from_path(path) {
        Ok(now) if now == known => DiskChange::Unchanged,
        Ok(now) => DiskChange::Modified(now),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DiskChange::Deleted,
        Err(_) => DiskChange::Unchanged,
    }
}

/// 디스크에서 읽은 원시 파일 (디코딩 전).
#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub metadata: DiskMetadata,
    /// 파일 시스템의 읽기 전용 속성.
    pub readonly: bool,
    /// 파일 크기에 따른 열기 방식.
    pub open_mode: OpenMode,
}

/// 파일을 읽고 크기·읽기 전용 여부를 판정한다 (디코딩은 하지 않음).
pub fn read_file(path: &Path, policy: &SizePolicy) -> Result<LoadedFile, FileIoError> {
    let meta = fs::metadata(path).map_err(|e| FileIoError::from_io(path, e))?;
    if meta.is_dir() {
        return Err(FileIoError::new(FileIoErrorKind::IsDirectory, path));
    }
    let bytes = fs::read(path).map_err(|e| FileIoError::from_io(path, e))?;
    let metadata = DiskMetadata {
        modified_time: meta.modified().ok(),
        byte_len: bytes.len() as u64,
    };
    Ok(LoadedFile {
        path: path.to_path_buf(),
        open_mode: policy.classify(metadata.byte_len),
        readonly: meta.permissions().readonly(),
        bytes,
        metadata,
    })
}

/// 바이트가 바이너리 파일로 판정되는지 여부 (PLAN.md §3.3-1의 5단계, P1-04 감지 규칙과 동일).
pub fn is_binary(bytes: &[u8]) -> bool {
    detect_encoding(bytes) == DetectionOutcome::Binary
}

/// 열린 문서와 그 열기 방식.
#[derive(Debug, Clone)]
pub struct OpenedDocument {
    pub document: Document,
    pub open_mode: OpenMode,
    /// 파일 시스템의 읽기 전용 속성 (탭 자물쇠 표시, 뷰어에서 "편집 모드로 열기" 후에도 저장 시 다른 이름으로 저장 유도).
    ///
    /// 문서의 읽기 전용 여부([`Document::is_readonly`])는 뷰어·바이너리 열람 때문에도 켜지므로 이 값과 구분한다.
    pub file_readonly: bool,
}

/// [`open_document`] 결과.
#[derive(Debug, Clone)]
pub enum OpenOutcome {
    /// 텍스트 문서로 열림.
    Opened(Box<OpenedDocument>),
    /// 바이너리 파일 — UI가 확인한 뒤 [`open_binary_anyway`]로 연다.
    Binary(Box<LoadedFile>),
}

/// 파일을 읽어 인코딩·줄바꿈을 감지한 [`Document`]로 연다.
///
/// 파일이 읽기 전용이거나 크기가 뷰어 구간이면 문서도 읽기 전용으로 연다.
pub fn open_document(
    path: &Path,
    policy: &SizePolicy,
    default_encoding: TextEncoding,
    default_line_ending: LineEnding,
) -> Result<OpenOutcome, FileIoError> {
    let LoadedFile {
        path,
        bytes,
        metadata,
        readonly: file_readonly,
        open_mode,
    } = read_file(path, policy)?;
    let readonly = file_readonly || open_mode == OpenMode::Viewer;
    // 바이트 소유권을 넘겨 본문 복사를 줄인다 (10MB 파일에서 열기 중 최대 사용량 40MB → 약 10MB).
    match Document::from_owned_bytes_with_defaults(
        Some(path.clone()),
        bytes,
        default_encoding,
        default_line_ending,
        readonly,
        Some(metadata),
    ) {
        Ok(document) => Ok(OpenOutcome::Opened(Box::new(OpenedDocument {
            document,
            open_mode,
            file_readonly,
        }))),
        Err(bytes) => Ok(OpenOutcome::Binary(Box::new(LoadedFile {
            path,
            bytes,
            metadata,
            readonly: file_readonly,
            open_mode,
        }))),
    }
}

/// 사용자가 확인한 바이너리 파일을 `UTF-8`(잘못된 바이트는 `U+FFFD`)로 디코딩해 읽기 전용 문서로 연다.
pub fn open_binary_anyway(loaded: LoadedFile, default_line_ending: LineEnding) -> OpenedDocument {
    let decoded = decode_with_encoding(&loaded.bytes, TextEncoding::Utf8);
    OpenedDocument {
        document: Document::from_decoded(
            Some(loaded.path),
            decoded,
            default_line_ending,
            true,
            Some(loaded.metadata),
        ),
        open_mode: loaded.open_mode,
        file_readonly: loaded.readonly,
    }
}

/// `bytes`를 `path`에 원자적으로 저장하고 저장 후 디스크 메타데이터를 돌려준다.
///
/// 대상이 읽기 전용 파일이면 쓰지 않고 [`FileIoErrorKind::ReadOnly`]를 돌려준다.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<DiskMetadata, FileIoError> {
    write_atomic_with(path, |file| file.write_all(bytes))
}

/// 임시 파일에 `write`로 내용을 쓴 뒤 `path`로 교체한다. `write`가 실패하면 원본은 그대로 둔다.
fn write_atomic_with(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> Result<DiskMetadata, FileIoError> {
    // 심볼릭 링크는 링크 자체를 일반 파일로 바꾸지 않고 가리키는 파일에 저장한다.
    let target = resolve_symlinks(path).map_err(|e| FileIoError::from_io(path, e))?;
    let existing = match fs::metadata(&target) {
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(FileIoError::from_io(path, e)),
    };
    if let Some(meta) = &existing {
        if meta.is_dir() {
            return Err(FileIoError::new(FileIoErrorKind::IsDirectory, path));
        }
        // Unix에서는 폴더 쓰기 권한만 있으면 읽기 전용 파일도 rename으로 덮어써지므로 직접 막는다.
        if meta.permissions().readonly() {
            return Err(FileIoError::new(FileIoErrorKind::ReadOnly, path));
        }
    }

    let dir = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let file_name = target
        .file_name()
        .ok_or_else(|| FileIoError::new(FileIoErrorKind::Other, path))?;
    let mut prefix = std::ffi::OsString::from(".");
    prefix.push(file_name);
    prefix.push(".");

    let mut temp = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".tmp")
        .make_in(dir, |p| create_temp_file(p, existing.as_ref()))
        .map_err(|e| FileIoError::from_io(path, e))?;

    write(temp.as_file_mut())
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|e| FileIoError::from_io(path, e))?;

    // `NamedTempFile::persist`는 Windows에서 rename 직전에 속성을 NORMAL로 초기화해 숨김 속성이 사라지므로
    // 직접 rename한다. 실패하면 `TempPath`가 드롭되며 임시 파일을 지운다.
    let temp_path = temp.into_temp_path();
    fs::rename(&temp_path, &target).map_err(|e| FileIoError::from_io(path, e))?;
    // 이미 대상 이름으로 옮겨졌으므로 임시 경로 삭제를 끈다 (파일 시스템 작업 없음).
    let _ = temp_path.keep();

    // rename 결과(폴더 항목)까지 디스크에 남긴다. Windows는 폴더를 이렇게 열 수 없으므로 Unix만.
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }

    DiskMetadata::from_path(&target).map_err(|e| FileIoError::from_io(path, e))
}

/// 심볼릭 링크가 마지막으로 가리키는 파일 경로를 구한다 (대상이 없어도 그 경로를 돌려준다).
fn resolve_symlinks(path: &Path) -> io::Result<PathBuf> {
    const MAX_DEPTH: usize = 40;
    let mut target = path.to_path_buf();
    for _ in 0..MAX_DEPTH {
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = fs::read_link(&target)?;
                // 상대 링크는 링크가 있는 폴더 기준. 절대 링크는 join이 그대로 돌려준다.
                target = match target.parent() {
                    Some(parent) => parent.join(link),
                    None => link,
                };
            }
            Ok(_) => return Ok(target),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(target),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("심볼릭 링크가 너무 많이 이어져 있습니다"))
}

/// 원자적 저장용 임시 파일을 만든다. 원본이 있으면 권한·속성을 이어받는다.
///
/// - Unix: 새 파일은 일반 파일처럼 `0o666 & !umask`, 기존 파일은 원본 권한(실행 비트 포함)을 그대로 쓴다.
///   (`tempfile` 기본값 `0o600`으로 두면 새로 저장한 파일을 다른 사용자가 읽지 못한다.)
/// - Windows: 원본의 숨김·시스템 속성을 유지한다 (rename으로 교체하면 새 파일 속성이 남는다).
fn create_temp_file(path: &Path, original: Option<&fs::Metadata>) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 원본 권한으로 바꾸기 전까지 다른 사용자가 열지 못하게, 기존 파일이 있으면 소유자 전용으로 만든다.
        options.mode(if original.is_some() { 0o600 } else { 0o666 });
        let file = options.open(path)?;
        if let Some(meta) = original {
            file.set_permissions(meta.permissions())?;
        }
        Ok(file)
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
        if let Some(meta) = original {
            options.attributes(
                meta.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM),
            );
        }
        options.open(path)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = original;
        options.open(path)
    }
}

/// [`save_document`] 오류.
#[derive(Debug)]
pub enum SaveError {
    /// 경로가 없는 새 문서 → "다른 이름으로 저장"이 필요함.
    NoPath,
    /// 현재 인코딩으로 표현할 수 없는 문자가 있음 (경고 대화상자 — P1-12).
    Unmappable(UnmappableError),
    /// 파일 쓰기 실패. 원본 파일은 바뀌지 않았다.
    Io(FileIoError),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPath => write!(f, "저장할 경로가 없습니다"),
            Self::Unmappable(e) => write!(
                f,
                "{}(으)로 표현할 수 없는 문자가 {}개 있습니다",
                e.encoding.label(),
                e.unmappable.len()
            ),
            Self::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// 문서를 현재 인코딩·줄바꿈으로 직렬화해 원자적으로 저장하고, 성공하면 [`Document::mark_saved`]를 호출한다.
///
/// `target`이 `Some`이면 "다른 이름으로 저장", `None`이면 문서의 현재 경로에 저장한다.
/// 실패하면 문서 상태(수정 플래그·경로)는 바뀌지 않는다.
pub fn save_document(doc: &mut Document, target: Option<&Path>) -> Result<(), SaveError> {
    let path = match target {
        Some(p) => p.to_path_buf(),
        None => doc.path().map(Path::to_path_buf).ok_or(SaveError::NoPath)?,
    };
    let bytes = doc.encode_for_save().map_err(SaveError::Unmappable)?;
    let metadata = write_atomic(&path, &bytes).map_err(SaveError::Io)?;
    doc.mark_saved(Some(path), Some(metadata));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_change_detects_modify_delete_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memo.txt");
        fs::write(&path, "처음").unwrap();
        let known = DiskMetadata::from_path(&path).unwrap();
        assert_eq!(check_disk_change(&path, Some(known)), DiskChange::Unchanged);
        assert_eq!(check_disk_change(&path, None), DiskChange::Unchanged);

        // 크기가 바뀜
        fs::write(&path, "다른 프로그램이 고친 내용").unwrap();
        let DiskChange::Modified(now) = check_disk_change(&path, Some(known)) else {
            panic!("바뀐 것으로 봐야 함");
        };
        assert_ne!(now.byte_len, known.byte_len);

        // 크기는 같고 수정 시각만 바뀜
        let same_len = DiskMetadata {
            modified_time: known
                .modified_time
                .map(|t| t - std::time::Duration::from_secs(60)),
            ..now
        };
        assert!(matches!(
            check_disk_change(&path, Some(same_len)),
            DiskChange::Modified(_)
        ));

        fs::remove_file(&path).unwrap();
        assert_eq!(check_disk_change(&path, Some(known)), DiskChange::Deleted);
    }

    const MB: u64 = 1024 * 1024;

    /// 폴더에 남은 임시 파일(`.이름.*.tmp`)이 없는지 확인한다.
    fn leftover_temp_files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "tmp"))
            .collect()
    }

    #[test]
    fn size_policy_default_boundaries() {
        let policy = SizePolicy::default();
        assert_eq!(policy.classify(0), OpenMode::Edit);
        assert_eq!(policy.classify(MB), OpenMode::Edit);
        assert_eq!(policy.classify(MB + 1), OpenMode::EditWithNotice);
        assert_eq!(policy.classify(5 * MB), OpenMode::EditWithNotice);
        assert_eq!(policy.classify(5 * MB + 1), OpenMode::Viewer);
        assert_eq!(policy.classify(50 * MB), OpenMode::Viewer);
    }

    #[test]
    fn size_policy_custom_boundaries() {
        let policy = SizePolicy::new(2 * MB, 10 * MB);
        assert_eq!(policy.classify(2 * MB), OpenMode::Edit);
        assert_eq!(policy.classify(2 * MB + 1), OpenMode::EditWithNotice);
        assert_eq!(policy.classify(10 * MB), OpenMode::EditWithNotice);
        assert_eq!(policy.classify(10 * MB + 1), OpenMode::Viewer);

        // 뷰어 경계가 안내 경계보다 작으면 안내 구간 없이 바로 뷰어
        let policy = SizePolicy::new(100, 10);
        assert_eq!(policy.viewer_above(), 100);
        assert_eq!(policy.classify(100), OpenMode::Edit);
        assert_eq!(policy.classify(101), OpenMode::Viewer);
    }

    #[test]
    fn write_atomic_creates_and_replaces_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memo.txt");

        let meta = write_atomic(&path, "첫 내용".as_bytes()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), "첫 내용".as_bytes());
        assert_eq!(meta.byte_len, "첫 내용".len() as u64);

        let meta = write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(meta.byte_len, 6);
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn failed_write_keeps_original_and_cleans_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memo.txt");
        fs::write(&path, "원본 내용").unwrap();

        // 절반쯤 쓰다가 디스크 오류가 난 상황
        let err = write_atomic_with(&path, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("디스크 가득 참"))
        })
        .unwrap_err();
        assert_eq!(err.kind(), FileIoErrorKind::Other);
        assert_eq!(fs::read_to_string(&path).unwrap(), "원본 내용");
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn readonly_file_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.txt");
        fs::write(&path, "잠긴 파일").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms.clone()).unwrap();

        let err = write_atomic(&path, b"new").unwrap_err();
        assert_eq!(err.kind(), FileIoErrorKind::ReadOnly);
        assert_eq!(fs::read_to_string(&path).unwrap(), "잠긴 파일");

        // 읽기 전용 파일은 읽기 전용 문서로 열린다
        let policy = SizePolicy::default();
        let loaded = read_file(&path, &policy).unwrap();
        assert!(loaded.readonly);
        let OpenOutcome::Opened(opened) =
            open_document(&path, &policy, TextEncoding::Utf8, LineEnding::Lf).unwrap()
        else {
            panic!("텍스트 파일이 바이너리로 판정됨");
        };
        assert!(opened.document.is_readonly());
        assert!(opened.file_readonly);

        // tempdir 정리를 위해 읽기 전용 해제
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn directory_and_missing_paths_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let policy = SizePolicy::default();

        let err = read_file(dir.path(), &policy).unwrap_err();
        assert_eq!(err.kind(), FileIoErrorKind::IsDirectory);
        let err = write_atomic(dir.path(), b"x").unwrap_err();
        assert_eq!(err.kind(), FileIoErrorKind::IsDirectory);

        let missing = dir.path().join("없는 파일.txt");
        let err = read_file(&missing, &policy).unwrap_err();
        assert_eq!(err.kind(), FileIoErrorKind::NotFound);
        assert!(err.to_string().contains("없는 파일.txt"));

        // 없는 폴더 안에 저장 → 오류, 아무것도 생기지 않음
        let err = write_atomic(&dir.path().join("없는 폴더").join("a.txt"), b"x").unwrap_err();
        assert_eq!(err.kind(), FileIoErrorKind::NotFound);
    }

    #[test]
    fn open_document_detects_binary_and_size_mode() {
        let dir = tempfile::tempdir().unwrap();
        let policy = SizePolicy::new(10, 20);

        let small = dir.path().join("small.txt");
        fs::write(&small, "짧음").unwrap();
        let OpenOutcome::Opened(opened) =
            open_document(&small, &policy, TextEncoding::Utf8, LineEnding::Lf).unwrap()
        else {
            panic!("텍스트 파일이 바이너리로 판정됨");
        };
        assert_eq!(opened.open_mode, OpenMode::Edit);
        assert!(!opened.document.is_readonly());
        assert_eq!(opened.document.text(), "짧음");

        let medium = dir.path().join("medium.txt");
        fs::write(&medium, "a".repeat(15)).unwrap();
        let OpenOutcome::Opened(opened) =
            open_document(&medium, &policy, TextEncoding::Utf8, LineEnding::Lf).unwrap()
        else {
            panic!("텍스트 파일이 바이너리로 판정됨");
        };
        assert_eq!(opened.open_mode, OpenMode::EditWithNotice);
        assert!(!opened.document.is_readonly());

        let large = dir.path().join("large.log");
        fs::write(&large, "a".repeat(21)).unwrap();
        let OpenOutcome::Opened(opened) =
            open_document(&large, &policy, TextEncoding::Utf8, LineEnding::Lf).unwrap()
        else {
            panic!("텍스트 파일이 바이너리로 판정됨");
        };
        assert_eq!(opened.open_mode, OpenMode::Viewer);
        assert!(opened.document.is_readonly());
        // 뷰어라서 읽기 전용일 뿐 파일 자체는 쓸 수 있다
        assert!(!opened.file_readonly);

        let bin = dir.path().join("image.bin");
        fs::write(&bin, b"hello\0world").unwrap();
        assert!(is_binary(&fs::read(&bin).unwrap()));
        let OpenOutcome::Binary(loaded) =
            open_document(&bin, &policy, TextEncoding::Utf8, LineEnding::Lf).unwrap()
        else {
            panic!("바이너리 파일이 텍스트로 열림");
        };
        let opened = open_binary_anyway(*loaded, LineEnding::Lf);
        assert!(opened.document.is_readonly());
        assert_eq!(opened.document.text(), "hello\0world");
        assert_eq!(opened.document.path(), Some(bin.as_path()));
    }

    #[test]
    fn save_document_roundtrip_and_save_as() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        fs::write(&path, "줄1\r\n줄2\r\n").unwrap();

        let OpenOutcome::Opened(opened) = open_document(
            &path,
            &SizePolicy::default(),
            TextEncoding::Utf8,
            LineEnding::Lf,
        )
        .unwrap() else {
            panic!("텍스트 파일이 바이너리로 판정됨");
        };
        let mut doc = opened.document;
        assert!(doc.set_text("줄1\n줄2\n줄3\n"));
        assert!(doc.is_modified());

        save_document(&mut doc, None).unwrap();
        assert!(!doc.is_modified());
        assert_eq!(fs::read(&path).unwrap(), "줄1\r\n줄2\r\n줄3\r\n".as_bytes());
        assert_eq!(doc.disk_metadata().unwrap().byte_len, 18);

        // 새 문서는 경로가 없으면 다른 이름으로 저장이 필요
        let mut untitled =
            Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::Lf);
        untitled.set_text("새 문서");
        assert!(matches!(
            save_document(&mut untitled, None),
            Err(SaveError::NoPath)
        ));
        let new_path = dir.path().join("새 문서.txt");
        save_document(&mut untitled, Some(&new_path)).unwrap();
        assert_eq!(untitled.title(), "새 문서.txt");
        assert!(!untitled.is_modified());
        assert_eq!(fs::read_to_string(&new_path).unwrap(), "새 문서");
    }

    #[test]
    fn failed_save_keeps_document_modified() {
        let dir = tempfile::tempdir().unwrap();

        // 표현 불가 문자 → 파일을 만들지 않고 수정 상태 유지
        let path = dir.path().join("euc.txt");
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::EucKr, LineEnding::Lf);
        doc.set_text("한글 😀");
        assert!(matches!(
            save_document(&mut doc, Some(&path)),
            Err(SaveError::Unmappable(_))
        ));
        assert!(!path.exists());
        assert!(doc.is_modified());
        assert_eq!(doc.path(), None);

        // 쓰기 실패 → 경로·수정 상태 그대로
        let bad = dir.path().join("없는 폴더").join("a.txt");
        let mut doc = Document::new_untitled_with_defaults(1, TextEncoding::Utf8, LineEnding::Lf);
        doc.set_text("내용");
        assert!(matches!(
            save_document(&mut doc, Some(&bad)),
            Err(SaveError::Io(_))
        ));
        assert!(doc.is_modified());
        assert_eq!(doc.path(), None);
    }

    #[cfg(windows)]
    #[test]
    fn hidden_attribute_is_kept_after_save() {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".hidden.txt");
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .attributes(FILE_ATTRIBUTE_HIDDEN)
            .open(&path)
            .unwrap()
            .write_all(b"old")
            .unwrap();

        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        let attrs = fs::metadata(&path).unwrap().file_attributes();
        assert_ne!(attrs & FILE_ATTRIBUTE_HIDDEN, 0, "숨김 속성이 사라짐");
    }

    #[cfg(windows)]
    #[test]
    fn locked_target_keeps_original_and_cleans_temp_file() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.txt");
        fs::write(&path, "원본").unwrap();

        // 다른 프로그램이 공유 없이 열어 둔 상태 → 교체(rename) 단계에서 실패
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let err = write_atomic(&path, b"new").unwrap_err();
        assert_ne!(err.kind(), FileIoErrorKind::ReadOnly);
        assert!(leftover_temp_files(dir.path()).is_empty());
        drop(lock);
        assert_eq!(fs::read_to_string(&path).unwrap(), "원본");
    }

    #[cfg(unix)]
    #[test]
    fn unix_permissions_follow_original_or_umask() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();

        // 새 파일은 tempfile 기본값(0o600)이 아니라 일반 파일 권한(0o666 & !umask)
        let new_file = dir.path().join("new.txt");
        write_atomic(&new_file, b"x").unwrap();
        let reference = dir.path().join("reference.txt");
        fs::write(&reference, b"x").unwrap();
        assert_eq!(
            fs::metadata(&new_file).unwrap().permissions().mode() & 0o777,
            fs::metadata(&reference).unwrap().permissions().mode() & 0o777
        );

        // 기존 파일은 실행 비트까지 원본 권한 유지
        let script = dir.path().join("run.sh");
        fs::write(&script, b"echo old").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o750)).unwrap();
        write_atomic(&script, b"echo new").unwrap();
        assert_eq!(
            fs::metadata(&script).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_kept_and_target_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.txt");
        fs::write(&real, b"old").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink("real.txt", &link).unwrap();

        write_atomic(&link, b"new").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"new");

        // 대상이 없는 링크도 링크를 지우지 않고 대상 파일을 만든다
        let dangling = dir.path().join("dangling.txt");
        std::os::unix::fs::symlink("created.txt", &dangling).unwrap();
        write_atomic(&dangling, b"made").unwrap();
        assert!(
            fs::symlink_metadata(&dangling)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(dir.path().join("created.txt")).unwrap(), b"made");
    }
}
