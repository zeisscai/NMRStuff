//! 处理管线编排：把 `Vec<ProcOp>` 作用到 FID 上，产出 `Spectrum1D`。
//!
//! 关键性质：
//! 1. **顺序敏感**：步骤按列表顺序执行，非法顺序（如 FFT 之后再开窗）会被跳过并给出警告，
//!    而不是静默出错 —— 这对应“错误可诊断”的要求。
//! 2. **自动相位会被回填**：`Phase { automatic: true }` 解出的具体角度写回 history，
//!    保证报告里的参数表是“实际用了什么”，而不是“界面显示什么”。
//! 3. **轴只在最后统一平移**：定标步骤累加 `ppm_shift`，避免中间步骤反复重采样。

use std::time::Instant;

use mnv_core::{
    ApodKind, Axis, BaselineKind, Fid, MnvError, NormMode, ProcOp, Result, Spectrum1D,
};
use num_complex::Complex32;

use crate::{apodize, baseline, fft, phase, smooth};

#[derive(Clone, Debug, Default)]
pub struct ProcReport {
    /// 被跳过的步骤及原因（非法顺序、参数越界等）
    pub warnings: Vec<String>,
    /// 实际生效的步骤（含自动相位的解算结果）
    pub applied: Vec<ProcOp>,
    pub elapsed_ms: f64,
}

#[derive(Clone, Debug)]
pub struct ProcOutput {
    pub spectrum: Spectrum1D,
    pub report: ProcReport,
}

/// 执行完整处理链。
pub fn process(fid: &Fid, steps: &[ProcOp]) -> Result<ProcOutput> {
    if fid.data.is_empty() {
        return Err(MnvError::incomplete("FID 为空，无法处理"));
    }
    let t0 = Instant::now();
    let mut report = ProcReport::default();

    let dwell = fid.meta.dwell_time();
    let mut time: Vec<Complex32> = fid.data.clone();
    let mut freq: Option<Vec<Complex32>> = None;
    let mut ppm_shift = 0.0f64;

    for step in steps {
        match step {
            ProcOp::Apodization {
                kind,
                lb_hz,
                gb_hz,
                sine_shift,
            } => {
                if freq.is_some() {
                    report
                        .warnings
                        .push("窗函数必须位于 FFT 之前，已跳过".into());
                    continue;
                }
                if *kind == ApodKind::None {
                    continue;
                }
                apodize::apply(&mut time, *kind, *lb_hz, *gb_hz, *sine_shift, dwell);
                report.applied.push(step.clone());
            }
            ProcOp::ZeroFill { factor } => {
                if freq.is_some() {
                    report
                        .warnings
                        .push("零填充必须位于 FFT 之前，已跳过".into());
                    continue;
                }
                fft::zero_fill(&mut time, (*factor).max(1));
                report.applied.push(step.clone());
            }
            ProcOp::Fft => {
                if freq.is_some() {
                    report.warnings.push("重复的 FFT，已跳过".into());
                    continue;
                }
                fft::forward(&mut time);
                freq = Some(fft::to_descending_ppm(&time));
                report.applied.push(step.clone());
            }
            ProcOp::GroupDelay { points } => {
                let Some(spec) = freq.as_mut() else {
                    report
                        .warnings
                        .push("群延迟校正需在 FFT 之后，已跳过".into());
                    continue;
                };
                // 时域延迟 τ = points·dw 等价于频谱乘 exp(-i2πντ)；
                // 校正需乘 exp(+i2πντ) → φ = -360·ν·τ。在 ppm 降序排列下，
                // ν 从 +sw/2 递减到 -sw/2，故 φ1（低 ppm − 高 ppm）= +360·points。
                phase::apply(spec, 0.0, 360.0 * points);
                report.applied.push(step.clone());
            }
            ProcOp::Phase {
                ph0_deg,
                ph1_deg,
                automatic,
            } => {
                let Some(spec) = freq.as_mut() else {
                    report.warnings.push("相位校正需在 FFT 之后，已跳过".into());
                    continue;
                };
                let (p0, p1) = if *automatic {
                    phase::auto_phase(spec)
                } else {
                    (*ph0_deg, *ph1_deg)
                };
                phase::apply(spec, p0, p1);
                report.applied.push(ProcOp::Phase {
                    ph0_deg: p0,
                    ph1_deg: p1,
                    automatic: *automatic,
                });
            }
            ProcOp::Baseline {
                kind,
                lambda,
                order,
                iterations,
            } => {
                if *kind == BaselineKind::None {
                    continue;
                }
                let Some(spec) = freq.as_mut() else {
                    report.warnings.push("基线校正需在 FFT 之后，已跳过".into());
                    continue;
                };
                let mut y: Vec<f32> = spec.iter().map(|c| c.re).collect();
                let b = baseline::compute(&y, *kind, *lambda, *order, *iterations);
                for (v, base) in y.iter_mut().zip(b) {
                    *v -= base;
                }
                write_back(spec, &y);
                report.applied.push(step.clone());
            }
            ProcOp::Smooth { kind, window } => {
                if *kind == mnv_core::SmoothKind::None {
                    continue;
                }
                let Some(spec) = freq.as_mut() else {
                    report.warnings.push("平滑需在 FFT 之后，已跳过".into());
                    continue;
                };
                let y: Vec<f32> = spec.iter().map(|c| c.re).collect();
                let s = smooth::compute(&y, *kind, *window);
                write_back(spec, &s);
                report.applied.push(step.clone());
            }
            ProcOp::Calibrate {
                reference_ppm,
                target_ppm,
                delta_ppm,
            } => {
                let shift = match reference_ppm {
                    Some(r) => target_ppm - r,
                    None => *delta_ppm,
                };
                ppm_shift += shift;
                report.applied.push(step.clone());
            }
            ProcOp::Normalize { mode } => {
                let Some(spec) = freq.as_mut() else {
                    report.warnings.push("归一化需在 FFT 之后，已跳过".into());
                    continue;
                };
                normalize(spec, *mode);
                report.applied.push(step.clone());
            }
        }
    }

    let spec = freq.ok_or_else(|| MnvError::processing("处理链中缺少 FFT 步骤，无法生成谱图"))?;
    let n = spec.len();
    let y: Vec<f32> = spec.iter().map(|c| c.re).collect();

    let half_ppm = if fid.meta.spectrometer_mhz.abs() > 1e-12 {
        fid.meta.spectral_width_hz / 2.0 / fid.meta.spectrometer_mhz
    } else {
        0.0
    };
    let axis = Axis::new(
        fid.meta.carrier_ppm + half_ppm + ppm_shift,
        fid.meta.carrier_ppm - half_ppm + ppm_shift,
        n,
    );

    let mut meta = fid.meta.clone();
    meta.scanned_points = fid.meta.scanned_points.max(1);

    let mut spectrum = Spectrum1D::new(axis, y, meta);
    spectrum.source = fid.source.clone();
    spectrum.history = report.applied.clone();
    report.elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    Ok(ProcOutput { spectrum, report })
}

fn write_back(spec: &mut [Complex32], y: &[f32]) {
    for (c, v) in spec.iter_mut().zip(y) {
        c.re = *v;
        c.im = 0.0;
    }
}

fn normalize(spec: &mut [Complex32], mode: NormMode) {
    match mode {
        NormMode::None => {}
        NormMode::Max => {
            let max = spec.iter().fold(0.0f32, |a, c| a.max(c.re.abs()));
            if max > 0.0 {
                for c in spec.iter_mut() {
                    c.re /= max;
                }
            }
        }
        NormMode::Sum => {
            let sum: f32 = spec.iter().map(|c| c.re.abs()).sum();
            if sum > 0.0 {
                for c in spec.iter_mut() {
                    c.re /= sum;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnv_core::AcquisitionMeta;

    fn toy_fid() -> Fid {
        let n = 4096;
        let sw = 4000.0f64;
        let sfo = 400.0f64;
        let meta = AcquisitionMeta {
            nucleus: mnv_core::Nucleus::H1,
            spectrometer_mhz: sfo,
            spectral_width_hz: sw,
            carrier_ppm: 5.0,
            ..Default::default()
        };
        let dwell = 1.0 / sw;
        // 单一 Lorentz 峰在 2.5 ppm 处
        let nu = (2.5 - 5.0) * sfo;
        let data = (0..n)
            .map(|i| {
                let t = i as f64 * dwell;
                let a = (-t / 0.3).exp();
                Complex32::new((a * (2.0 * std::f64::consts::PI * nu * t).cos()) as f32, (a * (2.0 * std::f64::consts::PI * nu * t).sin()) as f32)
            })
            .collect();
        Fid::new(data, meta)
    }

    #[test]
    fn pipeline_produces_peak_at_expected_ppm() {
        let fid = toy_fid();
        let steps = vec![
            ProcOp::Fft,
            ProcOp::Phase {
                ph0_deg: 0.0,
                ph1_deg: 0.0,
                automatic: true,
            },
        ];
        let out = process(&fid, &steps).expect("处理应成功");
        let (idx, _) = out
            .spectrum
            .y
            .iter()
            .enumerate()
            .fold((0usize, f32::MIN), |acc, (i, v)| {
                if *v > acc.1 {
                    (i, *v)
                } else {
                    acc
                }
            });
        let ppm = out.spectrum.x.ppm_at(idx);
        assert!((ppm - 2.5).abs() < 0.05, "峰位 {ppm} ppm");
        assert_eq!(out.report.applied.len(), 2);
    }

    #[test]
    fn wrong_order_is_reported_not_silently_applied() {
        let fid = toy_fid();
        let steps = vec![
            ProcOp::Fft,
            ProcOp::Apodization {
                kind: ApodKind::Exponential,
                lb_hz: 1.0,
                gb_hz: 0.0,
                sine_shift: 0.0,
            },
        ];
        let out = process(&fid, &steps).unwrap();
        assert_eq!(out.report.warnings.len(), 1);
        assert_eq!(out.report.applied.len(), 1);
    }

    #[test]
    fn missing_fft_errors() {
        let fid = toy_fid();
        let steps = vec![ProcOp::ZeroFill { factor: 2 }];
        assert!(process(&fid, &steps).is_err());
    }

    #[test]
    fn calibration_shifts_axis() {
        let fid = toy_fid();
        let steps = vec![
            ProcOp::Fft,
            ProcOp::Calibrate {
                reference_ppm: Some(2.5),
                target_ppm: 0.0,
                delta_ppm: 0.0,
            },
        ];
        let out = process(&fid, &steps).unwrap();
        let (idx, _) = out
            .spectrum
            .y
            .iter()
            .enumerate()
            .fold((0usize, f32::MIN), |acc, (i, v)| {
                if *v > acc.1 {
                    (i, *v)
                } else {
                    acc
                }
            });
        let ppm = out.spectrum.x.ppm_at(idx);
        assert!(ppm.abs() < 0.05, "定标后峰位 {ppm} ppm");
    }
}
