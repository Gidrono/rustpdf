//! Graphics helpers — ink smoothing, shape geometry, snap guides.

use doxo_document::{PageId, RectPts, RgbaColor, ShapeKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InkStroke {
    pub page: PageId,
    pub points: Vec<[f32; 2]>,
    pub color: RgbaColor,
    pub width: f32,
}

pub fn begin_stroke(page: PageId, color: RgbaColor, width: f32) -> InkStroke {
    InkStroke {
        page,
        points: Vec::new(),
        color,
        width,
    }
}

pub fn simplify_stroke(points: &[[f32; 2]], min_dist: f32) -> Vec<[f32; 2]> {
    if points.is_empty() {
        return Vec::new();
    }
    let mut out = vec![points[0]];
    for p in &points[1..] {
        let last = *out.last().unwrap();
        let dx = p[0] - last[0];
        let dy = p[1] - last[1];
        if (dx * dx + dy * dy).sqrt() >= min_dist {
            out.push(*p);
        }
    }
    if out.last() != points.last() {
        out.push(*points.last().unwrap());
    }
    out
}

pub fn normalize_shape_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> RectPts {
    RectPts {
        x: x0.min(x1),
        y: y0.min(y1),
        w: (x1 - x0).abs().max(1.0),
        h: (y1 - y0).abs().max(1.0),
    }
}

pub fn shape_label(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Rectangle => "Rectangle",
        ShapeKind::Ellipse => "Ellipse",
        ShapeKind::Line => "Line",
        ShapeKind::Arrow => "Arrow",
    }
}

pub fn begin_stroke_stub(page: PageId) -> InkStroke {
    begin_stroke(page, RgbaColor::RED, 2.0)
}

/// Alignment guide in PDF page space.
#[derive(Debug, Clone, Copy)]
pub struct SnapGuide {
    pub vertical: bool,
    pub pos: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapResult {
    pub dx: f32,
    pub dy: f32,
    pub guides: [Option<SnapGuide>; 4],
}

const SNAP_THRESHOLD: f32 = 6.0;

/// Snap moving rect against page bounds and other rects (edges + centers).
pub fn snap_rect_move(
    moving: RectPts,
    dx: f32,
    dy: f32,
    page_w: f32,
    page_h: f32,
    others: &[RectPts],
) -> SnapResult {
    let mut nx = moving.x + dx;
    let mut ny = moving.y + dy;
    let mut guides = [None; 4];
    let mut gi = 0usize;

    let mut push = |g: SnapGuide| {
        if gi < guides.len() {
            guides[gi] = Some(g);
            gi += 1;
        }
    };

    let targets_x = |r: RectPts| [r.x, r.x + r.w * 0.5, r.x + r.w];
    let targets_y = |r: RectPts| [r.y, r.y + r.h * 0.5, r.y + r.h];

    let mut x_candidates = vec![0.0, page_w * 0.5, page_w];
    let mut y_candidates = vec![0.0, page_h * 0.5, page_h];
    for o in others {
        x_candidates.extend_from_slice(&targets_x(*o));
        y_candidates.extend_from_slice(&targets_y(*o));
    }

    let moving_xs = [nx, nx + moving.w * 0.5, nx + moving.w];
    let moving_ys = [ny, ny + moving.h * 0.5, ny + moving.h];

    let mut best_dx: Option<(f32, f32)> = None; // (delta_to_apply, guide_pos)
    for mx in moving_xs {
        for &tx in &x_candidates {
            let d = tx - mx;
            if d.abs() <= SNAP_THRESHOLD {
                if best_dx.map(|(bd, _)| d.abs() < bd.abs()).unwrap_or(true) {
                    best_dx = Some((d, tx));
                }
            }
        }
    }
    if let Some((d, pos)) = best_dx {
        nx += d;
        push(SnapGuide {
            vertical: true,
            pos,
        });
    }

    let mut best_dy: Option<(f32, f32)> = None;
    for my in moving_ys {
        for &ty in &y_candidates {
            let d = ty - my;
            if d.abs() <= SNAP_THRESHOLD {
                if best_dy.map(|(bd, _)| d.abs() < bd.abs()).unwrap_or(true) {
                    best_dy = Some((d, ty));
                }
            }
        }
    }
    if let Some((d, pos)) = best_dy {
        ny += d;
        push(SnapGuide {
            vertical: false,
            pos,
        });
    }

    SnapResult {
        dx: nx - moving.x,
        dy: ny - moving.y,
        guides,
    }
}
