//! Markdown 미리보기 이미지 주소 해석과 디코딩 (PLAN.md §3.8-4, P3-04).
//!
//! - 주소 해석([`resolve`]): `http(s)://`·`//`는 원격(기본 차단), `data:` 등 그 밖의 스킴은 지원하지 않음,
//!   나머지는 로컬 경로 — 절대 경로는 그대로, 상대 경로는 문서 폴더 기준이며 `%20` 같은 퍼센트 인코딩을 푼다.
//!   저장하지 않은 문서는 상대 경로를 풀 수 없다.
//! - 디코딩([`decode_file`]): PNG·JPEG·GIF(첫 프레임)는 `image`, SVG는 `resvg`로 RGBA를 만든다.
//!   긴 변이 `max_side`를 넘으면 비율을 지켜 줄여 디코딩한다 (메모리 — PLAN.md §7 리스크).
//!
//! egui에 의존하지 않는다. 텍스처 업로드·해제는 `ui::markdown_view`가 맡는다.

use std::path::{Path, PathBuf};

/// 이미지 주소 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    /// 로컬 파일 경로 (있는지는 확인하지 않음)
    Local(PathBuf),
    /// 원격 주소 — 기본 차단
    Remote(String),
    /// 표시할 수 없는 주소 (지원하지 않는 스킴, 저장하지 않은 문서의 상대 경로 등) — 사유
    Unsupported(String),
}

/// `url`을 해석한다. `doc_dir`은 문서가 있는 폴더(저장하지 않은 문서면 `None`).
pub fn resolve(url: &str, doc_dir: Option<&Path>) -> ImageSource {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || url.starts_with("//") {
        return ImageSource::Remote(url.to_owned());
    }
    let path = if let Some(rest) = lower.strip_prefix("file://") {
        // file:///C:/a.png, file:///home/a.png
        let rest = &url[url.len() - rest.len()..];
        let rest = rest
            .strip_prefix('/')
            .filter(|r| has_drive(r))
            .unwrap_or(rest);
        percent_decode(rest)
    } else if has_scheme(url) {
        return ImageSource::Unsupported(format!("지원하지 않는 주소: {url}"));
    } else {
        // `a.png#frag`, `a.png?v=1`의 뒤쪽은 버린다
        let end = url.find(['?', '#']).unwrap_or(url.len());
        percent_decode(&url[..end])
    };
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return ImageSource::Local(path);
    }
    match doc_dir {
        Some(dir) => ImageSource::Local(dir.join(path)),
        None => {
            ImageSource::Unsupported("저장하지 않은 문서라 상대 경로를 찾을 수 없음".to_owned())
        }
    }
}

/// `C:` 같은 드라이브 문자로 시작하는지.
fn has_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// `scheme:`으로 시작하는지 (Windows 드라이브 문자 `C:`는 제외).
fn has_scheme(s: &str) -> bool {
    match s.find(':') {
        Some(i) if i > 1 => s[..i]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        _ => false,
    }
}

/// `%XX`를 바이트로 바꾼다 (UTF-8이 아니게 되면 원문 그대로).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (hex(bytes.get(i + 1)), hex(bytes.get(i + 2)))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

fn hex(b: Option<&u8>) -> Option<u8> {
    (*b? as char).to_digit(16).map(|d| d as u8)
}

/// 디코딩한 이미지 (RGBA 8비트, 비율을 지켜 줄였을 수 있음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// 원본 크기 (화면 표시 크기 계산용)
    pub original: (u32, u32),
    pub rgba: Vec<u8>,
}

/// 파일을 디코딩한다. 긴 변이 `max_side`를 넘으면 비율을 지켜 줄인다.
pub fn decode_file(path: &Path, max_side: u32) -> Result<DecodedImage, String> {
    let is_svg = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    if is_svg {
        return decode_svg(path, max_side);
    }
    let reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let image = reader.decode().map_err(|e| e.to_string())?;
    let original = (image.width(), image.height());
    let image = if original.0.max(original.1) > max_side {
        image.thumbnail(max_side, max_side)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    Ok(DecodedImage {
        width: rgba.width(),
        height: rgba.height(),
        original,
        rgba: rgba.into_raw(),
    })
}

fn decode_svg(path: &Path, max_side: u32) -> Result<DecodedImage, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let options = resvg::usvg::Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(&data, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let original = (size.width().ceil() as u32, size.height().ceil() as u32);
    let longest = original.0.max(original.1).max(1);
    let scale = (max_side as f32 / longest as f32).min(1.0);
    let width = ((size.width() * scale).ceil() as u32).max(1);
    let height = ((size.height() * scale).ceil() as u32).max(1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("SVG 크기가 잘못됨: {width}×{height}"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia는 미리 곱한 알파 — egui 텍스처에는 곧은 알파로 넘긴다
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Ok(DecodedImage {
        width,
        height,
        original,
        rgba,
    })
}

/// 파일 헤더만 읽어 원본 크기를 빠르게 얻는다 (SVG·읽기 실패는 `None`).
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    let is_svg = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    if is_svg {
        return None;
    }
    image::image_dimensions(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_remote_local_and_unsupported() {
        let dir = Path::new("/docs/notes");
        assert_eq!(
            resolve("https://example.com/a.png", Some(dir)),
            ImageSource::Remote("https://example.com/a.png".into())
        );
        assert_eq!(
            resolve("//cdn.example.com/a.png", Some(dir)),
            ImageSource::Remote("//cdn.example.com/a.png".into())
        );
        assert_eq!(
            resolve("images/내 그림%201.png?v=2", Some(dir)),
            ImageSource::Local(dir.join("images/내 그림 1.png"))
        );
        assert!(matches!(
            resolve("data:image/png;base64,AAAA", Some(dir)),
            ImageSource::Unsupported(_)
        ));
        assert!(matches!(
            resolve("a.png", None),
            ImageSource::Unsupported(_)
        ));
        #[cfg(windows)]
        {
            assert_eq!(
                resolve("C:\\pics\\a.png", None),
                ImageSource::Local(PathBuf::from("C:\\pics\\a.png"))
            );
            assert_eq!(
                resolve("file:///C:/pics/a%20b.png", None),
                ImageSource::Local(PathBuf::from("C:/pics/a b.png"))
            );
        }
        #[cfg(not(windows))]
        assert_eq!(
            resolve("file:///home/me/a%20b.png", None),
            ImageSource::Local(PathBuf::from("/home/me/a b.png"))
        );
    }

    #[test]
    fn large_image_is_decoded_downscaled_keeping_ratio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.png");
        image::RgbaImage::from_pixel(4000, 1000, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();
        assert_eq!(image_size(&path), Some((4000, 1000)));
        let decoded = decode_file(&path, 2048).unwrap();
        assert_eq!(decoded.original, (4000, 1000));
        assert_eq!((decoded.width, decoded.height), (2048, 512));
        assert_eq!(decoded.rgba.len(), 2048 * 512 * 4);
        assert_eq!(&decoded.rgba[..4], &[10, 20, 30, 255]);

        let small = dir.path().join("small.png");
        image::RgbaImage::from_pixel(30, 20, image::Rgba([0, 0, 0, 255]))
            .save(&small)
            .unwrap();
        let decoded = decode_file(&small, 2048).unwrap();
        assert_eq!((decoded.width, decoded.height), (30, 20));
    }

    #[test]
    fn svg_is_rasterized_and_scaled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="100"><rect width="400" height="100" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        let decoded = decode_file(&path, 200).unwrap();
        assert_eq!(decoded.original, (400, 100));
        assert_eq!((decoded.width, decoded.height), (200, 50));
        assert_eq!(&decoded.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn broken_file_reports_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.png");
        std::fs::write(&path, b"not an image").unwrap();
        assert!(decode_file(&path, 2048).is_err());
        assert!(decode_file(&dir.path().join("none.png"), 2048).is_err());
    }
}
