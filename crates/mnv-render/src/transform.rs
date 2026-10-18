//! 屏幕坐标与 ppm/强度之间的换算。

use mnv_core::ViewRange;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ScreenRect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.right() && py >= self.y && py <= self.bottom()
    }

    pub fn inset(&self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            w: (self.w - dx * 2.0).max(1.0),
            h: (self.h - dy * 2.0).max(1.0),
        }
    }

    /// 在矩形内侧四周留出边距（左侧给纵轴、底部给横轴）。
    pub fn plot_area(&self, left: f32, right: f32, top: f32, bottom: f32) -> Self {
        Self {
            x: self.x + left,
            y: self.y + top,
            w: (self.w - left - right).max(1.0),
            h: (self.h - top - bottom).max(1.0),
        }
    }
}

/// 谱图坐标变换：ppm → x 像素，强度 → y 像素。
#[derive(Clone, Debug)]
pub struct Transform {
    pub rect: ScreenRect,
    pub view: ViewRange,
    /// 底部（基线）对应的强度值
    pub y_min: f32,
    /// 顶部对应的强度值
    pub y_max: f32,
}

impl Transform {
    pub fn new(rect: ScreenRect, view: ViewRange, y_min: f32, y_max: f32) -> Self {
        let (lo, hi) = if y_max > y_min {
            (y_min, y_max)
        } else {
            (y_min, y_min + 1.0)
        };
        Self {
            rect,
            view,
            y_min: lo,
            y_max: hi,
        }
    }

    pub fn x_of_ppm(&self, ppm: f64) -> f32 {
        let span = self.view.span().max(1e-9);
        let f = (self.view.from_ppm - ppm) / span;
        self.rect.x + (f as f32) * self.rect.w
    }

    pub fn ppm_of_x(&self, x: f32) -> f64 {
        let span = self.view.span();
        if self.rect.w <= 0.0 {
            return self.view.from_ppm;
        }
        let f = ((x - self.rect.x) / self.rect.w).clamp(0.0, 1.0) as f64;
        self.view.from_ppm - f * span
    }

    pub fn y_of(&self, v: f32) -> f32 {
        let span = (self.y_max - self.y_min).max(1e-9);
        let f = ((v - self.y_min) / span).clamp(-0.2, 1.2);
        self.rect.bottom() - f * self.rect.h
    }

    pub fn value_of_y(&self, y: f32) -> f32 {
        if self.rect.h <= 0.0 {
            return self.y_min;
        }
        let f = ((self.rect.bottom() - y) / self.rect.h).clamp(0.0, 1.0);
        self.y_min + f * (self.y_max - self.y_min)
    }

    /// 每像素对应的 ppm（用于拖拽灵敏度与最小选区宽度）。
    pub fn ppm_per_px(&self) -> f64 {
        if self.rect.w <= 0.0 {
            0.0
        } else {
            self.view.span() / self.rect.w as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppm_maps_left_to_right() {
        let t = Transform::new(
            ScreenRect::new(0.0, 0.0, 100.0, 50.0),
            ViewRange::new(10.0, 0.0),
            -1.0,
            9.0,
        );
        assert!((t.x_of_ppm(10.0) - 0.0).abs() < 1e-4);
        assert!((t.x_of_ppm(0.0) - 100.0).abs() < 1e-4);
        assert!((t.x_of_ppm(5.0) - 50.0).abs() < 1e-4);
    }

    #[test]
    fn round_trip() {
        let t = Transform::new(
            ScreenRect::new(10.0, 20.0, 300.0, 100.0),
            ViewRange::new(12.0, -1.0),
            -0.5,
            4.0,
        );
        for ppm in [12.0, 8.5, 3.0, -1.0] {
            let back = t.ppm_of_x(t.x_of_ppm(ppm));
            assert!((back - ppm).abs() < 1e-4, "{ppm} → {back}");
        }
    }
}
