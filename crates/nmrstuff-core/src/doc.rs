//! 文档—页面—画布对象模型（三层结构，v0.1 简化版）。
//!
//! * `Document` 持有原始 FID、若干数据集（谱图 + 分析结果）以及一页画布；
//! * `Page` 中的对象用归一化坐标（0–1）定位，与纸张尺寸解耦，便于缩放与导出；
//! * 谱图数据用 `Arc` 共享，因此 `Document::clone()` 极其廉价 —— 撤销栈直接存快照。

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::analysis::Analysis;
use crate::fid::Fid;
use crate::proc_op::ProcRecipe;
use crate::spectrum::Spectrum1D;

/// 归一化矩形（相对页面宽高）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect01 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect01 {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    pub fn intersect(&self, other: &Rect01) -> bool {
        !(other.x > self.x + self.w
            || other.x + other.w < self.x
            || other.y > self.y + self.h
            || other.y + other.h < self.y)
    }

    pub fn clamp_inside(&mut self) {
        self.w = self.w.clamp(0.02, 1.0);
        self.h = self.h.clamp(0.02, 1.0);
        self.x = self.x.clamp(0.0, 1.0 - self.w);
        self.y = self.y.clamp(0.0, 1.0 - self.h);
    }
}

/// 谱图显示窗口（ppm，from 通常大于 to）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewRange {
    pub from_ppm: f64,
    pub to_ppm: f64,
}

impl ViewRange {
    pub fn new(from_ppm: f64, to_ppm: f64) -> Self {
        if from_ppm >= to_ppm {
            Self {
                from_ppm,
                to_ppm,
            }
        } else {
            Self {
                from_ppm: to_ppm,
                to_ppm: from_ppm,
            }
        }
    }

    pub fn span(&self) -> f64 {
        (self.from_ppm - self.to_ppm).abs()
    }

    pub fn center(&self) -> f64 {
        (self.from_ppm + self.to_ppm) * 0.5
    }

    pub fn contains(&self, ppm: f64) -> bool {
        ppm <= self.from_ppm && ppm >= self.to_ppm
    }

    /// 以中心缩放：`factor < 1` 放大。
    pub fn zoomed(&self, factor: f64, anchor_ppm: f64) -> Self {
        let span = (self.span() * factor).clamp(1e-4, 1.0e5);
        let anchor = anchor_ppm.clamp(self.to_ppm, self.from_ppm);
        // 让 anchor 在窗口中的相对位置保持不变
        let rel = if self.span() > 0.0 {
            (self.from_ppm - anchor) / self.span()
        } else {
            0.5
        };
        let from = anchor + rel * span;
        ViewRange::new(from, from - span)
    }

    pub fn panned(&self, delta_ppm: f64) -> Self {
        ViewRange::new(self.from_ppm + delta_ppm, self.to_ppm + delta_ppm)
    }

    /// 限制在数据轴范围内（允许少量留白）。
    pub fn clamped_to(&self, data: ViewRange, margin_ratio: f64) -> Self {
        let margin = data.span() * margin_ratio;
        let lo = data.to_ppm - margin;
        let hi = data.from_ppm + margin;
        let span = self.span().min(hi - lo);
        let mut from = self.from_ppm;
        if from > hi {
            from = hi;
        }
        if from - span < lo {
            from = lo + span;
        }
        ViewRange::new(from, from - span)
    }
}

impl Default for ViewRange {
    fn default() -> Self {
        Self::new(12.0, -1.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpectrumStyle {
    pub line_width: f32,
    /// RGBA
    pub color: [u8; 4],
    pub show_axis: bool,
    pub show_peaks: bool,
    pub show_integrals: bool,
    pub show_multiplets: bool,
    pub show_grid: bool,
    pub font_pt: f32,
    /// 峰标注保留小数位
    pub decimals: usize,
}

impl Default for SpectrumStyle {
    fn default() -> Self {
        Self {
            line_width: 1.2,
            color: [20, 30, 40, 255],
            show_axis: true,
            show_peaks: true,
            show_integrals: true,
            show_multiplets: true,
            show_grid: false,
            font_pt: 12.0,
            decimals: 2,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpectrumItem {
    pub dataset: usize,
    pub rect: Rect01,
    pub view: ViewRange,
    pub style: SpectrumStyle,
}

impl SpectrumItem {
    pub fn new(dataset: usize, rect: Rect01, view: ViewRange) -> Self {
        Self {
            dataset,
            rect,
            view,
            style: SpectrumStyle::default(),
        }
    }
}

/// 局部放大插图。`source` 为母谱上的取景范围，画布上以连接框标示。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InsetItem {
    pub dataset: usize,
    pub rect: Rect01,
    pub view: ViewRange,
    pub source: ViewRange,
    pub style: SpectrumStyle,
    pub linked: bool,
}

impl InsetItem {
    pub fn new(dataset: usize, rect: Rect01, view: ViewRange, source: ViewRange) -> Self {
        let mut style = SpectrumStyle::default();
        style.font_pt = 7.0;
        style.line_width = 1.0;
        Self {
            dataset,
            rect,
            view,
            source,
            style,
            linked: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextItem {
    pub rect: Rect01,
    pub text: String,
    pub size_pt: f32,
    pub bold: bool,
    pub color: [u8; 4],
}

impl TextItem {
    pub fn new(rect: Rect01, text: &str, size_pt: f32) -> Self {
        Self {
            rect,
            text: text.to_string(),
            size_pt,
            bold: false,
            color: [20, 30, 40, 255],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CanvasItem {
    Spectrum(SpectrumItem),
    Inset(InsetItem),
    Text(TextItem),
}

impl CanvasItem {
    pub fn rect(&self) -> Rect01 {
        match self {
            CanvasItem::Spectrum(s) => s.rect,
            CanvasItem::Inset(i) => i.rect,
            CanvasItem::Text(t) => t.rect,
        }
    }

    pub fn rect_mut(&mut self) -> &mut Rect01 {
        match self {
            CanvasItem::Spectrum(s) => &mut s.rect,
            CanvasItem::Inset(i) => &mut i.rect,
            CanvasItem::Text(t) => &mut t.rect,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            CanvasItem::Spectrum(_) => "谱图",
            CanvasItem::Inset(_) => "局部放大",
            CanvasItem::Text(_) => "文本",
        }
    }

    pub fn dataset(&self) -> Option<usize> {
        match self {
            CanvasItem::Spectrum(s) => Some(s.dataset),
            CanvasItem::Inset(i) => Some(i.dataset),
            CanvasItem::Text(_) => None,
        }
    }

    pub fn style(&self) -> Option<&SpectrumStyle> {
        match self {
            CanvasItem::Spectrum(s) => Some(&s.style),
            CanvasItem::Inset(i) => Some(&i.style),
            CanvasItem::Text(_) => None,
        }
    }

    pub fn style_mut(&mut self) -> Option<&mut SpectrumStyle> {
        match self {
            CanvasItem::Spectrum(s) => Some(&mut s.style),
            CanvasItem::Inset(i) => Some(&mut i.style),
            CanvasItem::Text(_) => None,
        }
    }

    pub fn view(&self) -> Option<ViewRange> {
        match self {
            CanvasItem::Spectrum(s) => Some(s.view),
            CanvasItem::Inset(i) => Some(i.view),
            CanvasItem::Text(_) => None,
        }
    }

    pub fn view_mut(&mut self) -> Option<&mut ViewRange> {
        match self {
            CanvasItem::Spectrum(s) => Some(&mut s.view),
            CanvasItem::Inset(i) => Some(&mut i.view),
            CanvasItem::Text(_) => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    /// 纸张尺寸（mm）。A4 纵向 210×297。
    pub width_mm: f32,
    pub height_mm: f32,
    pub items: Vec<CanvasItem>,
    pub show_margin: bool,
}

impl Default for Page {
    fn default() -> Self {
        Self::a4_1d()
    }
}

impl Page {
    /// 一维谱的标准版式：标题 + 主谱 + 峰表区。
    pub fn a4_1d() -> Self {
        Self {
            width_mm: 210.0,
            height_mm: 297.0,
            items: vec![
                CanvasItem::Text(TextItem::new(
                    Rect01::new(0.06, 0.035, 0.88, 0.03),
                    "¹H NMR (400 MHz, CDCl₃)",
                    12.0,
                )),
                CanvasItem::Spectrum(SpectrumItem::new(
                    0,
                    Rect01::new(0.06, 0.09, 0.88, 0.42),
                    ViewRange::new(12.0, -1.0),
                )),
                CanvasItem::Text(TextItem::new(
                    Rect01::new(0.06, 0.55, 0.88, 0.08),
                    "峰表 / 多重峰表（框选多重峰后可用“报告”生成 JACS 数据行）",
                    9.0,
                )),
            ],
            show_margin: false,
        }
    }

    pub fn spectrum_item_index(&self) -> Option<usize> {
        self.items
            .iter()
            .position(|i| matches!(i, CanvasItem::Spectrum(_)))
    }

    pub fn primary_item_mut(&mut self) -> Option<&mut SpectrumItem> {
        self.items.iter_mut().find_map(|i| match i {
            CanvasItem::Spectrum(s) => Some(s),
            _ => None,
        })
    }

    pub fn primary_item(&self) -> Option<&SpectrumItem> {
        self.items.iter().find_map(|i| match i {
            CanvasItem::Spectrum(s) => Some(s),
            _ => None,
        })
    }

    /// 页面宽高比，供画布等比缩放。
    pub fn aspect(&self) -> f32 {
        self.width_mm / self.height_mm
    }
}

/// 数据集 = 一张谱 + 它的分析结果。
#[derive(Clone, Debug)]
pub struct Dataset {
    pub name: String,
    pub spectrum: Arc<Spectrum1D>,
    pub analysis: Analysis,
    pub visible: bool,
}

impl Dataset {
    pub fn new(name: impl Into<String>, spectrum: Arc<Spectrum1D>) -> Self {
        Self {
            name: name.into(),
            spectrum,
            analysis: Analysis::default(),
            visible: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    pub title: String,
    pub path: Option<PathBuf>,
    pub fid: Option<Arc<Fid>>,
    pub datasets: Vec<Dataset>,
    pub page: Page,
    pub recipe: ProcRecipe,
    pub dirty: bool,
}

impl Document {
    pub fn empty(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            path: None,
            fid: None,
            datasets: Vec::new(),
            page: Page::a4_1d(),
            recipe: ProcRecipe::default(),
            dirty: false,
        }
    }

    pub fn with_fid(title: impl Into<String>, fid: Fid, spectrum: Spectrum1D) -> Self {
        let mut doc = Self::empty(title);
        doc.page.items.retain(|i| !matches!(i, CanvasItem::Spectrum(_)));
        doc.page.items.insert(
            1,
            CanvasItem::Spectrum(SpectrumItem::new(
                0,
                Rect01::new(0.06, 0.09, 0.88, 0.30),
                ViewRange::new(spectrum.x.start_ppm, spectrum.x.end_ppm),
            )),
        );
        doc.fid = Some(Arc::new(fid));
        doc.datasets.push(Dataset::new("1D", Arc::new(spectrum)));
        doc.dirty = false;
        doc
    }

    pub fn spectrum(&self, dataset: usize) -> Option<&Spectrum1D> {
        self.datasets.get(dataset).map(|d| d.spectrum.as_ref())
    }

    pub fn analysis(&self, dataset: usize) -> Option<&Analysis> {
        self.datasets.get(dataset).map(|d| &d.analysis)
    }

    pub fn analysis_mut(&mut self, dataset: usize) -> Option<&mut Analysis> {
        self.datasets.get_mut(dataset).map(|d| &mut d.analysis)
    }

    /// 当前主谱（第一个数据集）。
    pub fn primary(&self) -> Option<&Dataset> {
        self.datasets.first()
    }

    pub fn primary_spectrum(&self) -> Option<&Spectrum1D> {
        self.datasets.first().map(|d| d.spectrum.as_ref())
    }

    /// 数据本身的 ppm 范围。
    pub fn data_range(&self) -> Option<ViewRange> {
        self.primary_spectrum()
            .map(|s| ViewRange::new(s.x.start_ppm, s.x.end_ppm))
    }

    /// 设备描述，用于标题与状态栏。
    pub fn instrument_text(&self) -> String {
        match self.primary_spectrum() {
            Some(s) => format!(
                "{:.0} MHz · {}",
                s.meta.spectrometer_mhz,
                s.meta.nucleus.display()
            ),
            None => "—".into(),
        }
    }
}
