//! 积分：区间求和 + 积分曲线。
//!
//! 采用“两端点连线作为局部基线”的线性基线扣除（与常规手动积分一致），
//! 梯形法累加。曲线按区间总面积归一化到用户指定数值（如 3H）。

use mnv_core::{IntegralRegion, Spectrum1D};

/// 区间积分和（已扣除两端连线基线）。
pub fn region_sum(spec: &Spectrum1D, from_ppm: f64, to_ppm: f64) -> f64 {
    let (a, b) = spec.x.index_range(from_ppm, to_ppm);
    if b <= a {
        return 0.0;
    }
    let y0 = spec.y[a] as f64;
    let y1 = spec.y[b] as f64;
    let n = (b - a) as f64;
    let mut sum = 0.0;
    for i in a..b {
        let t0 = (i - a) as f64 / n;
        let t1 = (i + 1 - a) as f64 / n;
        let base0 = y0 + (y1 - y0) * t0;
        let base1 = y0 + (y1 - y0) * t1;
        let v0 = spec.y[i] as f64 - base0;
        let v1 = spec.y[i + 1] as f64 - base1;
        sum += 0.5 * (v0 + v1);
    }
    sum
}

/// 新建一个积分区间（数值先设为 1.0，随后由用户或“按氢数归一”调整）。
pub fn new_region(spec: &Spectrum1D, from_ppm: f64, to_ppm: f64) -> IntegralRegion {
    let raw = region_sum(spec, from_ppm, to_ppm);
    IntegralRegion::new(from_ppm.max(to_ppm), from_ppm.min(to_ppm), raw)
}

/// 积分曲线：返回 `(ppm, 高度)` 的点列，按 ppm 降序（与扫描顺序一致）。
///
/// 高度语义：`curve_height` 是“1H 在 1.0 数值时的显示高度”，
/// 因此曲线终点高度 ∝ `value`（3H 的曲线是 1H 的三倍高），便于目视比较。
pub fn curve_points(spec: &Spectrum1D, region: &IntegralRegion, samples: usize) -> Vec<(f64, f32)> {
    let (a, b) = spec.x.index_range(region.from_ppm, region.to_ppm);
    if b <= a {
        return Vec::new();
    }
    let n = b - a;
    let step = ((n / samples.max(2)).max(1)) as usize;
    let y0 = spec.y[a] as f64;
    let y1 = spec.y[b] as f64;
    let raw_total = region.raw_sum.abs().max(1e-12);
    let scale = region.curve_height as f64 * region.value.max(0.0) as f64 / raw_total;

    let mut acc = 0.0f64;
    let mut out = Vec::with_capacity(n / step + 2);
    out.push((spec.x.ppm_at(a), 0.0));
    let mut i = a;
    while i < b {
        let end = (i + step).min(b);
        for k in i..end {
            let t0 = (k - a) as f64 / n as f64;
            let t1 = (k + 1 - a) as f64 / n as f64;
            let base0 = y0 + (y1 - y0) * t0;
            let base1 = y0 + (y1 - y0) * t1;
            let v0 = spec.y[k] as f64 - base0;
            let v1 = spec.y[k + 1] as f64 - base1;
            acc += 0.5 * (v0 + v1);
        }
        out.push((spec.x.ppm_at(end), (acc * scale) as f32));
        i = end;
    }
    out
}

/// 按面积把积分值“凑”成最接近的整数比 —— 这是化学上最常见的做法
/// （先积分，再按最小峰为基准放大，直到各峰接近整数）。
///
/// 返回实际采用的缩放因子；`candidates` 通常取 `[1, 2, 3, 4]`，
/// 例如比例 1 : 1.5 : 1.5 会选 2 → 2H : 3H : 3H。
pub fn assign_by_area(regions: &mut [IntegralRegion], candidates: &[f32]) -> f32 {
    let areas: Vec<f64> = regions.iter().map(|r| r.raw_sum.abs()).collect();
    let max = areas.iter().cloned().fold(0.0f64, f64::max);
    if max <= 0.0 {
        return 1.0;
    }
    // 只用“主信号”（≥ 最大面积的 10%）确定基准，避免水峰/内标把比例带偏
    let cutoff = max * 0.1;
    let base_min = areas
        .iter()
        .filter(|a| **a >= cutoff)
        .cloned()
        .fold(f64::MAX, f64::min);
    let base_min = if base_min.is_finite() { base_min } else { max };
    let ratios: Vec<f64> = areas.iter().map(|a| a / base_min).collect();

    // 先算出每个候选缩放因子的“取整误差”，再取误差接近最小值的**最小**缩放因子：
    // 化学上更合理（2H:3H:3H 优先于 4H:6H:6H）。
    let mut errors: Vec<(f32, f64)> = candidates
        .iter()
        .map(|s| {
            let sf = *s as f64;
            let err: f64 = areas
                .iter()
                .zip(ratios.iter())
                .filter(|(a, _)| **a >= cutoff)
                .map(|(_, ratio)| {
                    let v = ratio * sf;
                    (v - v.round()).abs()
                })
                .sum();
            (*s, err)
        })
        .collect();
    errors.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let min_err = errors.first().map(|e| e.1).unwrap_or(f64::MAX);
    let best = errors
        .iter()
        .filter(|(_, e)| *e <= min_err + 0.15)
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .copied()
        .unwrap_or((1.0, min_err));
    for (r, ratio) in regions.iter_mut().zip(ratios.iter()) {
        let v = ratio * best.0 as f64;
        r.value = if v >= 1.0 {
            v.round() as f32
        } else {
            ((v * 10.0).round() / 10.0).max(0.1) as f32
        };
        r.edited = false;
    }
    best.0
}

/// 按最接近的整数比对全部积分做归一化（常用于 ¹H）。
pub fn normalize_to_integers(regions: &mut [IntegralRegion], target_total: f32) {
    let total: f32 = regions.iter().map(|r| r.value).sum();
    if total <= 0.0 || regions.is_empty() {
        return;
    }
    let scale = target_total / total;
    for r in regions.iter_mut() {
        r.value = (r.value * scale).round().max(1.0);
        r.edited = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnv_core::{AcquisitionMeta, Axis};

    fn flat_spectrum() -> Spectrum1D {
        // 4..6 ppm 之间有一个高 1、宽 1 ppm 的矩形峰，其余为 0
        let n = 1001;
        let axis = Axis::new(10.0, 0.0, n);
        let y: Vec<f32> = (0..n)
            .map(|i| {
                let ppm = axis.ppm_at(i);
                if (4.0..=5.0).contains(&ppm) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        Spectrum1D::new(axis, y, AcquisitionMeta::default())
    }

    #[test]
    fn rectangle_area_is_width_times_height() {
        let s = flat_spectrum();
        let area = region_sum(&s, 3.5, 5.5);
        // 100 个点高 1 → 面积 ≈ 100（点数口径），与梯形法一致
        assert!((area - 100.0).abs() < 2.0, "area={area}");
    }

    #[test]
    fn curve_is_monotonic_and_normalized() {
        let s = flat_spectrum();
        let region = new_region(&s, 3.5, 5.5);
        let pts = curve_points(&s, &region, 50);
        assert!(!pts.is_empty());
        let first = pts[0].1;
        let last = pts[pts.len() - 1].1;
        assert!(first.abs() < 1e-6, "起点贴近基线");
        // 终点高度 = curve_height × value（value 默认 1.0）
        assert!((last - region.curve_height).abs() < 1e-3, "last={last}");
        for w in pts.windows(2) {
            assert!(w[1].1 >= w[0].1 - 1e-4, "积分曲线应单调不降");
        }
    }
}
