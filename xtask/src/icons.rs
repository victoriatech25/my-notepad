//! `cargo xtask icons`: 앱 아이콘 SVG 원본에서 Windows `.ico`·macOS `.icns`·창 아이콘 PNG를 만든다 (P5-01).
//!
//! 결과는 `assets/icons/`에 쓰고 저장소에 커밋한다 (빌드할 때마다 만들지 않는다). SVG를 고친 뒤 다시 실행한다.
//! - 32px 이하는 줄을 줄이고 굵게 그린 `vic-notepad-small.svg`, 그보다 큰 크기는 `vic-notepad.svg`를 쓴다
//! - `.ico`는 각 크기를 PNG로 담는다 (Windows Vista 이상이 읽는 형식)
//! - `.icns`는 `iconutil`과 같은 구성: 16·32px(`ic04`·`ic05`)는 ARGB, 나머지는 PNG.
//!   16·32px 자리(`icp4`·`icp5`)에 PNG를 넣으면 macOS가 깨진 그림으로 읽는다

use std::fs;
use std::path::Path;

/// `.ico`에 담을 크기 — 탐색기·작업 표시줄·Alt+Tab이 배율별로 골라 쓴다
const ICO_SIZES: &[u32] = &[16, 20, 24, 32, 40, 48, 64, 128, 256];

/// `.icns` ARGB 항목 (종류 코드, 픽셀 크기)
const ICNS_ARGB: &[(&[u8; 4], u32)] = &[(b"ic04", 16), (b"ic05", 32)];

/// `.icns` PNG 항목 (종류 코드, 픽셀 크기). `ic11`~`ic14`는 Retina(@2x) 자리
const ICNS_PNG: &[(&[u8; 4], u32)] = &[
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
];

/// 창 아이콘(실행 중 작업 표시줄·Dock)으로 앱에 넣는 PNG 크기
const WINDOW_ICON_SIZE: u32 = 512;

/// 이 크기 이하는 작은 크기용 원본을 쓴다
const SMALL_MAX: u32 = 32;

pub fn generate(root: &Path) -> Result<(), String> {
    let dir = root.join("assets/icons");
    let large = load_svg(&dir.join("vic-notepad.svg"))?;
    let small = load_svg(&dir.join("vic-notepad-small.svg"))?;
    let render = |size: u32| -> Result<resvg::tiny_skia::Pixmap, String> {
        let tree = if size <= SMALL_MAX { &small } else { &large };
        render_pixmap(tree, size)
    };
    let render_png = |size: u32| -> Result<Vec<u8>, String> {
        render(size)?
            .encode_png()
            .map_err(|e| format!("{size}px PNG 인코딩 실패: {e}"))
    };

    let ico: Vec<(u32, Vec<u8>)> = ICO_SIZES
        .iter()
        .map(|&size| render_png(size).map(|png| (size, png)))
        .collect::<Result<_, _>>()?;
    write(&dir.join("vic-notepad.ico"), &build_ico(&ico))?;

    let mut icns: Vec<(&[u8; 4], Vec<u8>)> = ICNS_ARGB
        .iter()
        .map(|&(kind, size)| render(size).map(|pixmap| (kind, argb_rle(&pixmap))))
        .collect::<Result<_, _>>()?;
    for &(kind, size) in ICNS_PNG {
        icns.push((kind, render_png(size)?));
    }
    write(&dir.join("vic-notepad.icns"), &build_icns(&icns))?;

    write(
        &dir.join(format!("vic-notepad-{WINDOW_ICON_SIZE}.png")),
        &render_png(WINDOW_ICON_SIZE)?,
    )?;

    println!("\n✔ {}", dir.display());
    Ok(())
}

fn load_svg(path: &Path) -> Result<resvg::usvg::Tree, String> {
    let data = fs::read(path).map_err(|e| format!("{} 읽기 실패: {e}", path.display()))?;
    resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .map_err(|e| format!("{} 해석 실패: {e}", path.display()))
}

fn render_pixmap(tree: &resvg::usvg::Tree, size: u32) -> Result<resvg::tiny_skia::Pixmap, String> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or("빈 크기 아이콘")?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// `ARGB` 머리 + A·R·G·B 채널을 차례로 RLE. 압축하지 않고 128바이트 이하 그대로 복사 묶음(제어 바이트 `n - 1`)만 쓴다
fn argb_rle(pixmap: &resvg::tiny_skia::Pixmap) -> Vec<u8> {
    let pixels: Vec<_> = pixmap.pixels().iter().map(|p| p.demultiply()).collect();
    let mut out = b"ARGB".to_vec();
    for channel in 0..4 {
        let bytes: Vec<u8> = pixels
            .iter()
            .map(|c| match channel {
                0 => c.alpha(),
                1 => c.red(),
                2 => c.green(),
                _ => c.blue(),
            })
            .collect();
        for chunk in bytes.chunks(128) {
            out.push(chunk.len() as u8 - 1);
            out.extend_from_slice(chunk);
        }
    }
    out
}

/// ICONDIR(6 B) + ICONDIRENTRY(16 B × N) + PNG 데이터. 너비·높이 256은 0으로 적는다
fn build_ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        let edge = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[edge, edge, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

/// `icns` 머리(8 B: 종류 + 전체 길이) + 항목(종류 4 B + 길이 4 B + 데이터). 길이는 빅엔디언
fn build_icns(entries: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let body: usize = entries.iter().map(|(_, data)| 8 + data.len()).sum();
    let mut out = Vec::with_capacity(8 + body);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&((8 + body) as u32).to_be_bytes());
    for (kind, data) in entries {
        out.extend_from_slice(*kind);
        out.extend_from_slice(&((8 + data.len()) as u32).to_be_bytes());
        out.extend_from_slice(data);
    }
    out
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| format!("{} 쓰기 실패: {e}", path.display()))?;
    println!(
        "  {:<22} {:>8} B",
        path.file_name().unwrap_or_default().to_string_lossy(),
        bytes.len()
    );
    Ok(())
}
