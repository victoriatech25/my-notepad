//! `cargo xtask package`: 배포 파일을 `target/dist/`에 만든다 (P5-02).
//!
//! Windows: 릴리스 빌드 → 포터블 zip(`Vic Notepad/my-notepad.exe`) → Inno Setup 설치 프로그램.
//! - 설치 스크립트는 `assets/windows/vic-notepad.iss` — 버전(`Cargo.toml`)과 실행 파일 경로를 `/D`로 넘긴다
//! - ISCC는 `ISCC` 환경 변수, `PATH`, Inno Setup 6 기본 설치 위치(사용자·전체) 순서로 찾는다.
//!   없으면 `winget install JRSoftware.InnoSetup` 안내 (GitHub Actions Windows 이미지에는 들어 있다)
//!
//! macOS (P5-03): arm64·x86_64 릴리스 빌드 → `lipo`로 Universal 바이너리 → `Vic Notepad.app` → ad-hoc 서명 → `.dmg`.
//! - 번들 정보는 `assets/macos/Info.plist`(`@VERSION@` → `Cargo.toml` 버전), 아이콘은 `assets/icons/vic-notepad.icns`
//! - 없는 Rust 대상(`x86_64-apple-darwin` 등)은 `rustup target add`로 받는다
//! - 서명은 개발자 인증서 없는 ad-hoc(`codesign -s -`)이다. Apple Silicon은 서명 없는 실행 파일을 아예 돌리지 않고,
//!   번들 서명이 없으면 내려받은 앱을 "손상됨"으로 막아 "그래도 열기"로도 넘길 수 없기 때문이다 (공증은 하지 않음 — PLAN.md v0.5)
//! - `.dmg`에는 앱과 `Applications` 바로 가기를 넣는다 (끌어다 놓아 설치)

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 사용자에게 보이는 제품명 (`src/branding.rs`의 `APP_NAME`과 같다)
const APP_NAME: &str = "Vic Notepad";

/// macOS Universal 바이너리에 담는 대상
const MAC_TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

pub fn run(root: &Path) -> Result<(), String> {
    let version = app_version(root)?;
    println!("▶ {APP_NAME} {version} 패키지");
    if cfg!(windows) {
        run_windows(root, &version)
    } else if cfg!(target_os = "macos") {
        run_macos(root, &version)
    } else {
        Err("패키지는 Windows·macOS에서만 만들 수 있습니다".to_string())
    }
}

fn run_windows(root: &Path, version: &str) -> Result<(), String> {
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

fn run_macos(root: &Path, version: &str) -> Result<(), String> {
    add_missing_targets(root)?;
    let mut slices = Vec::new();
    for target in MAC_TARGETS {
        let status = Command::new(super::cargo())
            .current_dir(root)
            .args([
                "build",
                "--release",
                "--package",
                "my-notepad",
                "--target",
                target,
            ])
            .status()
            .map_err(|e| format!("cargo 실행 실패: {e}"))?;
        if !status.success() {
            return Err(format!(
                "my-notepad {target} 릴리스 빌드가 실패했습니다 ({status})"
            ));
        }
        slices.push(root.join("target").join(target).join("release/my-notepad"));
    }

    let dist = root.join("target/dist");
    let app = dist.join(format!("{APP_NAME}.app"));
    if app.exists() {
        fs::remove_dir_all(&app).map_err(|e| format!("{} 지우기 실패: {e}", app.display()))?;
    }
    let contents = app.join("Contents");
    let macos_dir = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&macos_dir, &resources] {
        fs::create_dir_all(dir).map_err(|e| format!("{} 만들기 실패: {e}", dir.display()))?;
    }

    let mut lipo = Command::new("lipo");
    lipo.arg("-create")
        .arg("-output")
        .arg(macos_dir.join("my-notepad"))
        .args(&slices);
    run_tool(&mut lipo, "lipo")?;

    let plist_src = root.join("assets/macos/Info.plist");
    let plist = fs::read_to_string(&plist_src)
        .map_err(|e| format!("{} 읽기 실패: {e}", plist_src.display()))?
        .replace("@VERSION@", version);
    let plist_dst = contents.join("Info.plist");
    fs::write(&plist_dst, plist).map_err(|e| format!("{} 쓰기 실패: {e}", plist_dst.display()))?;
    let icns = root.join("assets/icons/vic-notepad.icns");
    fs::copy(&icns, resources.join("vic-notepad.icns"))
        .map_err(|e| format!("{} 복사 실패: {e}", icns.display()))?;

    run_tool(
        Command::new("codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(&app),
        "codesign",
    )?;
    run_tool(
        Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(&app),
        "codesign --verify",
    )?;
    println!(
        "  ✔ {} (Universal: {})",
        app.display(),
        MAC_TARGETS.join(" + ")
    );

    // 끌어다 놓아 설치하도록 앱 옆에 Applications 바로 가기를 둔다
    let staging = dist.join("dmg-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|e| format!("{} 지우기 실패: {e}", staging.display()))?;
    }
    fs::create_dir_all(&staging).map_err(|e| format!("{} 만들기 실패: {e}", staging.display()))?;
    run_tool(
        Command::new("ditto")
            .arg(&app)
            .arg(staging.join(format!("{APP_NAME}.app"))),
        "ditto",
    )?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .map_err(|e| format!("Applications 바로 가기 만들기 실패: {e}"))?;

    let dmg = dist.join(format!("vic-notepad-{version}-macos-universal.dmg"));
    run_tool(
        Command::new("hdiutil")
            .args(["create", "-quiet", "-ov", "-fs", "HFS+", "-format", "UDZO"])
            .args(["-volname", APP_NAME])
            .arg("-srcfolder")
            .arg(&staging)
            .arg(&dmg),
        "hdiutil",
    )?;
    let _ = fs::remove_dir_all(&staging);
    report(&dmg);
    Ok(())
}

/// Universal 빌드에 필요한 Rust 대상이 없으면 받는다 (`rust-toolchain.toml`의 툴체인에)
fn add_missing_targets(root: &Path) -> Result<(), String> {
    let installed = Command::new("rustup")
        .current_dir(root)
        .args(["target", "list", "--installed"])
        .output()
        .map_err(|e| format!("rustup 실행 실패: {e}"))?;
    let installed = String::from_utf8_lossy(&installed.stdout);
    let missing: Vec<&str> = MAC_TARGETS
        .into_iter()
        .filter(|target| !installed.lines().any(|line| line.trim() == *target))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    println!("  Rust 대상 추가: {}", missing.join(", "));
    run_tool(
        Command::new("rustup")
            .current_dir(root)
            .args(["target", "add"])
            .args(&missing),
        "rustup target add",
    )
}

fn run_tool(command: &mut Command, name: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("{name} 실행 실패: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name}이(가) 실패했습니다 ({status})"))
    }
}

fn report(path: &Path) {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!(
        "  ✔ {} ({:.1} MB)",
        path.display(),
        size as f64 / (1024.0 * 1024.0)
    );
}
