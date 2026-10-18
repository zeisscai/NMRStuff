//! nmrstuff-render —— 渲染几何层（**不依赖 GUI**）。
//!
//! 只做“数值 → 屏幕坐标”的换算与布局决策，实际绘制由 `nmrstuff-app` 调用 egui 完成。
//! 这样做的原因：
//! * 矢量导出（SVG/PDF）可以直接复用同一套几何；
//! * 布局逻辑可单测（黄金几何测试），不必起窗口；
//! * 保持分层原则：`render` 不依赖 `app`。

pub mod envelope;
pub mod label;
pub mod ticks;
pub mod transform;

pub use envelope::build_trace;
pub use label::{layout_peak_labels, LabelBox};
pub use ticks::{next_nice_step, nice_ticks, tick_text};
pub use transform::{ScreenRect, Transform};
