//! 处理步骤的可序列化描述。
//!
//! 这些类型是“处理链可复现”的核心：每一步既驱动计算，也能原样写入 JSON / 报告。
//! 算法实现位于 `mnv-proc`，本文件只描述“做什么”。

use serde::{Deserialize, Serialize};

/// 处理链格式版本，写入项目文件时用于向后兼容判断。
pub const PROC_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApodKind {
    None,
    Exponential,
    Gauss,
    LorentzGauss,
    SineBell,
    SineSquare,
    Trapezoid,
    Hamming,
    Hanning,
}

impl ApodKind {
    pub fn label(&self) -> &'static str {
        match self {
            ApodKind::None => "不加窗",
            ApodKind::Exponential => "指数窗 (LB)",
            ApodKind::Gauss => "高斯窗 (GM)",
            ApodKind::LorentzGauss => "洛伦兹-高斯 (LG)",
            ApodKind::SineBell => "正弦钟 (SINE)",
            ApodKind::SineSquare => "正弦平方 (SINSQ)",
            ApodKind::Trapezoid => "梯形窗",
            ApodKind::Hamming => "Hamming",
            ApodKind::Hanning => "Hanning",
        }
    }

    pub const ALL: [ApodKind; 9] = [
        ApodKind::None,
        ApodKind::Exponential,
        ApodKind::Gauss,
        ApodKind::LorentzGauss,
        ApodKind::SineBell,
        ApodKind::SineSquare,
        ApodKind::Trapezoid,
        ApodKind::Hamming,
        ApodKind::Hanning,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaselineKind {
    None,
    Whittaker,
    Snip,
    Polynomial,
    MovingAverage,
}

impl BaselineKind {
    pub fn label(&self) -> &'static str {
        match self {
            BaselineKind::None => "不校正",
            BaselineKind::Whittaker => "Whittaker (asLS)",
            BaselineKind::Snip => "SNIP",
            BaselineKind::Polynomial => "多项式拟合",
            BaselineKind::MovingAverage => "移动平均基线",
        }
    }

    pub const ALL: [BaselineKind; 5] = [
        BaselineKind::None,
        BaselineKind::Whittaker,
        BaselineKind::Snip,
        BaselineKind::Polynomial,
        BaselineKind::MovingAverage,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmoothKind {
    None,
    SavitzkyGolay,
    MovingAverage,
}

impl SmoothKind {
    pub fn label(&self) -> &'static str {
        match self {
            SmoothKind::None => "不平滑",
            SmoothKind::SavitzkyGolay => "Savitzky-Golay",
            SmoothKind::MovingAverage => "移动平均",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormMode {
    None,
    Max,
    Sum,
}

impl NormMode {
    pub fn label(&self) -> &'static str {
        match self {
            NormMode::None => "不归一化",
            NormMode::Max => "最大峰归一",
            NormMode::Sum => "面积和归一",
        }
    }
}

/// 单个处理步骤。参数全部内联，便于直接序列化与回放。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProcOp {
    Apodization {
        kind: ApodKind,
        /// 线展宽 LB（Hz），指数窗常用 0.3（¹H）/ 1–3（¹³C）
        lb_hz: f64,
        /// 高斯展宽 GB（Hz）
        gb_hz: f64,
        /// 正弦窗的相移（0.0 = 标准 sine-bell，0.5 = 位移 pi/2）
        sine_shift: f64,
    },
    ZeroFill {
        /// 目标点数倍数，结果取不小于 `points * factor` 的 2 的幂
        factor: usize,
    },
    Fft,
    /// 数字滤波群延迟补偿（按采样点数给出的线性相位）
    GroupDelay {
        points: f64,
    },
    Phase {
        ph0_deg: f64,
        /// 谱宽两端的总相位差（度）。正方向 = 向低 ppm 递增。
        ph1_deg: f64,
        automatic: bool,
    },
    Baseline {
        kind: BaselineKind,
        /// Whittaker 平滑强度（常用 1e4 – 1e7）
        lambda: f64,
        /// 多项式阶数 / 移动平均窗口
        order: usize,
        iterations: usize,
    },
    Smooth {
        kind: SmoothKind,
        window: usize,
    },
    /// 单点定标：把 `reference_ppm` 处的峰移动到 `target_ppm`。
    Calibrate {
        reference_ppm: Option<f64>,
        target_ppm: f64,
        /// 直接整体平移量（当 `reference_ppm` 为 None 时使用）
        delta_ppm: f64,
    },
    Normalize {
        mode: NormMode,
    },
}

impl ProcOp {
    /// 一行人类可读描述，用于处理链列表、报告与撤销标签。
    pub fn describe(&self) -> String {
        match self {
            ProcOp::Apodization { kind, lb_hz, gb_hz, sine_shift } => match kind {
                ApodKind::None => "窗函数：无".into(),
                ApodKind::Exponential => format!("指数窗 LB = {lb_hz:.3} Hz"),
                ApodKind::Gauss => format!("高斯窗 LB = {lb_hz:.3} Hz, GB = {gb_hz:.3} Hz"),
                ApodKind::LorentzGauss => {
                    format!("LG 窗 LB = {lb_hz:.3} Hz, GB = {gb_hz:.3} Hz")
                }
                ApodKind::SineBell | ApodKind::SineSquare => {
                    format!("{} 相移 {sine_shift:.2}", kind.label())
                }
                _ => kind.label().to_string(),
            },
            ProcOp::ZeroFill { factor } => format!("零填充 ×{factor}"),
            ProcOp::Fft => "傅里叶变换".into(),
            ProcOp::GroupDelay { points } => format!("群延迟校正 {points:.2} 点"),
            ProcOp::Phase { ph0_deg, ph1_deg, automatic } => {
                if *automatic {
                    format!("自动相位（φ0 = {ph0_deg:.2}°, φ1 = {ph1_deg:.2}°）")
                } else {
                    format!("相位校正 φ0 = {ph0_deg:.2}°, φ1 = {ph1_deg:.2}°")
                }
            }
            ProcOp::Baseline { kind, lambda, order, .. } => match kind {
                BaselineKind::Whittaker => format!("基线：Whittaker λ = {lambda:.0}"),
                BaselineKind::Polynomial => format!("基线：{order} 阶多项式"),
                BaselineKind::MovingAverage => format!("基线：移动平均窗口 {order}"),
                _ => format!("基线：{}", kind.label()),
            },
            ProcOp::Smooth { kind, window } => format!("平滑：{} 窗口 {window}", kind.label()),
            ProcOp::Calibrate { reference_ppm, target_ppm, delta_ppm } => match reference_ppm {
                Some(r) => format!("定标：{r:.3} ppm → {target_ppm:.3} ppm"),
                None => format!("定标：平移 {delta_ppm:+.4} ppm"),
            },
            ProcOp::Normalize { mode } => format!("归一化：{}", mode.label()),
        }
    }

    /// 是否改变了数据的实值形态（用于判断何时需要重建缓存）。
    pub fn touches_spectrum(&self) -> bool {
        true
    }
}

/// 一条完整处理链 —— 项目的“可复现凭据”。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcRecipe {
    pub version: u32,
    pub steps: Vec<ProcOp>,
}

impl Default for ProcRecipe {
    fn default() -> Self {
        Self {
            version: PROC_FORMAT_VERSION,
            steps: recommended_1h(),
        }
    }
}

impl ProcRecipe {
    pub fn new(steps: Vec<ProcOp>) -> Self {
        Self {
            version: PROC_FORMAT_VERSION,
            steps,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn from_json(s: &str) -> crate::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    /// 处理链文本（每行一步），用于报告与 CLI 输出。
    pub fn describe_lines(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.describe()).collect()
    }

    /// 找到第一个匹配的步骤的可变引用。
    pub fn find_mut<F: Fn(&ProcOp) -> bool>(&mut self, pred: F) -> Option<&mut ProcOp> {
        self.steps.iter_mut().find(|s| pred(s))
    }
}

/// ¹H 推荐处理链。
///
/// 参数依据公开资料中常见的默认值（指数窗 0.3 Hz、零填充至 2 的幂、
/// Whittaker 基线），非任何商业软件的预设文件。
pub fn recommended_1h() -> Vec<ProcOp> {
    vec![
        ProcOp::Apodization {
            kind: ApodKind::Exponential,
            lb_hz: 0.3,
            gb_hz: 0.0,
            sine_shift: 0.0,
        },
        ProcOp::ZeroFill { factor: 2 },
        ProcOp::Fft,
        ProcOp::Phase {
            ph0_deg: 0.0,
            ph1_deg: 0.0,
            automatic: true,
        },
        ProcOp::Baseline {
            kind: BaselineKind::Whittaker,
            lambda: 1.0e5,
            order: 2,
            iterations: 10,
        },
        ProcOp::Calibrate {
            reference_ppm: None,
            target_ppm: 0.0,
            delta_ppm: 0.0,
        },
    ]
}

/// ¹³C 推荐处理链：更宽的窗函数，不积分。
pub fn recommended_13c() -> Vec<ProcOp> {
    vec![
        ProcOp::Apodization {
            kind: ApodKind::Exponential,
            lb_hz: 1.0,
            gb_hz: 0.0,
            sine_shift: 0.0,
        },
        ProcOp::ZeroFill { factor: 2 },
        ProcOp::Fft,
        ProcOp::Phase {
            ph0_deg: 0.0,
            ph1_deg: 0.0,
            automatic: true,
        },
        ProcOp::Baseline {
            kind: BaselineKind::Whittaker,
            lambda: 1.0e6,
            order: 2,
            iterations: 10,
        },
        ProcOp::Calibrate {
            reference_ppm: None,
            target_ppm: 0.0,
            delta_ppm: 0.0,
        },
    ]
}

/// 纯时域步骤（不做 FT），用于“只导出 FID”场景。
pub fn recommended_fid_only() -> Vec<ProcOp> {
    vec![ProcOp::Apodization {
        kind: ApodKind::Exponential,
        lb_hz: 0.3,
        gb_hz: 0.0,
        sine_shift: 0.0,
    }]
}
