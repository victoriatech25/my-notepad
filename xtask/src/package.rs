//! `cargo xtask package`: 배포 파일을 `target/dist/`에 만든다 (P5-02).
//!
//! Windows: 릴리스 빌드 → 포터블 zip(`Vic Notepad/my-notepad.exe`) → Inno Setup 설치 프로그램.
//! - 설치 스크립트는 `assets/windows/vic-notepad.iss` — 버전(`Cargo.toml`)과 실행 파일 경로를 `/D`로 넘긴다
//! - ISCC는 `ISCC` 환경 변수, `PATH`, Inno Setup 6 기본 설치 위치(사용자·전체) 순서로 찾는다.
//!   없으면 `winget install JRSoftware.InnoSetup` 안내 (GitHub Actions Windows 이미지에는 들어 있다)
//!
//! macOS `.app`·`.dmg`는 P5-03에서 더한다.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 사용자에게 보이는 제품명 (`src/branding.rs`의 `APP_NAME`과 같다)
const APP_NAME: &str = "Vic Notepad";

pub fn run(root: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("지금은 Windows 패키지만 만들 수 있습니다 (macOS는 P5-03)".to_string());
    }
    let version = app_version(root)?;
    println!("▶ {APP_NAME} {version} 패키지");

    let status = Command::new(super::cargo())
        .current_dir(root)
        .args(["build", "--release", "--package", "my-notepad"])
        .status()
        .map_err(|e| format!("cargo 실행 실패: {e}"))?;
    if !status.success() {
        return Err(format!("my-notepad 릴리스 빌드가 실패했습니다 ({status})"));
    }
    let exe = root.join("target/release/my-notepad.exe");

    let dist = root.join("target/dist");
    fs::create_dir_all(&dist).map_err(|e| format!("{} 만들기 실패: {e}", dist.display()))?;

    let zip_path = dist.join(format!("vic-notepad-{version}-windows-x64-portable.zip"));
    write_portable_zip(&exe, &zip_path)?;
    report(&zip_path);

    let iscc = find_iscc().ok_or(
        "Inno Setup 6(ISCC.exe)을 찾지 못했습니다. 설치: winget install JRSoftware.InnoSetup \
         (다른 위치라면 ISCC 환경 변수에 ISCC.exe 경로)",
    )?;
    println!("  ISCC: {}", iscc.display());
    let script = root.join("assets/windows/vic-notepad.iss");
    let status = Command::new(&iscc)
        .current_dir(root)
        .arg("/Q")
        .arg(format!("/DAppVersion={version}"))
        .arg(format!("/DAppExe={}", exe.display()))
        .arg(format!("/O{}", dist.display()))
        .arg(&script)
        .status()
        .map_err(|e| format!("ISCC 실행 실패: {e}"))?;
    if !status.success() {
        return Err(format!("설치 프로그램 컴파일이 실패했습니다 ({status})"));
    }
    report(&dist.join(format!("vic-notepad-{version}-windows-x64-setup.exe")));
    Ok(())
}

/// 본 앱 `Cargo.toml`의 `[package] version`
fn app_version(root: &Path) -> Result<String, String> {
    let manifest = root.join("Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .map_err(|e| format!("{} 읽기 실패: {e}", manifest.display()))?;
    text.lines()
        .find_map(|line| line.strip_prefix("version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .map(str::to_string)
        .ok_or_else(|| format!("{}에서 version을 찾지 못했습니다", manifest.display()))
}

/// 압축을 풀면 `Vic Notepad` 폴더 하나가 나오게 담는다
fn write_portable_zip(exe: &Path, zip_path: &Path) -> Result<(), String> {
    let bytes = fs::read(exe).map_err(|e| format!("{} 읽기 실패: {e}", exe.display()))?;
    let file = fs::File::create(zip_path)
        .map_err(|e| format!("{} 만들기 실패: {e}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);
    let fail = |e: &dyn std::fmt::Display| format!("{} 쓰기 실패: {e}", zip_path.display());
    zip.start_file(format!("{APP_NAME}/my-notepad.exe"), options)
        .map_err(|e| fail(&e))?;
    zip.write_all(&bytes).map_err(|e| fail(&e))?;
    zip.finish().map_err(|e| fail(&e))?;
    Ok(())
}

fn find_iscc() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ISCC").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let in_path = std::env::var_os("PATH").into_iter().flat_map(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("ISCC.exe"))
            .collect::<Vec<_>>()
    });
    let installed = [
        ("LOCALAPPDATA", r"Programs\Inno Setup 6"),
        ("ProgramFiles(x86)", "Inno Setup 6"),
        ("ProgramFiles", "Inno Setup 6"),
    ]
    .into_iter()
    .filter_map(|(var, dir)| {
        std::env::var_os(var).map(|base| PathBuf::from(base).join(dir).join("ISCC.exe"))
    });
    in_path.chain(installed).find(|path| path.is_file())
}

fn report(path: &Path) {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!(
        "  ✔ {} ({:.1} MB)",
        path.display(),
        size as f64 / (1024.0 * 1024.0)
    );
}
