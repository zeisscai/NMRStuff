//! mnv-formats —— 原始数据读取。
//!
//! 策略：**格式按公开规范自写解析器，处理与分析全部自研**。
//! 已实现：
//! * Bruker TopSpin（`acqus` + `fid`，i32 交错复数，含群延迟、溶剂、脉冲程序）
//! * JCAMP-DX（`X++(Y..Y)`，支持 `DIF` / `DUP`）
//! * CSV / 空白分隔两列文本
//! * 内置合成数据（无数据文件时用于自检与演示）
//!
//! 未实现（后续扩展）：Varian `procpar`、JEOL Delta、NMReDATA。
//! 未实现格式一律返回 `Unsupported` 并给出可操作的提示，而不是静默失败。

pub mod bruker;
pub mod detect;
pub mod jcamp;
pub mod synthetic;

use mnv_core::{Fid, Result, SourceInfo, Spectrum1D, Vendor};

/// 读入结果：可能是时域 FID（需要处理），也可能是已处理的频域谱（无需 FFT）。
#[derive(Clone, Debug)]
pub enum LoadedData {
    Fid(Fid),
    Spectrum(Spectrum1D),
}

#[derive(Clone, Debug)]
pub struct Loaded {
    pub data: LoadedData,
    pub source: SourceInfo,
    pub title: String,
    /// 读入过程中的提示（例如“使用 pdata 中的处理参数”）
    pub notes: Vec<String>,
}

impl Loaded {
    pub fn vendor(&self) -> Vendor {
        self.source.vendor
    }

    /// 统一的 ppm 范围，供视图初始化。
    pub fn ppm_range(&self) -> Option<(f64, f64)> {
        match &self.data {
            LoadedData::Fid(f) => {
                let half = f.meta.spectral_width_hz / 2.0 / f.meta.spectrometer_mhz.max(1e-9);
                Some((f.meta.carrier_ppm + half, f.meta.carrier_ppm - half))
            }
            LoadedData::Spectrum(s) => Some((s.x.start_ppm, s.x.end_ppm)),
        }
    }

    pub fn nucleus(&self) -> mnv_core::Nucleus {
        match &self.data {
            LoadedData::Fid(f) => f.meta.nucleus.clone(),
            LoadedData::Spectrum(s) => s.meta.nucleus.clone(),
        }
    }
}

/// 统一入口：按路径自动识别并读取。
pub fn load(path: &str) -> Result<Loaded> {
    detect::load_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_demo_loads() {
        let fid = synthetic::demo_ethyl_acetate();
        assert!(fid.len() > 8000);
        assert!((fid.meta.spectrometer_mhz - 400.13).abs() < 0.01);
    }

    #[test]
    fn missing_path_errors_helpfully() {
        let e = load("Z:/definitely/not/here").unwrap_err();
        let text = e.to_string();
        assert!(text.contains("不存在") || text.contains("I/O"));
    }
}
