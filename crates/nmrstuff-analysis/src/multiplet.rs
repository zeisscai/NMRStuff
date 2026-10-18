//! 多重峰解算（一阶近似）。
//!
//! 本实现刻意采用**可解释的启发式**，而不是黑箱拟合：
//!
//! 1. 候选峰筛选：区域内、高于相对阈值、剔除溶剂/伪峰；
//! 2. 子峰聚类：相邻间距 > `max_gap_hz` 视为另一个多重峰；
//! 3. 基频估计：相邻间距的中位数作为 J（一阶模型下等间距）；
//! 4. 强度模式检验：与 Pascal 三角形比较，偏差过大则判为 `m` 并提示高阶耦合；
//! 5. 输出残差 `fit_residual_hz`，供 UI 提示“这组峰不太规整”。
//!
//! 能力边界：**不做** AB/ABX 的量子力学拟合（v2 规划），遇到强耦合只会给 `m`。

use nmrstuff_core::{Multiplet, MultipletLabel, Peak, PeakKind, Spectrum1D};

use crate::integrate::region_sum;

#[derive(Clone, Debug)]
pub struct MultipletParams {
    /// 同一多重峰内相邻子峰的最大间距（Hz）
    pub max_gap_hz: f64,
    /// 判定等间距的容差（Hz）
    pub j_tolerance_hz: f64,
    /// 相对区域内最高峰的阈值
    pub min_height_rel: f32,
}

impl Default for MultipletParams {
    fn default() -> Self {
        Self {
            max_gap_hz: 25.0,
            j_tolerance_hz: 0.5,
            min_height_rel: 0.02,
        }
    }
}

/// 解算一个选区内的全部多重峰。
pub fn analyze_region(
    spec: &Spectrum1D,
    peaks: &[Peak],
    from_ppm: f64,
    to_ppm: f64,
    params: &MultipletParams,
) -> Vec<Multiplet> {
    let hi = from_ppm.max(to_ppm);
    let lo = from_ppm.min(to_ppm);
    let mut idx: Vec<usize> = peaks
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.ppm <= hi
                && p.ppm >= lo
                && !matches!(p.kind, PeakKind::Solvent | PeakKind::Artifact)
        })
        .map(|(i, _)| i)
        .collect();
    if idx.is_empty() {
        return Vec::new();
    }
    idx.sort_by(|a, b| {
        peaks[*b]
            .ppm
            .partial_cmp(&peaks[*a].ppm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let max_h = idx
        .iter()
        .map(|i| peaks[*i].height)
        .fold(0.0f32, f32::max);
    let threshold = max_h * params.min_height_rel;
    idx.retain(|i| peaks[*i].height >= threshold);

    if idx.is_empty() {
        return Vec::new();
    }

    let sfo = spec.meta.spectrometer_mhz.max(1e-9);
    let max_gap_ppm = params.max_gap_hz / sfo;

    // 聚类
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for i in idx {
        match clusters.last_mut() {
            Some(c) if (peaks[c[c.len() - 1]].ppm - peaks[i].ppm) <= max_gap_ppm => c.push(i),
            _ => clusters.push(vec![i]),
        }
    }

    clusters
        .into_iter()
        .map(|cluster| build_multiplet(spec, peaks, &cluster, sfo, params))
        .collect()
}

fn build_multiplet(
    spec: &Spectrum1D,
    peaks: &[Peak],
    cluster: &[usize],
    sfo: f64,
    params: &MultipletParams,
) -> Multiplet {
    // 1) 簇内再去一次肩峰/噪声：只保留不低于簇内最高峰 12% 的子峰。
    //    这是“自动标峰不准”的直接对策 —— 宁可少标，也不要把肩峰算进 J。
    let cluster_max = cluster
        .iter()
        .map(|i| peaks[*i].height)
        .fold(0.0f32, f32::max);
    let kept: Vec<usize> = cluster
        .iter()
        .copied()
        .filter(|i| peaks[*i].height >= cluster_max * 0.12)
        .collect();
    let cluster: Vec<usize> = if kept.is_empty() {
        cluster.to_vec()
    } else {
        kept
    };

    let from_ppm = peaks[cluster[0]].ppm;
    let to_ppm = peaks[*cluster.last().unwrap()].ppm;
    let mut spacings: Vec<f64> = Vec::new();
    for w in cluster.windows(2) {
        spacings.push((peaks[w[0]].ppm - peaks[w[1]].ppm) * sfo);
    }

    let (label, j_values, residual) = if cluster.len() == 1 {
        (MultipletLabel::S, Vec::new(), 0.0)
    } else {
        // 2) 先用中位数剔掉明显偏离的间距（重叠峰/肩峰残留）
        if spacings.len() >= 3 {
            let med = median(&spacings);
            spacings.retain(|s| (s - med).abs() <= med * 0.35 + 0.3);
        }
        let n = spacings.len().max(1) as f64;
        let mean = spacings.iter().sum::<f64>() / n;
        let rms = (spacings.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / n).sqrt();
        let heights: Vec<f32> = cluster.iter().map(|i| peaks[*i].height).collect();
        let label = classify(&heights, mean, params, rms);
        let j = spacings.iter().map(|s| (s * 10.0).round() / 10.0).collect();
        (label, j, rms)
    };

    // 积分区间向外扩：既避免“两点连线削掉中间峰”，也要把洛伦兹尾收进来。
    // 扩边量取“最宽半高宽的 8 倍”和“簇宽的 35%”中较大者。
    let widest_hz = cluster
        .iter()
        .map(|i| peaks[*i].width_hz)
        .fold(0.0f32, f32::max)
        .max(1.0);
    let span = (from_ppm - to_ppm).abs();
    let pad = (widest_hz as f64 * 8.0 / sfo).max(span * 0.35).max(0.03);
    let raw = region_sum(spec, from_ppm + pad, to_ppm - pad);
    Multiplet {
        from_ppm,
        to_ppm,
        peaks: cluster.to_vec(),
        j_values_hz: j_values,
        integral: raw,
        integral_value: 1.0,
        label,
        assignment: None,
        fit_residual_hz: residual,
    }
}

fn classify(
    heights: &[f32],
    mean: f64,
    params: &MultipletParams,
    rms: f64,
) -> MultipletLabel {
    let n = heights.len();
    // 间距不整齐 → 视为复杂多重峰
    if rms > params.j_tolerance_hz && n > 2 {
        return MultipletLabel::M;
    }
    // 强度模式与 Pascal 三角形比较
    if let Some(expected) = pascal(n) {
        let hmax = heights.iter().cloned().fold(0.0f32, f32::max).max(1e-9);
        let emax = expected.iter().cloned().fold(0.0f64, f64::max);
        let mut worst = 0.0f64;
        for (h, e) in heights.iter().zip(expected.iter()) {
            let rel_h = (*h / hmax) as f64;
            let rel_e = e / emax;
            worst = worst.max((rel_h - rel_e).abs());
        }
        if worst > 0.35 {
            return MultipletLabel::M;
        }
    }
    // 间距非常小的情况（耦合常数接近线宽）不强行标注
    if mean < 0.8 {
        return MultipletLabel::M;
    }
    MultipletLabel::from_peak_count(n)
}

/// 全谱自动分组：按“相邻间距 > max_gap_hz”切段，每段独立解算。
///
/// 这是“自动给个初步结果”的入口；界面上的主路径仍是用户框选后调用 `analyze_region`。
pub fn multiplets_from_auto(
    spec: &Spectrum1D,
    peaks: &[Peak],
    params: &MultipletParams,
) -> Vec<Multiplet> {
    let mut idx: Vec<usize> = peaks
        .iter()
        .enumerate()
        .filter(|(_, p)| !matches!(p.kind, PeakKind::Solvent | PeakKind::Artifact))
        .map(|(i, _)| i)
        .collect();
    if idx.is_empty() {
        return Vec::new();
    }
    idx.sort_by(|a, b| {
        peaks[*b]
            .ppm
            .partial_cmp(&peaks[*a].ppm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let max_gap_ppm = params.max_gap_hz / spec.meta.spectrometer_mhz.max(1e-9);
    let mut clusters: Vec<(f64, f64)> = Vec::new();
    for i in idx {
        match clusters.last_mut() {
            Some(c) if (c.1 - peaks[i].ppm) <= max_gap_ppm => c.1 = peaks[i].ppm,
            _ => clusters.push((peaks[i].ppm, peaks[i].ppm)),
        }
    }
    let mut out = Vec::new();
    for (hi, lo) in clusters {
        // 单一孤峰间距窗口给一点余量，避免浮点边界丢峰
        let pad = max_gap_ppm * 0.01;
        let found = analyze_region(spec, peaks, hi + pad, lo - pad, params);
        if let Some(first) = found.into_iter().next() {
            out.push(first);
        }
    }
    out
}

fn median(v: &[f64]) -> f64 {
    let mut c = v.to_vec();
    c.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = c.len() / 2;
    if c.len() % 2 == 0 {
        (c[m - 1] + c[m]) * 0.5
    } else {
        c[m]
    }
}

fn pascal(n: usize) -> Option<Vec<f64>> {    if !(2..=6).contains(&n) {
        return None;
    }
    let mut row = vec![1.0f64];
    for _ in 1..n {
        let mut next = vec![1.0];
        for w in row.windows(2) {
            next.push(w[0] + w[1]);
        }
        next.push(1.0);
        row = next;
    }
    Some(row)
}

#[cfg(test)]
mod tests {    use super::*;
    use nmrstuff_core::{AcquisitionMeta, Axis};

    fn multiplet_spectrum(center: f64, j_hz: f64, ratio: &[f64]) -> (Spectrum1D, Vec<Peak>) {
        let sfo = 400.0;
        let n = 8192;
        let sw = 4000.0;
        let axis = Axis::new(10.0, 0.0, n);
        let nlines = ratio.len();
        let mut y = vec![0.0f32; n];
        let mut peaks = Vec::new();
        for (k, r) in ratio.iter().enumerate() {
            let off = (k as f64 - (nlines as f64 - 1.0) / 2.0) * j_hz / sfo;
            let ppm = center + off;
            for i in 0..n {
                let x = axis.ppm_at(i);
                y[i] += (*r as f32) * (-((x - ppm) as f32 / 0.004).powi(2)).exp();
            }
            let mut p = Peak::new(ppm, *r as f32);
            p.width_hz = 1.0;
            peaks.push(p);
        }
        peaks.sort_by(|a, b| b.ppm.partial_cmp(&a.ppm).unwrap());
        (
            Spectrum1D::new(
                axis,
                y,
                AcquisitionMeta {
                    spectrometer_mhz: sfo,
                    spectral_width_hz: sw,
                    ..Default::default()
                },
            ),
            peaks,
        )
    }

    #[test]
    fn quartet_is_recognized() {
        let (spec, peaks) = multiplet_spectrum(4.12, 7.1, &[1.0, 3.0, 3.0, 1.0]);
        let m = analyze_region(&spec, &peaks, 4.2, 4.0, &MultipletParams::default());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].label, MultipletLabel::Q);
        assert_eq!(m[0].j_values_hz.len(), 3);
        assert!((m[0].j_values_hz[0] - 7.1).abs() < 0.2);
        assert!(m[0].fit_residual_hz < 0.2);
    }

    #[test]
    fn triplet_label_and_singlet() {
        let (spec, peaks) = multiplet_spectrum(1.26, 7.1, &[1.0, 2.0, 1.0]);
        let m = analyze_region(&spec, &peaks, 1.4, 1.1, &MultipletParams::default());
        assert_eq!(m[0].label, MultipletLabel::T);

        let (spec2, peaks2) = multiplet_spectrum(2.04, 7.1, &[1.0]);
        let m2 = analyze_region(&spec2, &peaks2, 2.1, 2.0, &MultipletParams::default());
        assert_eq!(m2[0].label, MultipletLabel::S);
    }

    #[test]
    fn uneven_spacing_becomes_m() {
        let (spec, mut peaks) = multiplet_spectrum(3.0, 7.1, &[1.0, 3.0, 3.0, 1.0]);
        peaks[2].ppm += 0.05; // 打乱间距
        let m = analyze_region(&spec, &peaks, 3.1, 2.9, &MultipletParams::default());
        assert_eq!(m[0].label, MultipletLabel::M);
    }

    #[test]
    fn solvent_peak_excluded() {
        let (spec, mut peaks) = multiplet_spectrum(7.26, 7.1, &[1.0]);
        peaks[0].kind = PeakKind::Solvent;
        let m = analyze_region(&spec, &peaks, 7.4, 7.1, &MultipletParams::default());
        assert!(m.is_empty());
    }
}
