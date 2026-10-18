//! 分析结果的数据结构。
//!
//! 算法在 `mnv-analysis`，类型定义放这里，避免 `analysis` 依赖具体算法实现。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeakKind {
    Compound,
    Solvent,
    Impurity,
    Satellite,
    Artifact,
}

impl PeakKind {
    pub fn label(&self) -> &'static str {
        match self {
            PeakKind::Compound => "样品峰",
            PeakKind::Solvent => "溶剂峰",
            PeakKind::Impurity => "杂质峰",
            PeakKind::Satellite => "卫星峰",
            PeakKind::Artifact => "伪峰",
        }
    }

    /// 报告中是否默认剔除。
    pub fn is_excluded_from_report(&self) -> bool {
        matches!(self, PeakKind::Solvent | PeakKind::Artifact)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeakSource {
    Manual,
    Automatic,
    Multiplet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Peak {
    pub ppm: f64,
    pub height: f32,
    /// 半高宽（Hz），由局部曲线估算，仅作参考
    pub width_hz: f32,
    /// 面积（积分和，未归一）
    pub area: f64,
    pub kind: PeakKind,
    pub source: PeakSource,
    /// 手动指定的氢数（多重峰归一化后写回）
    pub assigned_h: Option<f32>,
    pub note: Option<String>,
}

impl Peak {
    pub fn new(ppm: f64, height: f32) -> Self {
        Self {
            ppm,
            height,
            width_hz: 0.0,
            area: 0.0,
            kind: PeakKind::Compound,
            source: PeakSource::Automatic,
            assigned_h: None,
            note: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegralRegion {
    pub from_ppm: f64,
    pub to_ppm: f64,
    /// 归一化后的数值（用户可改成 3H 等）
    pub value: f32,
    /// 原始积分和（含线性基线扣除）
    pub raw_sum: f64,
    /// 是否被用户手动改过数值
    pub edited: bool,
    /// 曲线显示高度（0–1，占视图高度比例）
    pub curve_height: f32,
}

impl IntegralRegion {
    pub fn new(from_ppm: f64, to_ppm: f64, raw_sum: f64) -> Self {
        Self {
            from_ppm,
            to_ppm,
            value: 1.0,
            raw_sum,
            edited: false,
            curve_height: 0.18,
        }
    }

    pub fn width_ppm(&self) -> f64 {
        (self.from_ppm - self.to_ppm).abs()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MultipletLabel {
    S,
    D,
    T,
    Q,
    Quin,
    Sext,
    Sept,
    Dd,
    Dt,
    Td,
    M,
    Unresolved,
}

impl MultipletLabel {
    pub fn symbol(&self) -> &'static str {
        match self {
            MultipletLabel::S => "s",
            MultipletLabel::D => "d",
            MultipletLabel::T => "t",
            MultipletLabel::Q => "q",
            MultipletLabel::Quin => "quint",
            MultipletLabel::Sext => "sext",
            MultipletLabel::Sept => "sept",
            MultipletLabel::Dd => "dd",
            MultipletLabel::Dt => "dt",
            MultipletLabel::Td => "td",
            MultipletLabel::M => "m",
            MultipletLabel::Unresolved => "m",
        }
    }

    pub fn from_peak_count(n: usize) -> Self {
        match n {
            1 => MultipletLabel::S,
            2 => MultipletLabel::D,
            3 => MultipletLabel::T,
            4 => MultipletLabel::Q,
            5 => MultipletLabel::Quin,
            6 => MultipletLabel::Sext,
            7 => MultipletLabel::Sept,
            _ => MultipletLabel::M,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Multiplet {
    pub from_ppm: f64,
    pub to_ppm: f64,
    /// 指向 `Analysis::peaks` 的下标
    pub peaks: Vec<usize>,
    pub j_values_hz: Vec<f64>,
    /// 积分面积（已按数值归一）
    pub integral: f64,
    pub integral_value: f32,
    pub label: MultipletLabel,
    /// 例如 `2H`；也用于报告
    pub assignment: Option<String>,
    /// 一阶模型拟合残差（Hz），越大越应提示“疑似高阶耦合”
    pub fit_residual_hz: f64,
}

impl Multiplet {
    pub fn peak_count(&self) -> usize {
        self.peaks.len()
    }

    pub fn center_ppm(&self) -> f64 {
        (self.from_ppm + self.to_ppm) / 2.0
    }

    /// 报告用区间字符串：单峰（区间宽度为 0）用点值，多重峰用范围。
    pub fn range_text(&self, decimals: usize) -> String {
        if (self.from_ppm - self.to_ppm).abs() < 1e-9 {
            format!("{:.*}", decimals, self.center_ppm())
        } else {
            let hi = self.from_ppm.max(self.to_ppm);
            let lo = self.from_ppm.min(self.to_ppm);
            // 端点含负数时用带空格的连接符，避免出现 “0.00–-0.03” 这种歧义写法
            let sep = if lo < 0.0 { " – " } else { "–" };
            format!("{:.*}{}{:.*}", decimals, hi, sep, decimals, lo)
        }
    }

    pub fn j_text(&self, decimals: usize) -> Option<String> {
        if self.j_values_hz.is_empty() {
            return None;
        }
        let mut uniq: Vec<f64> = Vec::new();
        for j in &self.j_values_hz {
            if !uniq.iter().any(|u| (u - j).abs() < 0.15) {
                uniq.push(*j);
            }
        }
        let parts: Vec<String> = uniq
            .iter()
            .map(|j| format!("{:.*}", decimals, j))
            .collect();
        Some(parts.join(", "))
    }
}

/// 单个数据集的全部分析结果。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub peaks: Vec<Peak>,
    pub integrals: Vec<IntegralRegion>,
    pub multiplets: Vec<Multiplet>,
}

impl Analysis {
    pub fn clear(&mut self) {
        self.peaks.clear();
        self.integrals.clear();
        self.multiplets.clear();
    }

    /// 峰值按 ppm 降序（与轴方向一致）。
    pub fn sorted_peak_indices(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.peaks.len()).collect();
        idx.sort_by(|a, b| {
            self.peaks[*b]
                .ppm
                .partial_cmp(&self.peaks[*a].ppm)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        idx
    }

    /// 报告用：把归属的氢数按总积分归一化到最接近的整数比。
    pub fn normalize_integrals(&mut self, target_total_h: f32) {
        let total: f32 = self.integrals.iter().map(|i| i.value).sum();
        if total <= 0.0 {
            return;
        }
        let scale = target_total_h / total;
        for i in &mut self.integrals {
            i.value = (i.value * scale * 2.0).round() / 2.0;
        }
    }

    pub fn find_integral_at(&self, ppm: f64) -> Option<usize> {
        self.integrals.iter().position(|i| {
            ppm <= i.from_ppm.max(i.to_ppm) && ppm >= i.from_ppm.min(i.to_ppm)
        })
    }
}
