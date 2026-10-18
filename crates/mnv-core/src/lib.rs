//! mnv-core —— 数据模型层
//!
//! 设计约束：
//! * 本 crate 不依赖任何 GUI；
//! * 所有处理参数可序列化，处理链（history）可导出、可回放；
//! * 谱图对象以 `Arc` 共享，文档快照的代价接近 O(1)，使“快照式撤销”成为可行方案。
//!
//! 全部实现均依据公开教材、公开论文与公开格式规范独立编写。

pub mod analysis;
pub mod axis;
pub mod doc;
pub mod error;
pub mod fid;
pub mod meta;
pub mod nucleus;
pub mod proc_op;
pub mod solvent;
pub mod spectrum;

pub use analysis::{
    Analysis, IntegralRegion, Multiplet, MultipletLabel, Peak, PeakKind, PeakSource,
};
pub use axis::Axis;
pub use doc::{
    CanvasItem, Dataset, Document, InsetItem, Page, Rect01, SpectrumItem, SpectrumStyle, TextItem,
    ViewRange,
};
pub use error::{MnvError, Result};
pub use fid::Fid;
pub use meta::{AcquisitionMeta, SourceInfo, Vendor};
pub use nucleus::Nucleus;
pub use proc_op::{
    ApodKind, BaselineKind, NormMode, ProcOp, ProcRecipe, SmoothKind, PROC_FORMAT_VERSION,
};
pub use solvent::{Solvent, SOLVENTS};
pub use spectrum::Spectrum1D;
