use num_complex::Complex32;
use serde::{Deserialize, Serialize};

use crate::meta::{AcquisitionMeta, SourceInfo};

/// 时域数据（FID）。v0.1 仅支持一维；多维留作后续以 `Vec<Fid>` 组织。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fid {
    pub data: Vec<Complex32>,
    pub meta: AcquisitionMeta,
    pub source: SourceInfo,
}

impl Fid {
    pub fn new(data: Vec<Complex32>, meta: AcquisitionMeta) -> Self {
        let mut meta = meta;
        meta.scanned_points = data.len();
        Self {
            data,
            meta,
            source: SourceInfo::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn dwell_time(&self) -> f64 {
        self.meta.dwell_time()
    }

    /// FID 的最大绝对值，用于展示与归一化。
    pub fn max_abs(&self) -> f32 {
        self.data
            .iter()
            .fold(0.0f32, |acc, c| acc.max(c.norm()))
    }
}
