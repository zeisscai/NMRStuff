//! 相位校正：手动 0/1 阶与自动相位。
//!
//! 相位模型（本实现的约定，务必与 UI 显示一致）：
//!
//! ```text
//! y(ν) = Re{ X(ν) · exp(-i·φ(ν)) }
//! φ(ν) = φ0 + φ1 · i/(N-1)       // i = 0 对应最高 ppm
//! ```
//!
//! 即 `φ1` 表示**谱宽两端（高 ppm → 低 ppm）的总相位差**，而不是“每 ppm 的角度”。
//! 这样做的原因：一维谱的像素索引与 ppm 一一对应，用索引做线性插值最稳定。
//!
//! 自动相位采用公开的“熵最小化 + 负值惩罚”目标函数：
//! 一阶相位正确时谱图最“尖锐”（导数分布最不均匀 → 熵最小），且不出现负峰。
//! 先做二维粗网格搜索，再做变步长坐标下降细化。

use num_complex::Complex32;

/// 施加相位校正。
pub fn apply(spec: &mut [Complex32], ph0_deg: f64, ph1_deg: f64) {
    let n = spec.len();
    if n == 0 {
        return;
    }
    let mut ramp = Ramp::new(ph0_deg, ph1_deg, n);
    for c in spec.iter_mut() {
        let (s, co) = ramp.next();
        let (re, im) = (c.re as f64, c.im as f64);
        *c = Complex32::new((re * co + im * s) as f32, (im * co - re * s) as f32);
    }
}

/// 用给定相位生成实部预览（不修改原数据），供 UI 拖动时实时重算。
pub fn phased_real(spec: &[Complex32], ph0_deg: f64, ph1_deg: f64) -> Vec<f32> {
    let n = spec.len();
    let mut ramp = Ramp::new(ph0_deg, ph1_deg, n);
    let mut out = Vec::with_capacity(n);
    for c in spec.iter() {
        let (s, co) = ramp.next();
        out.push((c.re as f64 * co + c.im as f64 * s) as f32);
    }
    out
}

/// 自动相位：返回 `(φ0, φ1)`。
///
/// 三阶段：
/// 1. 抽稀到约 512 点做 10° 网格全局搜索（避开局部极小，同时控制耗时）；
/// 2. 全分辨率下在最优点附近做 ±10°、步长 2° 的局部网格；
/// 3. 变步长坐标下降细化到 0.1°。
pub fn auto_phase(spec: &[Complex32]) -> (f64, f64) {
    let n = spec.len();
    if n < 16 {
        return (0.0, 0.0);
    }

    let stride = (n / 512).max(1);
    let re_c: Vec<f32> = spec.iter().step_by(stride).map(|c| c.re).collect();
    let im_c: Vec<f32> = spec.iter().step_by(stride).map(|c| c.im).collect();

    let mut best = (0.0f64, 0.0f64, f64::INFINITY);
    let mut p0 = -180.0;
    while p0 <= 180.0 {
        let mut p1 = -180.0;
        while p1 <= 180.0 {
            let c = cost(&re_c, &im_c, p0, p1);
            if c < best.2 {
                best = (p0, p1, c);
            }
            p1 += 10.0;
        }
        p0 += 10.0;
    }

    // 局部细化仍用抽稀数据（成本低一个数量级），最后再用全分辨率做坐标下降
    let re: Vec<f32> = spec.iter().map(|c| c.re).collect();
    let im: Vec<f32> = spec.iter().map(|c| c.im).collect();
    let mut refined = (best.0, best.1, best.2);
    for (base0, step0, base1, step1) in [
        (best.0, 2.0, best.1, 2.0),
        (best.0, 0.5, best.1, 0.5),
    ] {
        let mut bb = refined;
        let mut i = -5;
        while i <= 5 {
            let mut j = -5;
            while j <= 5 {
                let c0 = base0 + i as f64 * step0;
                let c1 = base1 + j as f64 * step1;
                let c = cost(&re_c, &im_c, c0, c1);
                if c < bb.2 {
                    bb = (c0, c1, c);
                }
                j += 1;
            }
            i += 1;
        }
        refined = bb;
    }

    let (mut p0, mut p1, mut cur) = refined;
    let mut s0 = 0.5;
    let mut s1 = 0.5;
    while s0 > 0.05 {
        let mut improved = false;
        for (d0, d1) in [
            (s0, 0.0),
            (-s0, 0.0),
            (0.0, s1),
            (0.0, -s1),
            (s0, s1),
            (-s0, -s1),
            (s0, -s1),
            (-s0, s1),
        ] {
            let c = cost(&re, &im, p0 + d0, p1 + d1);
            if c < cur {
                cur = c;
                p0 += d0;
                p1 += d1;
                improved = true;
            }
        }
        if !improved {
            s0 *= 0.5;
            s1 *= 0.5;
        }
    }
    accept_or_zero(spec, p0, p1)
}

/// 安全阀：若解出的相位把谱整体转得几乎为零（幅度不到原始复数幅度的一半），
/// 说明落入了退化解，退回到“不校正”，把决定权交还用户。
fn accept_or_zero(spec: &[Complex32], p0: f64, p1: f64) -> (f64, f64) {
    let raw = spec.iter().fold(0.0f32, |a, c| a.max(c.norm()));
    if raw <= 0.0 {
        return (0.0, 0.0);
    }
    let y = phased_real(spec, p0, p1);
    let got = y.iter().fold(0.0f32, |a, v| a.max(v.abs()));
    if got < raw * 0.5 {
        (0.0, 0.0)
    } else {
        (p0, p1)
    }
}

/// 相位斜坡的增量计算，避免每个点都调用 sin/cos（大谱图上这是主要开销）。
struct Ramp {
    s: f64,
    c: f64,
    ds: f64,
    dc: f64,
    step: f64,
    s0: f64,
    c0: f64,
    k: usize,
}

impl Ramp {
    fn new(ph0_deg: f64, ph1_deg: f64, n: usize) -> Self {
        let step = ph1_deg.to_radians() / (n.max(2) - 1) as f64;
        let (s0, c0) = ph0_deg.to_radians().sin_cos();
        let (ds, dc) = step.sin_cos();
        Self {
            s: s0,
            c: c0,
            ds,
            dc,
            step,
            s0,
            c0,
            k: 0,
        }
    }

    #[inline]
    fn next(&mut self) -> (f64, f64) {
        let out = (self.s, self.c);
        // 每 512 步用精确值重新同步，抑制增量累积误差
        self.k += 1;
        if self.k % 512 == 0 {
            let phi = self.k as f64 * self.step;
            let (s, c) = phi.sin_cos();
            // sin(φ0+φ) = sinφ0·cosφ + cosφ0·sinφ
            self.s = self.s0 * c + self.c0 * s;
            self.c = self.c0 * c - self.s0 * s;
        } else {
            let s = self.s * self.dc + self.c * self.ds;
            let c = self.c * self.dc - self.s * self.ds;
            self.s = s;
            self.c = c;
        }
        out
    }
}

/// 目标函数：负值惩罚 + “吸收成分的平方能量占比”。
///
/// ```text
/// cost = 6·neg_ratio − 100 · Σ(y⁺)² / Σ|X|²
/// ```
///
/// 正确的相位让 `y⁺` 尽量等于复幅度 |X|（吸收成分最大）；错误相位把能量转到
/// 色散/虚部，分子随之下降。早期的“导数熵最小化”方案在合成数据上会奖励
/// “把某些峰转没”（特征越少熵越小），实测会把第二个峰消掉，故弃用。
fn cost(re: &[f32], im: &[f32], ph0: f64, ph1: f64) -> f64 {
    let n = re.len();
    let mut ramp = Ramp::new(ph0, ph1, n);
    let mut pos = 0.0f64;
    let mut neg = 0.0f64;
    let mut pos_sq = 0.0f64;
    let mut mag_sq = 0.0f64;

    for i in 0..n {
        let (re_i, im_i) = (re[i] as f64, im[i] as f64);
        mag_sq += re_i * re_i + im_i * im_i;
        let (s, co) = ramp.next();
        let y = re_i * co + im_i * s;
        if y > 0.0 {
            pos += y;
            pos_sq += y * y;
        } else {
            neg -= y;
        }
    }
    if mag_sq <= 0.0 {
        return f64::INFINITY;
    }
    let neg_ratio = if pos + neg > 0.0 { neg / (pos + neg) } else { 0.0 };
    let absorption = pos_sq / mag_sq;

    6.0 * neg_ratio - 100.0 * absorption
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_with_phase(ph0: f32, ph1: f32) -> Vec<Complex32> {
        // 单峰 + 相位误差：数据被旋转了 +φ，正确校正是乘 e^{-iφ}
        let n = 1024;
        (0..n)
            .map(|i| {
                let x = i as f32 / n as f32;
                let amp = (-((x - 0.5) * 40.0).powi(2)).exp();
                let p = (ph0 + ph1 * x).to_radians();
                Complex32::new(amp * p.cos(), amp * p.sin())
            })
            .collect()
    }

    /// 两个峰 + 随位置线性变化的相位（模拟一阶相位误差）。
    /// 单峰且相位恒定时目标函数存在“整段缩放不变”的退化解，
    /// 因此验收标准是“校正后的谱是否为正吸收峰”，而不是对齐某个特定角度。
    fn two_peaks(ph0: f32, ph1: f32) -> Vec<Complex32> {
        let n = 2048;
        (0..n)
            .map(|i| {
                let x = i as f32 / n as f32;
                let amp = (-((x - 0.32) * 26.0).powi(2)).exp()
                    + 0.65 * (-((x - 0.68) * 34.0).powi(2)).exp();
                let p = (ph0 + ph1 * x).to_radians();
                Complex32::new(amp * p.cos(), amp * p.sin())
            })
            .collect()
    }

    #[test]
    fn manual_phase_recovers_peak() {
        let mut s = synthetic_with_phase(60.0, 0.0);
        apply(&mut s, 60.0, 0.0);
        let y: Vec<f32> = s.iter().map(|c| c.re).collect();
        let max = y.iter().cloned().fold(f32::MIN, f32::max);
        let min = y.iter().cloned().fold(f32::MAX, f32::min);
        assert!(max > 0.9, "峰值应恢复, max={max}");
        assert!(min > -0.05, "不应残留明显负值, min={min}");
    }

    #[test]
    fn auto_phase_produces_absorption_spectrum() {
        let s = two_peaks(40.0, 90.0);
        let (p0, p1) = auto_phase(&s);
        let y = phased_real(&s, p0, p1);
        let max = y.iter().cloned().fold(f32::MIN, f32::max);
        let min = y.iter().cloned().fold(f32::MAX, f32::min);
        assert!(max > 0.9, "自动相位 φ0={p0:.1} φ1={p1:.1}, max={max}");
        assert!(min > -0.06, "自动相位后残留负值 {min}");
    }
}
