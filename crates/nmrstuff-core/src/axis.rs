use serde::{Deserialize, Serialize};

use crate::meta::AcquisitionMeta;

/// 单调递减的 ppm 轴：`index 0` 对应高 ppm（谱图左端），`index n-1` 对应低 ppm。
///
/// 这一约定与 NMR 社区惯例一致（左高右低），并且使渲染层可以直接按 index 递增绘制。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Axis {
    pub start_ppm: f64,
    pub end_ppm: f64,
    pub points: usize,
}

impl Axis {
    pub fn new(start_ppm: f64, end_ppm: f64, points: usize) -> Self {
        Self {
            start_ppm,
            end_ppm,
            points,
        }
    }

    /// 从元数据推导轴：`start = (o1 + sw/2)/sfo1`，`end = (o1 - sw/2)/sfo1`。
    pub fn from_meta(meta: &AcquisitionMeta, points: usize) -> Self {
        let half_ppm = if meta.spectrometer_mhz.abs() > 1e-12 {
            meta.spectral_width_hz / 2.0 / meta.spectrometer_mhz
        } else {
            0.0
        };
        Self {
            start_ppm: meta.carrier_ppm + half_ppm,
            end_ppm: meta.carrier_ppm - half_ppm,
            points,
        }
    }

    pub fn span_ppm(&self) -> f64 {
        self.start_ppm - self.end_ppm
    }

    pub fn ppm_per_point(&self) -> f64 {
        if self.points <= 1 {
            0.0
        } else {
            self.span_ppm() / (self.points - 1) as f64
        }
    }

    pub fn ppm_at(&self, index: usize) -> f64 {
        self.start_ppm - index as f64 * self.ppm_per_point()
    }

    /// 连续索引（可带小数，用于插值取峰高）。
    pub fn index_at_ppm(&self, ppm: f64) -> f64 {
        let pp = self.ppm_per_point();
        if pp.abs() < 1e-15 {
            return 0.0;
        }
        (self.start_ppm - ppm) / pp
    }

    pub fn contains(&self, ppm: f64) -> bool {
        ppm <= self.start_ppm && ppm >= self.end_ppm
    }

    /// 归一化位置：0.0 = 高 ppm 端，1.0 = 低 ppm 端。
    pub fn fraction_of_ppm(&self, ppm: f64) -> f64 {
        if self.span_ppm().abs() < 1e-15 {
            0.5
        } else {
            (self.start_ppm - ppm) / self.span_ppm()
        }
    }

    pub fn ppm_of_fraction(&self, f: f64) -> f64 {
        self.start_ppm - f.clamp(0.0, 1.0) * self.span_ppm()
    }

    /// 视口内索引区间（含端点，已裁剪）。
    pub fn index_range(&self, from_ppm: f64, to_ppm: f64) -> (usize, usize) {
        let (hi, lo) = if from_ppm >= to_ppm {
            (from_ppm, to_ppm)
        } else {
            (to_ppm, from_ppm)
        };
        let a = self.index_at_ppm(hi).floor().max(0.0) as usize;
        let b = self.index_at_ppm(lo).ceil().max(0.0) as usize;
        (a.min(self.points.saturating_sub(1)), b.min(self.points.saturating_sub(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_is_descending() {
        let a = Axis::new(10.0, 0.0, 101);
        assert!((a.ppm_at(0) - 10.0).abs() < 1e-9);
        assert!((a.ppm_at(100) - 0.0).abs() < 1e-9);
        assert!((a.ppm_per_point() - 0.1).abs() < 1e-9);
        assert!((a.index_at_ppm(7.5) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn range_clamped() {
        let a = Axis::new(10.0, 0.0, 11);
        assert_eq!(a.index_range(12.0, -2.0), (0, 10));
    }
}
