//! P0-09 스크롤 동기화 대응 계산 (UI와 무관한 순수 함수).
//!
//! - 소스: 줄마다의 y(위쪽) 목록 → "y ↔ 소수점 줄 번호"
//! - 미리보기: 블록 위치표(소스 줄 범위 + y·높이) → "소수점 줄 번호 ↔ y"
//!   블록 안은 줄 범위 비율로, 블록 사이(빈 줄)는 앞 블록 끝 ~ 다음 블록 시작을 선형 보간한다.

use crate::render::BlockPos;

/// 소스 쪽 줄 위치표: `tops[i]` = i번째 줄의 위쪽 y, `total` = 전체 높이
pub struct SourceLines {
    pub tops: Vec<f32>,
    pub total: f32,
}

impl SourceLines {
    /// y → 소수점 줄 번호 (줄 안에서의 비율 포함)
    pub fn line_at(&self, y: f32) -> f32 {
        if self.tops.is_empty() {
            return 0.0;
        }
        let i = self.tops.partition_point(|&t| t <= y).saturating_sub(1);
        let top = self.tops[i];
        let bottom = self.tops.get(i + 1).copied().unwrap_or(self.total);
        let frac = if bottom > top {
            ((y - top) / (bottom - top)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        i as f32 + frac
    }

    /// 소수점 줄 번호 → y
    pub fn y_of_line(&self, line: f32) -> f32 {
        if self.tops.is_empty() {
            return 0.0;
        }
        let line = line.max(0.0);
        let i = (line.floor() as usize).min(self.tops.len() - 1);
        let top = self.tops[i];
        let bottom = self.tops.get(i + 1).copied().unwrap_or(self.total);
        top + (bottom - top) * (line - i as f32).clamp(0.0, 1.0)
    }
}

/// 미리보기 쪽 구간: 소스 줄 [l0, l1) ↔ y [y0, y1)
#[derive(Clone, Copy, Debug)]
struct Segment {
    l0: f32,
    l1: f32,
    y0: f32,
    y1: f32,
}

/// 블록 위치표를 "블록 구간 + 블록 사이 간격 구간"의 연속된 목록으로 만든다.
fn segments(blocks: &[BlockPos], line_count: usize, total: f32) -> Vec<Segment> {
    let mut segs = Vec::with_capacity(blocks.len() * 2 + 1);
    let (mut l, mut y) = (0.0_f32, 0.0_f32);
    for b in blocks {
        let (bl0, bl1) = (b.first_line as f32, b.last_line as f32 + 1.0);
        let (by0, by1) = (b.top, b.top + b.height);
        if bl0 > l || by0 > y {
            segs.push(Segment {
                l0: l,
                l1: bl0.max(l),
                y0: y,
                y1: by0.max(y),
            });
        }
        segs.push(Segment {
            l0: bl0.max(l),
            l1: bl1.max(l),
            y0: by0.max(y),
            y1: by1.max(y),
        });
        l = bl1.max(l);
        y = by1.max(y);
    }
    segs.push(Segment {
        l0: l,
        l1: (line_count as f32).max(l),
        y0: y,
        y1: total.max(y),
    });
    segs
}

fn lerp(from0: f32, from1: f32, to0: f32, to1: f32, v: f32) -> f32 {
    if from1 > from0 {
        to0 + (to1 - to0) * ((v - from0) / (from1 - from0)).clamp(0.0, 1.0)
    } else {
        to0
    }
}

/// 소스 줄(소수점) → 미리보기 y
pub fn preview_y_of_line(blocks: &[BlockPos], line_count: usize, total: f32, line: f32) -> f32 {
    let segs = segments(blocks, line_count, total);
    let s = segs
        .iter()
        .rev()
        .find(|s| s.l0 <= line && s.l1 > s.l0)
        .unwrap_or(&segs[0]);
    lerp(s.l0, s.l1, s.y0, s.y1, line)
}

/// 미리보기 y → 소스 줄(소수점)
pub fn line_of_preview_y(blocks: &[BlockPos], line_count: usize, total: f32, y: f32) -> f32 {
    let segs = segments(blocks, line_count, total);
    let s = segs
        .iter()
        .rev()
        .find(|s| s.y0 <= y && s.y1 > s.y0)
        .unwrap_or(&segs[0]);
    lerp(s.y0, s.y1, s.l0, s.l1, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(index: usize, first: usize, last: usize, top: f32, height: f32) -> BlockPos {
        BlockPos {
            index,
            kind: "문단",
            first_line: first,
            last_line: last,
            top,
            height,
        }
    }

    /// 줄 0-1 제목(0~40), 빈 줄 2, 줄 3-9 표(60~360), 빈 줄 10, 줄 11 문단(380~400)
    fn sample() -> Vec<BlockPos> {
        vec![
            pos(0, 0, 1, 0.0, 40.0),
            pos(1, 3, 9, 60.0, 300.0),
            pos(2, 11, 11, 380.0, 20.0),
        ]
    }

    #[test]
    fn block_edges_map_exactly() {
        let b = sample();
        assert_eq!(preview_y_of_line(&b, 12, 400.0, 0.0), 0.0);
        assert_eq!(preview_y_of_line(&b, 12, 400.0, 3.0), 60.0);
        assert_eq!(preview_y_of_line(&b, 12, 400.0, 11.0), 380.0);
        assert_eq!(line_of_preview_y(&b, 12, 400.0, 60.0), 3.0);
    }

    #[test]
    fn inside_block_is_proportional() {
        let b = sample();
        // 표: 줄 3~10(7줄) ↔ y 60~360(300px) → 줄 6.5는 중간 = 210
        assert!((preview_y_of_line(&b, 12, 400.0, 6.5) - 210.0).abs() < 0.01);
    }

    #[test]
    fn gap_between_blocks_is_interpolated() {
        let b = sample();
        // 빈 줄 2(줄 2~3) ↔ y 40~60
        assert!((preview_y_of_line(&b, 12, 400.0, 2.5) - 50.0).abs() < 0.01);
    }

    #[test]
    fn round_trip_is_stable_and_monotonic() {
        let b = sample();
        let mut prev = -1.0;
        let mut line = 0.0;
        while line < 12.0 {
            let y = preview_y_of_line(&b, 12, 400.0, line);
            assert!(y >= prev, "y가 감소: line {line}");
            prev = y;
            let back = line_of_preview_y(&b, 12, 400.0, y);
            assert!(
                (back - line).abs() < 0.01,
                "왕복 오차: {line} → {y} → {back}"
            );
            line += 0.25;
        }
    }

    #[test]
    fn source_lines_round_trip() {
        let src = SourceLines {
            tops: vec![0.0, 20.0, 40.0, 60.0],
            total: 80.0,
        };
        assert_eq!(src.line_at(30.0), 1.5);
        assert_eq!(src.y_of_line(1.5), 30.0);
        assert_eq!(src.line_at(79.0), 3.95);
    }
}
