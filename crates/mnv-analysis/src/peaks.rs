//! 峰拾取。
//!
//! 算法：一阶差分找局部极大 → 抛物线插值细化位置与高度 → 半高宽估计 → 距离合并 → 分类。
//! 全部为通用信号处理方法（教科书内容），不依赖任何商业软件的实现。

use mnv_core::{solvent, Nucleus, Peak, PeakKind, PeakSource, Spectrum1D};

#[derive(Clone, Debug)]
pub struct PickParams {
    /// 相对于全谱最大峰的阈值（默认 0.5%）
    pub threshold_rel: f32,
    /// 噪声倍数阈值（与相对阈值取较大者）
    pub noise_sigma: f32,
    /// 最小峰间距（Hz），用于合并肩峰
    pub min_distance_hz: f64,
    /// 只在指定 ppm 区间内拾取
    pub region: Option<(f64, f64)>,
    /// 溶剂峰识别容差（ppm）
    pub solvent_tolerance_ppm: f64,
}

impl Default for PickParams {
    fn default() -> Self {
        Self {
            threshold_rel: 0.005,
            noise_sigma: 6.0,
            min_distance_hz: 3.0,
            region: None,
            solvent_tolerance_ppm: 0.05,
        }
    }
}

/// 在给定谱图上拾取峰。
pub fn pick(spec: &Spectrum1D, params: &PickParams) -> Vec<Peak> {
    let n = spec.y.len();
    if n < 5 {
        return Vec::new();
    }
    let (a, b) = match params.region {
        Some((from, to)) => spec.x.index_range(from, to),
        None => (1, n - 2),
    };
    let a = a.max(1);
    let b = b.min(n - 2);
    if b <= a {
        return Vec::new();
    }

    let global_max = spec.y[a..=b].iter().fold(0.0f32, |m, v| m.max(*v));
    let noise = spec.noise_sigma() * params.noise_sigma;
    let threshold = (global_max * params.threshold_rel).max(noise).max(1e-6);

    let sw = spec.meta.spectral_width_hz.max(1e-9);
    let hz_per_point = sw / n as f64;

    let mut out: Vec<Peak> = Vec::new();

    for i in a..=b {
        let y = spec.y[i];
        if y < threshold {
            continue;
        }
        if !(y > spec.y[i - 1] && y >= spec.y[i + 1]) {
            continue;
        }
        // 抛物线插值：Δ = 0.5·(y₋ - y₊)/(y₋ - 2y + y₊)
        let yl = spec.y[i - 1] as f64;
        let yr = spec.y[i + 1] as f64;
        let yc = y as f64;
        let denom = yl - 2.0 * yc + yr;
        let delta = if denom.abs() > 1e-12 {
            (0.5 * (yl - yr) / denom).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let refined_h = yc - 0.25 * (yl - yr) * delta;
        let ppm = spec.x.ppm_at(i) - delta * spec.x.ppm_per_point();

        // 半高宽：向两侧找首次跌破半高的位置
        let half = refined_h as f32 * 0.5;
        let mut li = i;
        while li > a && spec.y[li] > half {
            li -= 1;
        }
        let mut ri = i;
        while ri < b && spec.y[ri] > half {
            ri += 1;
        }
        let width_pts = (ri - li).max(1) as f64;
        let width_hz = width_pts * hz_per_point;

        // 只有明显弱于主峰的疑似残余峰才自动归类为溶剂/杂质：
        // 强信号（如乙酸乙酯 2.04 ppm 的 CH₃CO，与丙酮 2.05 几乎重合）需人工确认。
        let kind = if (refined_h as f32) < global_max * 0.25 {
            classify(spec, ppm, params.solvent_tolerance_ppm)
        } else {
            PeakKind::Compound
        };
        let mut peak = Peak::new(ppm, refined_h as f32);
        peak.width_hz = width_hz as f32;
        peak.kind = kind;
        peak.source = PeakSource::Automatic;
        out.push(peak);
    }

    // 距离合并：同向保留更高者
    out.sort_by(|p, q| q.ppm.partial_cmp(&p.ppm).unwrap_or(std::cmp::Ordering::Equal));
    let min_gap_ppm = params.min_distance_hz / spec.meta.spectrometer_mhz.max(1e-9);
    let mut merged: Vec<Peak> = Vec::new();
    for p in out {
        match merged.last_mut() {
            Some(last) if (last.ppm - p.ppm).abs() < min_gap_ppm => {
                if p.height > last.height {
                    *last = p;
                }
            }
            _ => merged.push(p),
        }
    }
    merged
}

fn classify(spec: &Spectrum1D, ppm: f64, tol: f64) -> PeakKind {
    if let Some(_name) = solvent::match_residual(&spec.meta.nucleus, ppm, tol) {
        return PeakKind::Solvent;
    }
    // 内标 TMS / DSS 视为样品区内的参考峰
    if matches!(spec.meta.nucleus, Nucleus::H1 | Nucleus::C13) && ppm.abs() < tol * 0.6 {
        return PeakKind::Compound;
    }
    PeakKind::Compound
}

/// 把溶剂/杂质峰标记出来（用于“标峰后标红”的交互）。
pub fn mark_solvent(spec: &Spectrum1D, peaks: &mut [Peak], tolerance_ppm: f64) {
    for p in peaks.iter_mut() {
        if solvent::match_residual(&spec.meta.nucleus, p.ppm, tolerance_ppm).is_some() {
            p.kind = PeakKind::Solvent;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnv_core::{AcquisitionMeta, Axis};

    fn gaussian_spectrum(centers: &[(f64, f32)], width: f32) -> Spectrum1D {
        let n = 4096;
        let start = 10.0;
        let end = 0.0;
        let axis = Axis::new(start, end, n);
        let y: Vec<f32> = (0..n)
            .map(|i| {
                let ppm = axis.ppm_at(i);
                centers
                    .iter()
                    .map(|(c, amp)| amp * (-((ppm - c) as f32 / width).powi(2)).exp())
                    .sum()
            })
            .collect();
        let meta = AcquisitionMeta {
            spectrometer_mhz: 400.0,
            spectral_width_hz: 4000.0,
            ..Default::default()
        };
        Spectrum1D::new(axis, y, meta)
    }

    #[test]
    fn finds_three_peaks() {
        let s = gaussian_spectrum(&[(7.26, 0.05), (4.12, 1.0), (1.26, 0.9)], 0.01);
        let peaks = pick(&s, &PickParams::default());
        assert_eq!(peaks.len(), 3, "peaks={:?}", peaks.iter().map(|p| p.ppm).collect::<Vec<_>>());
        for p in &peaks {
            assert!(p.ppm > 1.0 && p.ppm < 7.5);
        }
    }

    #[test]
    fn classifies_chloroform_as_solvent() {
        let s = gaussian_spectrum(&[(7.26, 0.05), (2.04, 1.0)], 0.01);
        let peaks = pick(&s, &PickParams::default());
        let solvent_peak = peaks.iter().find(|p| (p.ppm - 7.26).abs() < 0.05).unwrap();
        assert_eq!(solvent_peak.kind, PeakKind::Solvent);
        // 强信号即使与溶剂/杂质位移重合，也不应被自动判为溶剂
        let strong = peaks.iter().find(|p| (p.ppm - 2.04).abs() < 0.05).unwrap();
        assert_eq!(strong.kind, PeakKind::Compound);
    }

    #[test]
    fn merges_close_peaks() {
        let s = gaussian_spectrum(&[(4.120, 1.0), (4.124, 1.0)], 0.01);
        let peaks = pick(&s, &PickParams::default());
        assert_eq!(peaks.len(), 1);
    }
}
