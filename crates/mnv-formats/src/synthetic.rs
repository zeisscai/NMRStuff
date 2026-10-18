//! 内置合成数据：乙酸乙酯的 ¹H 谱（CDCl₃），带有真实感的缺陷。
//!
//! 用途：
//! 1. 首次启动无需任何数据文件即可跑通完整流水线；
//! 2. 单元测试的已知答案（峰位、J 值、积分比例都写死在代码里）；
//! 3. 演示自动相位 —— 合成时故意引入 42° 零阶相位与 6.5 µs 时域延迟（线性相位）。

use mnv_core::{AcquisitionMeta, Fid, Nucleus, SourceInfo, Vendor};
use num_complex::Complex32;

/// 一条“单峰”的生成参数。
struct Line {
    ppm: f64,
    /// 该信号的总氢数（用于分配强度）
    protons: f64,
    /// 一级耦合分裂出的子峰数（1 = 单峰）
    multiplicity: usize,
    fwhm_hz: f64,
}

const J_HZ: f64 = 7.1;

const SIGNALS: &[Line] = &[
    // 乙酸乙酯：CH₂ 四重峰、CH₃ 单峰、CH₃ 三重峰
    Line { ppm: 4.12, protons: 2.0, multiplicity: 4, fwhm_hz: 1.05 },
    Line { ppm: 2.04, protons: 3.0, multiplicity: 1, fwhm_hz: 0.95 },
    Line { ppm: 1.26, protons: 3.0, multiplicity: 3, fwhm_hz: 1.05 },
    // 残余溶剂、水、内标
    Line { ppm: 7.26, protons: 0.10, multiplicity: 1, fwhm_hz: 0.90 },
    Line { ppm: 1.56, protons: 0.12, multiplicity: 1, fwhm_hz: 2.50 },
    Line { ppm: 0.00, protons: 0.15, multiplicity: 1, fwhm_hz: 0.80 },
];

pub const DEMO_SFO_MHZ: f64 = 400.13;
pub const DEMO_SW_HZ: f64 = 5200.0;
pub const DEMO_CARRIER_PPM: f64 = 5.5;
pub const DEMO_POINTS: usize = 16384;
/// 合成时引入的零阶相位误差（度）
pub const DEMO_PHASE0_DEG: f64 = 42.0;
/// 合成时引入的时域延迟（秒）→ 一阶（线性）相位误差
pub const DEMO_DELAY_S: f64 = 6.5e-6;

/// 生成演示 FID。
pub fn demo_ethyl_acetate() -> Fid {
    let sw = DEMO_SW_HZ;
    let sfo = DEMO_SFO_MHZ;
    let n = DEMO_POINTS;
    let dwell = 1.0 / sw;
    let (s_ph, c_ph) = DEMO_PHASE0_DEG.to_radians().sin_cos();
    let (s_ph, c_ph) = (s_ph as f32, c_ph as f32);

    let mut seed = 0x1234_5678u32;
    let mut noise = move || {
        // xorshift32：保证每次运行得到相同噪声，便于回归测试
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) - 0.5
    };

    let mut data = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 * dwell;
        let mut acc = Complex32::new(0.0, 0.0);

        for sig in SIGNALS {
            let pattern = binomial_pattern(sig.multiplicity);
            let weight_sum: f64 = pattern.iter().sum();
            let center_hz = (sig.ppm - DEMO_CARRIER_PPM) * sfo;
            let decay = (-std::f64::consts::PI * sig.fwhm_hz * t).exp();
            for (k, w) in pattern.iter().enumerate() {
                let offset = (k as f64 - (pattern.len() as f64 - 1.0) / 2.0) * J_HZ;
                let nu = center_hz + offset;
                let amp = sig.protons * w / weight_sum;
                let ph = 2.0 * std::f64::consts::PI * nu * (t - DEMO_DELAY_S);
                acc += Complex32::new(
                    (amp * decay * ph.cos()) as f32,
                    (amp * decay * ph.sin()) as f32,
                );
            }
        }

        // 零阶相位误差 + 复噪声
        let rotated = Complex32::new(
            acc.re * c_ph - acc.im * s_ph,
            acc.re * s_ph + acc.im * c_ph,
        );
        data.push(rotated + Complex32::new(noise() * 0.0025, noise() * 0.0025));
    }

    let meta = AcquisitionMeta {
        nucleus: Nucleus::H1,
        spectrometer_mhz: sfo,
        spectral_width_hz: sw,
        carrier_ppm: DEMO_CARRIER_PPM,
        solvent: Some("CDCl3".into()),
        pulse_program: Some("zg30".into()),
        scanned_points: n,
        group_delay: Some(0.0),
        temperature_k: Some(298.0),
        operator: Some("demo".into()),
        comment: Some("内置合成数据：乙酸乙酯 (CDCl₃)".into()),
    };
    let mut fid = Fid::new(data, meta);
    fid.source = SourceInfo {
        vendor: Vendor::Synthetic,
        path: None,
        detail: Some("synthetic::demo_ethyl_acetate".into()),
    };
    fid
}

/// 二项式强度比，例如 4 → 1:3:3:1。
fn binomial_pattern(n: usize) -> Vec<f64> {
    let mut row = vec![1.0f64];
    for _ in 1..n.max(1) {
        let mut next = vec![1.0];
        for w in row.windows(2) {
            next.push(w[0] + w[1]);
        }
        next.push(1.0);
        row = next;
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_has_expected_amplitude() {
        let fid = demo_ethyl_acetate();
        assert!(fid.max_abs() > 0.5);
        assert_eq!(fid.len(), DEMO_POINTS);
    }

    #[test]
    fn binomial_rows_are_correct() {
        assert_eq!(binomial_pattern(1), vec![1.0]);
        assert_eq!(binomial_pattern(2), vec![1.0, 1.0]);
        assert_eq!(binomial_pattern(3), vec![1.0, 2.0, 1.0]);
        assert_eq!(binomial_pattern(4), vec![1.0, 3.0, 3.0, 1.0]);
    }
}
