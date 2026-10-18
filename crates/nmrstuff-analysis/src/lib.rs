//! nmrstuff-analysis —— 标峰、积分、多重峰解算。
//!
//! 设计取向：
//! * **不承诺全自动**：默认参数偏保守（阈值较高、间距约束），把“手动 + 智能吸附”做顺；
//! * **可修正**：所有结果都是普通数据，允许用户增删后重算；
//! * **可解释**：每个结论都附带残差 / 置信信息，避免“黑箱给个数字”。

pub mod integrate;
pub mod multiplet;
pub mod peaks;

pub use integrate::{assign_by_area, curve_points, region_sum};
pub use multiplet::{analyze_region, multiplets_from_auto, MultipletParams};
pub use peaks::{pick, PickParams};
