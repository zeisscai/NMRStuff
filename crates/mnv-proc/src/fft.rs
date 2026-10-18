//! FFT 与频域重排。

use num_complex::Complex32;
use rustfft::FftPlanner;

/// 原地复数 FFT（不重排）。输入长度不限，`rustfft` 会自动选择算法。
pub fn forward(data: &mut [Complex32]) {
    let n = data.len();
    if n <= 1 {
        return;
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(data);
}

/// 把 FFT 输出（频率升序、DC 在 0）转换为 **ppm 降序** 的实部数组。
///
/// 映射关系 `k = (n/2 - i) mod n`：
/// * `i = 0` → `k = n/2`（+sw/2，Nyquist，对应最高 ppm）
/// * `i = n/2` → `k = 0`（DC）
/// * `i = n-1` → `k = n/2 + 1`（-sw/2 + sw/n）
///
/// 这样 `Spectrum1D::x` 就可以直接按 index 递增绘制（左高右低）。
pub fn to_descending_ppm(data: &[Complex32]) -> Vec<Complex32> {
    let n = data.len();
    let half = n / 2;
    (0..n)
        .map(|i| data[(half + n - i) % n])
        .collect()
}

/// 仅取实部（降序），供快速绘制。
pub fn real_descending(data: &[Complex32]) -> Vec<f32> {
    let n = data.len();
    let half = n / 2;
    (0..n).map(|i| data[(half + n - i) % n].re).collect()
}

/// 补零到不小于 `factor × len` 的 2 的幂。
pub fn zero_fill(data: &mut Vec<Complex32>, factor: usize) {
    let target = next_pow2(data.len().saturating_mul(factor.max(1)));
    if target > data.len() {
        data.resize(target, Complex32::new(0.0, 0.0));
    }
}

pub fn next_pow2(n: usize) -> usize {
    let mut p = 1usize;
    while p < n {
        p <<= 1;
    }
    p.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descending_puts_dc_in_middle() {
        let mut d = vec![Complex32::new(0.0, 0.0); 8];
        d[0] = Complex32::new(10.0, 0.0); // DC
        d[4] = Complex32::new(1.0, 0.0); // Nyquist
        let out = to_descending_ppm(&d);
        assert_eq!(out[0].re, 1.0); // 最高频在左端
        assert_eq!(out[4].re, 10.0); // DC 在中间
    }

    #[test]
    fn zero_fill_rounds_to_pow2() {
        let mut d = vec![Complex32::new(1.0, 0.0); 1000];
        zero_fill(&mut d, 2);
        assert_eq!(d.len(), 2048);
        assert_eq!(d[1500].re, 0.0);
    }

    #[test]
    fn fft_of_cosine_has_two_peaks() {
        let n = 64;
        let mut d: Vec<Complex32> = (0..n)
            .map(|i| {
                let ph = 2.0 * std::f32::consts::PI * 4.0 * i as f32 / n as f32;
                Complex32::new(ph.cos(), 0.0)
            })
            .collect();
        forward(&mut d);
        let mags: Vec<f32> = d.iter().map(|c| c.norm()).collect();
        let mut idx: Vec<usize> = (0..n).collect();
        idx.sort_by(|a, b| mags[*b].partial_cmp(&mags[*a]).unwrap());
        let mut top = vec![idx[0], idx[1]];
        top.sort();
        assert_eq!(top, vec![4, n - 4]);
    }
}
