//! 谱线采样：min/max 包络与放大后的插值。

use mnv_core::{Spectrum1D, ViewRange};

/// 一列像素的采样结果。
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub ppm: f64,
    pub min: f32,
    pub max: f32,
    pub last: f32,
}

/// 按目标列数生成谱线采样点。
///
/// * 缩小时（数据点远多于像素）：每列取 min/max，避免混叠；
/// * 放大时（像素多于数据点）：线性插值，保证谱线平滑。
pub fn build_trace(spec: &Spectrum1D, view: ViewRange, columns: usize) -> Vec<Column> {
    let columns = columns.max(2);
    let n = spec.y.len();
    if n < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(columns);

    let (a, b) = spec.x.index_range(view.from_ppm, view.to_ppm);
    if b <= a {
        return out;
    }
    let samples = b - a + 1;

    if samples >= columns {
        for k in 0..columns {
            let s = a + (k * samples) / columns;
            let e = (a + ((k + 1) * samples) / columns).min(b + 1).max(s + 1);
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            let mut last = 0.0;
            for v in &spec.y[s..e] {
                lo = lo.min(*v);
                hi = hi.max(*v);
                last = *v;
            }
            out.push(Column {
                ppm: spec.x.ppm_at(s),
                min: lo,
                max: hi,
                last,
            });
        }
    } else {
        for k in 0..columns {
            let f = k as f64 / (columns - 1) as f64;
            let ppm = view.from_ppm - f * view.span();
            let v = spec.value_at_ppm(ppm);
            out.push(Column {
                ppm,
                min: v,
                max: v,
                last: v,
            });
        }
    }
    out
}

/// 视口内的稳健纵轴范围：顶部取最大峰，底部取最小（允许少量负值）。
pub fn y_bounds(spec: &Spectrum1D, view: ViewRange) -> (f32, f32) {
    let (a, b) = spec.x.index_range(view.from_ppm, view.to_ppm);
    if b <= a {
        return (0.0, 1.0);
    }
    let mut hi = f32::MIN;
    let mut lo = f32::MAX;
    for v in &spec.y[a..=b] {
        hi = hi.max(*v);
        lo = lo.min(*v);
    }
    if hi <= lo {
        hi = lo + 1.0;
    }
    // 上下留余量：基线不贴底、峰顶不贴顶（峰标注还需要空间）
    let pad = (hi - lo) * 0.08;
    (lo - pad, hi + pad * 2.2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnv_core::{AcquisitionMeta, Axis};

    fn ramp(n: usize) -> Spectrum1D {
        let axis = Axis::new(10.0, 0.0, n);
        let y: Vec<f32> = (0..n).map(|i| i as f32).collect();
        Spectrum1D::new(axis, y, AcquisitionMeta::default())
    }

    #[test]
    fn downsampling_preserves_extremes() {
        let s = ramp(10_000);
        let cols = build_trace(&s, ViewRange::new(10.0, 0.0), 100);
        assert_eq!(cols.len(), 100);
        assert!(cols[0].min <= 1.0);
        assert!(cols[99].max >= 9_900.0);
    }

    #[test]
    fn upsampling_is_smooth() {
        let s = ramp(10);
        let cols = build_trace(&s, ViewRange::new(10.0, 0.0), 200);
        assert_eq!(cols.len(), 200);
        for w in cols.windows(2) {
            assert!((w[1].last - w[0].last).abs() < 10.0);
        }
    }

    #[test]
    fn y_bounds_pad_baseline() {
        let s = ramp(101);
        let (lo, hi) = y_bounds(&s, ViewRange::new(10.0, 0.0));
        assert!(lo < 0.0);
        assert!(hi > 100.0);
    }
}
