use serde::{Deserialize, Serialize};

use crate::nucleus::Nucleus;

/// 原始数据来源厂商/格式，用于错误诊断与“可复现”记录。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Vendor {
    Bruker,
    Varian,
    Jeol,
    Jacmp,
    Csv,
    Synthetic,
    Unknown,
}

impl Vendor {
    pub fn label(&self) -> &'static str {
        match self {
            Vendor::Bruker => "Bruker TopSpin",
            Vendor::Varian => "Varian / Agilent",
            Vendor::Jeol => "JEOL Delta",
            Vendor::Jacmp => "JCAMP-DX",
            Vendor::Csv => "CSV / 文本",
            Vendor::Synthetic => "内置合成数据",
            Vendor::Unknown => "未知",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceInfo {
    pub vendor: Vendor,
    pub path: Option<String>,
    /// 例如 acqus 里的 `##$PULPROG`、JCAMP 的 `##TITLE`
    pub detail: Option<String>,
}

impl Default for Vendor {
    fn default() -> Self {
        Vendor::Unknown
    }
}

/// 采集元数据。字段命名对齐公开格式（Bruker `acqus` / Varian `procpar` / JCAMP 头部）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcquisitionMeta {
    pub nucleus: Nucleus,
    /// 观测频率（MHz），Bruker `SFO1`
    pub spectrometer_mhz: f64,
    /// 谱宽（Hz），Bruker `SW_h`
    pub spectral_width_hz: f64,
    /// 载波/中心位置（ppm）
    pub carrier_ppm: f64,
    /// 溶剂名（原始参数文件里的写法）
    pub solvent: Option<String>,
    pub pulse_program: Option<String>,
    /// 时域复数点数
    pub scanned_points: usize,
    /// 数字滤波群延迟（采样点），Bruker `GRPDLY`
    pub group_delay: Option<f64>,
    pub temperature_k: Option<f64>,
    pub operator: Option<String>,
    pub comment: Option<String>,
}

impl Default for AcquisitionMeta {
    fn default() -> Self {
        Self {
            nucleus: Nucleus::H1,
            spectrometer_mhz: 400.0,
            spectral_width_hz: 8000.0,
            carrier_ppm: 6.0,
            solvent: None,
            pulse_program: None,
            scanned_points: 0,
            group_delay: None,
            temperature_k: None,
            operator: None,
            comment: None,
        }
    }
}

impl AcquisitionMeta {
    /// 每个采样点的驻留时间（秒）。
    pub fn dwell_time(&self) -> f64 {
        if self.spectral_width_hz.abs() < 1e-12 {
            0.0
        } else {
            1.0 / self.spectral_width_hz
        }
    }

    /// 采样总时长（秒）。
    pub fn acquisition_time(&self) -> f64 {
        self.scanned_points as f64 * self.dwell_time()
    }

    /// ppm per point（用于把像素宽度换算成 Hz）。
    pub fn ppm_per_point(&self, points: usize) -> f64 {
        if points == 0 || self.spectrometer_mhz.abs() < 1e-12 {
            0.0
        } else {
            (self.spectral_width_hz / self.spectrometer_mhz) / points as f64
        }
    }
}
