//! Markdown 미리보기 이미지 텍스처 캐시 (PLAN.md §3.8-4, P3-04).
//!
//! - 탭(미리보기)마다 하나. 경로별로 원본 크기·텍스처·오류를 기억한다.
//! - 크기는 파일 헤더만 읽어 바로 얻고(레이아웃이 처음부터 맞게), 디코딩은 작업 스레드에서 한다
//!   ([`core::image_load::decode_file`](crate::core::image_load::decode_file), 긴 변 [`MAX_DECODE_SIDE`]로 축소).
//! - 화면에 보일 때만 텍스처를 요청하고, [`RELEASE_AFTER_FRAMES`] 프레임 동안 보이지 않은 텍스처는 해제한다
//!   (크기는 남겨 레이아웃이 흔들리지 않게 하고, 다시 보이면 다시 디코딩한다).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions};

use crate::core::image_load::{self, DecodedImage};

/// 디코딩할 때 긴 변의 최대 픽셀 수 (더 크면 비율을 지켜 줄인다).
pub const MAX_DECODE_SIDE: u32 = 2048;
/// 이만큼 연속으로 보이지 않은 텍스처는 해제한다 (60fps 기준 약 2초).
pub const RELEASE_AFTER_FRAMES: u64 = 120;

#[derive(Default)]
struct Entry {
    /// 원본 크기 (px)
    size: Option<(u32, u32)>,
    texture: Option<TextureHandle>,
    loading: bool,
    error: Option<String>,
    /// 마지막으로 텍스처를 요청한 프레임
    last_seen: u64,
}

/// [`ImageCache::texture`] 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureState {
    Ready(TextureId),
    Loading,
    Failed(String),
}

pub struct ImageCache {
    entries: HashMap<PathBuf, Entry>,
    frame: u64,
    sender: Sender<(PathBuf, Result<DecodedImage, String>)>,
    receiver: Receiver<(PathBuf, Result<DecodedImage, String>)>,
}

impl std::fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageCache")
            .field("images", &self.entries.len())
            .field("textures", &self.texture_count())
            .field("frame", &self.frame)
            .finish()
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            entries: HashMap::new(),
            frame: 0,
            sender,
            receiver,
        }
    }
}

impl ImageCache {
    /// 프레임 시작: 끝난 디코딩 결과를 텍스처로 올린다.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        while let Ok((path, result)) = self.receiver.try_recv() {
            let entry = self.entries.entry(path.clone()).or_default();
            entry.loading = false;
            match result {
                Ok(image) => {
                    entry.size = Some(image.original);
                    let color = ColorImage::from_rgba_unmultiplied(
                        [image.width as usize, image.height as usize],
                        &image.rgba,
                    );
                    let name = format!("md-image:{}", path.display());
                    entry.texture = Some(ctx.load_texture(name, color, TextureOptions::LINEAR));
                    log::debug!(
                        "미리보기 이미지 텍스처 올림: {} ({}×{}, 원본 {}×{})",
                        path.display(),
                        image.width,
                        image.height,
                        image.original.0,
                        image.original.1
                    );
                }
                Err(e) => {
                    log::warn!("미리보기 이미지를 읽지 못함 ({}): {e}", path.display());
                    entry.error = Some(e);
                }
            }
        }
    }

    /// 프레임 끝: 오래 보이지 않은 텍스처를 해제한다. 해제한 개수를 돌려준다.
    pub fn end_frame(&mut self) -> usize {
        let mut released = 0;
        for (path, entry) in &mut self.entries {
            if entry.texture.is_some() && entry.last_seen + RELEASE_AFTER_FRAMES < self.frame {
                entry.texture = None;
                released += 1;
                log::info!("화면 밖 미리보기 이미지 텍스처 해제: {}", path.display());
            }
        }
        released
    }

    /// 원본 크기 (px). 처음이면 파일 헤더를 읽는다 — SVG는 디코딩이 끝나야 안다.
    /// 파일이 없으면 디코딩을 기다리지 않고 바로 오류로 기억한다 (첫 프레임부터 자리 표시).
    pub fn size_of(&mut self, path: &Path) -> Option<(u32, u32)> {
        let entry = self.entries.entry(path.to_path_buf()).or_default();
        if entry.size.is_none() && entry.error.is_none() && !entry.loading {
            entry.size = image_load::image_size(path);
            if entry.size.is_none() && !path.is_file() {
                entry.error = Some("파일이 없습니다".to_owned());
            }
        }
        entry.size
    }

    /// 화면에 보이는 이미지의 텍스처. 없으면 작업 스레드에 디코딩을 맡기고 `Loading`.
    pub fn texture(&mut self, ctx: &egui::Context, path: &Path) -> TextureState {
        let frame = self.frame;
        let entry = self.entries.entry(path.to_path_buf()).or_default();
        entry.last_seen = frame;
        if let Some(error) = &entry.error {
            return TextureState::Failed(error.clone());
        }
        if let Some(texture) = &entry.texture {
            return TextureState::Ready(texture.id());
        }
        if !entry.loading {
            entry.loading = true;
            let sender = self.sender.clone();
            let ctx = ctx.clone();
            let path = path.to_path_buf();
            std::thread::spawn(move || {
                let result = image_load::decode_file(&path, MAX_DECODE_SIDE);
                // 탭이 닫혀 받는 쪽이 없으면 버린다
                let _ = sender.send((path, result));
                ctx.request_repaint();
            });
        }
        TextureState::Loading
    }

    /// 읽기 실패한 이미지의 오류 (이미 알고 있을 때만).
    pub fn error_of(&self, path: &Path) -> Option<&str> {
        self.entries.get(path).and_then(|e| e.error.as_deref())
    }

    /// 지금 올라가 있는 텍스처 수.
    pub fn texture_count(&self) -> usize {
        self.entries
            .values()
            .filter(|e| e.texture.is_some())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_loaded(cache: &mut ImageCache, ctx: &egui::Context, path: &Path) -> TextureState {
        for _ in 0..500 {
            cache.begin_frame(ctx);
            let state = cache.texture(ctx, path);
            cache.end_frame();
            if state != TextureState::Loading {
                return state;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("디코딩이 끝나지 않음");
    }

    #[test]
    fn texture_loads_then_is_released_when_not_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        image::RgbaImage::from_pixel(64, 32, image::Rgba([1, 2, 3, 255]))
            .save(&path)
            .unwrap();
        let ctx = egui::Context::default();
        let mut cache = ImageCache::default();
        assert_eq!(cache.size_of(&path), Some((64, 32)));
        assert!(matches!(
            wait_loaded(&mut cache, &ctx, &path),
            TextureState::Ready(_)
        ));
        assert_eq!(cache.texture_count(), 1);

        // 보이지 않는 프레임이 이어지면 해제, 크기는 남는다
        let mut released = 0;
        for _ in 0..RELEASE_AFTER_FRAMES + 2 {
            cache.begin_frame(&ctx);
            released += cache.end_frame();
        }
        assert_eq!(released, 1);
        assert_eq!(cache.texture_count(), 0);
        assert_eq!(cache.size_of(&path), Some((64, 32)));

        // 다시 보이면 다시 디코딩
        assert!(matches!(
            wait_loaded(&mut cache, &ctx, &path),
            TextureState::Ready(_)
        ));
    }

    #[test]
    fn missing_or_broken_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut cache = ImageCache::default();
        let missing = dir.path().join("none.png");
        assert_eq!(cache.size_of(&missing), None);
        // 없는 파일은 디코딩을 기다리지 않고 바로 안다
        assert_eq!(cache.error_of(&missing), Some("파일이 없습니다"));
        assert!(matches!(
            wait_loaded(&mut cache, &ctx, &missing),
            TextureState::Failed(_)
        ));
        assert!(cache.error_of(&missing).is_some());
    }
}
