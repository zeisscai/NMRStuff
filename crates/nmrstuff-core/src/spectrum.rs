use serde::{Deserialize, Serialize};

use crate::axis::Axis;
use crate::meta::{AcquisitionMeta, SourceInfo};
use crate::proc_op::ProcOp;

/// 处理完成的频域谱。
///
/// `history` 记录从 FID 到本谱图经过的全部步骤；它既是“处理参数表”的数据源，
/// 也是回放与一致性测试的依据。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spectrum1D {
    pub x: Axis,
    pub y: Vec<f32>,
    pub meta: AcquisitionMeta,
    pub history: Vec<ProcOp>,
    pub source: SourceInfo,
}

impl Spectrum1D {
    pub fn new(x: Axis, y: Vec<f32>, meta: AcquisitionMeta) -> Self {
        Self {
            x,
            y,
            meta,
            history: Vec::new(),
            source: SourceInfo::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.y.len()
    }

    pub fn is_empty(&self) -> bool {
        self.y.is_empty()
    }

    /// 线性插值取值；越界返回端点值。
    pub fn value_at_ppm(&self, ppm: f64) -> f32 {
        if self.y.is_empty() {
            return 0.0;
        }
        let idx = self.x.index_at_ppm(ppm);
        if idx <= 0.0 {
            return self.y[0];
        }
        let last = self.y.len() - 1;
        if idx >= last as f64 {
            return self.y[last];
        }
        let i = idx.floor() as usize;
        let frac = (idx - i as f64) as f32;
        self.y[i] * (1.0 - frac) + self.y[i + 1] * frac
    }

    pub fn max_value(&self) -> f32 {
        self.y.iter().copied().fold(f32::MIN, f32::max)
    }

    pub fn min_value(&self) -> f32 {
        self.y.iter().copied().fold(f32::MAX, f32::min)
    }

    pub fn max_abs(&self) -> f32 {
        self.y.iter().fold(0.0f32, |a, v| a.max(v.abs()))
    }

    /// 稳健噪声估计（MAD）。用于自动标峰的默认阈值。
    pub fn noise_sigma(&self) -> f32 {
        if self.y.len() < 16 {
            return 0.0;
        }
        let mut d: Vec<f32> = self
            .y
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .collect();
        d.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let med = d[d.len() / 2];
        // 相邻差分绝对值的中位数 ≈ 0.954 * sigma * sqrt(2) / sqrt(pi) 的近似；
        // 采用 1.4826 * MAD 的简化常数形式即可满足阈值判断。
        med * 1.4826 / std::f32::consts::SQRT_2
    }

    /// 视口内的极值包络（每像素列 min/max），供渲染层直接使用。
    pub fn envelope(&self, from_ppm: f64, to_ppm: f64, buckets: usize) -> Vec<(f32, f32)> {
        let (a, b) = self.x.index_range(from_ppm, to_ppm);
        if a > b || self.y.is_empty() {
            return Vec::new();
        }
        let n = b - a + 1;
        let buckets = buckets.max(1);
        let mut out = Vec::with_capacity(buckets);
        for k in 0..buckets {
            let s = a + (k * n) / buckets;
            let e = (a + ((k + 1) * n) / buckets).max(s + 1).min(b + 1);
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for v in &self.y[s..e] {
                if *v < lo {
                    lo = *v;
                }
                if *v > hi {
                    hi = *v;
                }
            }
            out.push((lo, hi));
        }
        out
    }

    /// 复制一份并整体平移（ppm）。
    pub fn shifted(&self, delta_ppm: f64) -> Self {
        let mut s = self.clone();
        s.x.start_ppm += delta_ppm;
        s.x.end_ppm += delta_ppm;
        s
    }

    /// 按给定 ppm 窗口裁出局部谱（不重采样，仅切片），用于 inset 或局部统计。
    pub fn slice_ppm(&self, from_ppm: f64, to_ppm: f64) -> Option<Self> {
        let (a, b) = self.x.index_range(from_ppm, to_ppm);
        if b <= a {
            return None;
        }
        let y = self.y[a..=b].to_vec();
        let mut s = self.clone();
        s.x = Axis::new(self.x.ppm_at(a), self.x.ppm_at(b), y.len());
        s.y = y;
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toy() -> Spectrum1D {
        let y: Vec<f32> = (0..101).map(|i| i as f32).collect();
        Spectrum1D::new(Axis::new(10.0, 0.0, 101), y, AcquisitionMeta::default())
    }

    #[test]
    fn interpolation_is_linear() {
        let s = toy();
        assert!((s.value_at_ppm(2.5) - 75.0).abs() < 1e-3);
        assert!((s.value_at_ppm(100.0) - 0.0).abs() < 1e-3);
    }

    #[test]
    fn envelope_covers_range() {
        let s = toy();
        let env = s.envelope(10.0, 5.0, 50);
        assert_eq!(env.len(), 50);
        assert!((env[0].0 - 0.0).abs() < 1e-3);
        // 10–5 ppm 对应索引 0..50，因此右端极值约为 50
        assert!(env[49].1 >= 49.0);
        assert!(env[49].1 <= 51.0);
    }
}
