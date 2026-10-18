//! 窗函数（时域加权）。
//!
//! 约定：`t = i * dwell`（秒），`x = i / (n-1)`（归一化位置）。
//! 不同厂商对 LB 的符号约定不一致（正数=展宽 或 正数=锐化），本实现统一为：
//! * 指数窗：`w = exp(-π·LB·t)`，LB > 0 → 线宽增加、信噪比提高；
//! * 高斯窗：`w = exp(-π·LB·t)·exp(-(π·GB·t)²)`，GB > 0 → 抑制 FID 尾部；
//! * 洛伦兹-高斯：`w = exp(+π·LB·t)·exp(-(π·GB·t)²)`，用于分辨率增强（LB 常取负值）。

use nmrstuff_core::ApodKind;
use num_complex::Complex32;

pub fn apply(
    data: &mut [Complex32],
    kind: ApodKind,
    lb_hz: f64,
    gb_hz: f64,
    sine_shift: f64,
    dwell: f64,
) {
    let n = data.len();
    if n == 0 {
        return;
    }
    let n1 = (n - 1).max(1) as f64;
    for (i, c) in data.iter_mut().enumerate() {
        let x = i as f64 / n1;
        let t = i as f64 * dwell;
        let w = match kind {
            ApodKind::None => 1.0,
            ApodKind::Exponential => (-std::f64::consts::PI * lb_hz * t).exp(),
            ApodKind::Gauss => {
                (-std::f64::consts::PI * lb_hz * t).exp()
                    * (-(std::f64::consts::PI * gb_hz * t).powi(2)).exp()
            }
            ApodKind::LorentzGauss => {
                (std::f64::consts::PI * lb_hz * t).exp()
                    * (-(std::f64::consts::PI * gb_hz * t).powi(2)).exp()
            }
            ApodKind::SineBell => {
                let s = sine_shift.clamp(0.0, 0.95);
                (std::f64::consts::PI * (s + (1.0 - s) * x)).sin()
            }
            ApodKind::SineSquare => {
                let s = sine_shift.clamp(0.0, 0.95);
                (std::f64::consts::PI * (s + (1.0 - s) * x)).sin().powi(2)
            }
            ApodKind::Trapezoid => {
                let ramp = 0.02;
                (x / ramp).min((1.0 - x) / ramp).min(1.0).max(0.0)
            }
            ApodKind::Hamming => 0.54 - 0.46 * (2.0 * std::f64::consts::PI * x).cos(),
            ApodKind::Hanning => 0.5 - 0.5 * (2.0 * std::f64::consts::PI * x).cos(),
        };
        let w = w as f32;
        c.re *= w;
        c.im *= w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_decays_monotonically() {
        let mut d = vec![Complex32::new(1.0, 0.0); 100];
        apply(&mut d, ApodKind::Exponential, 5.0, 0.0, 0.0, 1e-4);
        assert!(d[0].re > d[50].re);
        assert!(d[50].re > d[99].re);
        assert!((d[99].re - (std::f32::consts::PI * -5.0 * 99e-4).exp()).abs() < 1e-4);
    }

    #[test]
    fn sine_bell_tapers_edges() {
        let mut d = vec![Complex32::new(1.0, 0.0); 101];
        apply(&mut d, ApodKind::SineBell, 0.0, 0.0, 0.0, 1e-4);
        assert!(d[0].re.abs() < 1e-6);
        assert!((d[50].re - 1.0).abs() < 1e-4);
    }
}
