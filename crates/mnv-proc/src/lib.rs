//! mnv-proc —— 一维 NMR 处理管线。
//!
//! 全部算法依据公开教材与论文独立实现，未参考任何商业软件的实现细节：
//! * 窗函数：数字信号处理教科书公式（见 `apodize` 内逐条注释）；
//! * FFT：`rustfft`（MIT/Apache）；
//! * 相位：公开的“熵最小化 + 负值惩罚”目标函数（`phase`）；
//! * 基线：Whittaker/asLS（Eilers 2003）、SNIP（Ryan 1988）、多项式拟合、移动平均；
//! * 平滑：Savitzky-Golay（Savitzky & Golay 1964）。
//!
//! 管线设计目标：**步骤即数据**。每一步都是 `mnv_core::ProcOp`，
//! 处理完成后原样写入 `Spectrum1D::history`，可导出、可回放。

pub mod apodize;
pub mod baseline;
pub mod fft;
pub mod phase;
pub mod pipeline;
pub mod smooth;

pub use pipeline::{process, ProcReport};
