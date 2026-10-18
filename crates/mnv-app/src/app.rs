//! 应用编排层：状态、撤销栈、动作与快捷键。
//!
//! 设计原则：
//! * `mnv-app` 只做“编排”，不实现算法；
//! * 文档快照极便宜（谱图用 `Arc` 共享），因此撤销栈直接存整份 `Document`；
//! * 任何改变数据的操作都必须成对出现 `let before = doc.clone()` / `push_undo()`。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mnv_analysis::{analyze_region, pick, region_sum, MultipletParams, PickParams};
use mnv_core::{
    AcquisitionMeta, CanvasItem, Dataset, Document, IntegralRegion, Nucleus, ProcOp, ProcRecipe,
    Rect01, Spectrum1D, TextItem, ViewRange,
};
use mnv_formats::LoadedData;
use mnv_report::JournalStyle;

pub const HELP: &str = "\
快捷键：
  Z  框选放大        F  全谱          滚轮  以光标为中心缩放
  K  逐峰标峰        I  积分          J  多重峰
  L  定标（点峰）    B  基线校正开关   Shift+P  自动相位
  Ctrl+Z / Ctrl+Y  撤销 / 重做        Esc 取消当前工具/框选
  Delete 删除选中的积分区间

交互：
  · 选择工具下拖动谱图 = 移动版面中的对象（对应“页面排版”）
  · 鼠标中键拖动 = 平移视图
  · 右侧面板可修改处理链参数并重新处理";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Zoom,
    Peak,
    Integral,
    Multiplet,
    Reference,
}

impl Tool {
    pub fn label(&self) -> &'static str {
        match self {
            Tool::Select => "选择/移动",
            Tool::Zoom => "框选放大",
            Tool::Peak => "标峰",
            Tool::Integral => "积分",
            Tool::Multiplet => "多重峰",
            Tool::Reference => "定标",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self {
            Tool::Select => "拖动谱图移动对象，中键拖动平移",
            Tool::Zoom => "在谱图上框选放大",
            Tool::Integral => "横向拖动划出积分区间",
            Tool::Multiplet => "框选一个多重峰，自动解算 δ / J / 积分",
            Tool::Reference => "点击一个已知峰，把它定标到目标位移",
            Tool::Peak => "点击谱峰添加标注（K 可自动标峰）",
        }
    }
}

/// 框选/拖动过程中的临时状态。
#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    None,
    /// 框选（ppm 区间），用于放大/积分/多重峰
    Span { start: f64, current: f64 },
    /// 平移视图
    Pan {
        start_x: f32,
        view: ViewRange,
    },
    /// 移动版面对象
    MoveItem {
        item: usize,
        grab: (f32, f32),
        rect: Rect01,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct CursorInfo {
    pub ppm: f64,
    pub intensity: f32,
    pub hz: f64,
}

pub struct HistoryEntry {
    pub label: String,
    pub before: Document,
    pub after: Document,
}

#[derive(Default)]
pub struct History {
    pub undo: Vec<HistoryEntry>,
    pub redo: Vec<HistoryEntry>,
}

impl History {
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn push(&mut self, label: impl Into<String>, before: Document, after: Document) {
        self.undo.push(HistoryEntry {
            label: label.into(),
            before,
            after,
        });
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        let e = self.undo.pop()?;
        let label = e.label.clone();
        *doc = e.before.clone();
        self.redo.push(e);
        Some(label)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        let e = self.redo.pop()?;
        let label = e.label.clone();
        *doc = e.after.clone();
        self.undo.push(e);
        Some(label)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

pub struct App {
    pub docs: Vec<Document>,
    pub active: usize,
    pub history: History,

    pub tool: Tool,
    pub drag: Drag,
    pub cursor: Option<CursorInfo>,
    pub selected_item: Option<usize>,
    pub selected_peak: Option<usize>,

    pub browser_dir: PathBuf,
    pub path_input: String,

    pub status: String,
    pub error: Option<String>,
    pub show_help: bool,
    pub show_report: bool,
    pub show_recipe: bool,
    pub report_text: String,
    pub journal: JournalStyle,

    pub pick_params: PickParams,
    pub multiplet_params: MultipletParams,
    pub integrate_target_h: f32,
    pub ref_target_ppm: f64,

    pub page_zoom: f32,
    pub frame_ms: f32,
    pub fonts_ok: bool,
    /// 处理参数改动后需要重新处理（避免拖动滑块时每帧都跑 FFT）
    pub proc_dirty: bool,
    pub drag_before: Option<Document>,
    pub last_action: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let fonts_ok = crate::theme::install_fonts(&cc.egui_ctx);
        crate::theme::apply(&cc.egui_ctx);
        Self::build(fonts_ok)
    }

    /// 不依赖窗口/GPU 的构造，供无头测试使用。
    fn build(fonts_ok: bool) -> Self {
        let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut app = Self {
            docs: Vec::new(),
            active: 0,
            history: History::default(),
            tool: Tool::Select,
            drag: Drag::None,
            cursor: None,
            selected_item: None,
            selected_peak: None,
            path_input: dir.to_string_lossy().to_string(),
            browser_dir: dir,
            status: "就绪".into(),
            error: None,
            show_help: false,
            show_report: false,
            show_recipe: false,
            report_text: String::new(),
            journal: JournalStyle::Jacs,
            pick_params: PickParams::default(),
            multiplet_params: MultipletParams::default(),
            integrate_target_h: 1.0,
            ref_target_ppm: 0.0,
            page_zoom: 1.0,
            frame_ms: 0.0,
            fonts_ok,
            proc_dirty: false,
            drag_before: None,
            last_action: Instant::now(),
        };
        app.load_demo();
        app
    }

    // ---------- 便捷访问 ----------

    pub fn doc(&self) -> Option<&Document> {
        self.docs.get(self.active)
    }

    pub fn doc_mut(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    pub fn spectrum(&self) -> Option<Arc<Spectrum1D>> {
        self.doc()
            .and_then(|d| d.datasets.first())
            .map(|d| d.spectrum.clone())
    }

    pub fn view(&self) -> Option<ViewRange> {
        self.doc()
            .and_then(|d| d.page.primary_item())
            .map(|i| i.view)
    }

    pub fn set_view(&mut self, v: ViewRange) {
        if let Some(doc) = self.doc_mut() {
            if let Some(item) = doc.page.primary_item_mut() {
                item.view = v;
            }
        }
    }

    pub fn data_range(&self) -> Option<ViewRange> {
        self.spectrum()
            .map(|s| ViewRange::new(s.x.start_ppm, s.x.end_ppm))
    }

    pub fn push_undo(&mut self, label: &str, before: Document) {
        if let Some(after) = self.doc().cloned() {
            self.history.push(label, before, after);
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.last_action = Instant::now();
    }

    pub fn fail(&mut self, e: impl std::fmt::Display) {
        self.error = Some(e.to_string());
    }

    // ---------- 载入 ----------

    pub fn load_demo(&mut self) {
        let fid = mnv_formats::synthetic::demo_ethyl_acetate();
        let recipe = ProcRecipe::default();
        match mnv_proc::process(&fid, &recipe.steps) {
            Ok(out) => {
                let spec = out.spectrum;
                let view = ViewRange::new(spec.x.start_ppm, spec.x.end_ppm);
                let mut doc = self.make_doc("演示：乙酸乙酯 (CDCl₃)", &spec, view);
                doc.fid = Some(Arc::new(fid));
                doc.recipe = recipe;
                doc.page = build_page(view, &spec, "¹H NMR (400 MHz, CDCl₃)");
                self.docs = vec![doc];
                self.active = 0;
                self.auto_pick_silent();
                self.history.clear();
                self.status = format!(
                    "已载入内置演示数据（{} 点，处理耗时 {:.0} ms）",
                    spec_len(&self.docs[0]),
                    out.report.elapsed_ms
                );
            }
            Err(e) => self.fail(e),
        }
    }

    pub fn open_path(&mut self, path: &str) {
        match mnv_formats::load(path) {
            Ok(loaded) => {
                let title = short_title(path);
                match loaded.data {
                    LoadedData::Fid(fid) => {
                        let steps = match fid.meta.nucleus {
                            Nucleus::C13 => mnv_core::proc_op::recommended_13c(),
                            _ => mnv_core::proc_op::recommended_1h(),
                        };
                        let recipe = ProcRecipe::new(steps);
                        match mnv_proc::process(&fid, &recipe.steps) {
                            Ok(out) => {
                                let mut spec = out.spectrum;
                                spec.source = loaded.source.clone();
                                let view = ViewRange::new(spec.x.start_ppm, spec.x.end_ppm);
                                let mut doc = self.make_doc(&title, &spec, view);
                                doc.fid = Some(Arc::new(fid));
                                doc.recipe = recipe;
                                doc.path = Some(PathBuf::from(path));
                                let label = format!(
                                    "{} NMR ({:.0} MHz)",
                                    spec.meta.nucleus.display(),
                                    spec.meta.spectrometer_mhz
                                );
                                doc.page = build_page(view, &spec, &label);
                                self.docs = vec![doc];
                                self.active = 0;
                                self.auto_pick_silent();
                                self.history.clear();
                                self.status = format!(
                                    "已打开 {}（{} 点，处理耗时 {:.0} ms）",
                                    loaded.source.vendor.label(),
                                    spec_len(&self.docs[0]),
                                    out.report.elapsed_ms
                                );
                                for note in loaded.notes {
                                    self.status.push_str(&format!(" · {note}"));
                                }
                            }
                            Err(e) => self.fail(e),
                        }
                    }
                    LoadedData::Spectrum(spec) => {
                        let view = ViewRange::new(spec.x.start_ppm, spec.x.end_ppm);
                        let mut doc = self.make_doc(&title, &spec, view);
                        doc.path = Some(PathBuf::from(path));
                        let label = format!(
                            "{}（已处理谱，无 FID）",
                            loaded.source.vendor.label()
                        );
                        doc.page = build_page(view, &spec, &label);
                        self.docs = vec![doc];
                        self.active = 0;
                        self.auto_pick_silent();
                        self.history.clear();
                        self.status = format!(
                            "已打开 {}（{} 点，无 FID：处理链为空）",
                            loaded.source.vendor.label(),
                            spec.y.len()
                        );
                        for note in loaded.notes {
                            self.status.push_str(&format!(" · {note}"));
                        }
                    }
                }
                self.selected_item = None;
                self.selected_peak = None;
            }
            Err(e) => self.fail(e),
        }
    }

    fn make_doc(&self, title: &str, spec: &Spectrum1D, view: ViewRange) -> Document {
        let mut doc = Document::empty(title);
        doc.datasets.push(Dataset::new("1D", Arc::new(spec.clone())));
        doc.page = build_page(view, spec, title);
        doc
    }

    // ---------- 处理 ----------

    /// 按当前处理链重新处理（若存在 FID）。
    pub fn reprocess(&mut self, label: &str) {
        let Some(fid) = self.doc().and_then(|d| d.fid.clone()) else {
            self.status = "该数据没有原始 FID，无法重新处理（仅可做显示调整）".into();
            return;
        };
        let steps = match self.doc() {
            Some(d) => d.recipe.steps.clone(),
            None => return,
        };
        let before = match self.doc().cloned() {
            Some(d) => d,
            None => return,
        };
        match mnv_proc::process(&fid, &steps) {
            Ok(out) => {
                let mut warnings = out.report.warnings.clone();
                // 把自动相位解出的角度写回处理链，保证界面显示“实际用了什么”
                let resolved = out.report.applied.iter().find_map(|s| match s {
                    ProcOp::Phase { ph0_deg, ph1_deg, .. } => Some((*ph0_deg, *ph1_deg)),
                    _ => None,
                });
                if let Some((p0, p1)) = resolved {
                    if let Some(doc) = self.doc_mut() {
                        for s in doc.recipe.steps.iter_mut() {
                            if let ProcOp::Phase { ph0_deg, ph1_deg, .. } = s {
                                *ph0_deg = p0;
                                *ph1_deg = p1;
                            }
                        }
                    }
                }
                let old_view = self.view();
                let new_spec = Arc::new(out.spectrum);
                if let Some(doc) = self.doc_mut() {
                    if let Some(ds) = doc.datasets.first_mut() {
                        ds.spectrum = new_spec.clone();
                    }
                }
                // 视图保持在数据范围内
                let data = ViewRange::new(new_spec.x.start_ppm, new_spec.x.end_ppm);
                if let Some(v) = old_view {
                    let v2 = v.clamped_to(data, 0.05);
                    self.set_view(v2);
                }
                self.push_undo(label, before);
                self.proc_dirty = false;
                self.status = format!(
                    "{label} · {} 点 · {:.1} ms",
                    new_spec.y.len(),
                    out.report.elapsed_ms
                );
                for w in warnings.drain(..) {
                    self.status.push_str(&format!(" · {w}"));
                }
            }
            Err(e) => self.fail(e),
        }
    }

    /// 就地修改处理链中的某个步骤。
    pub fn update_step<F: Fn(&mut ProcOp)>(&mut self, pred: fn(&ProcOp) -> bool, edit: F) {
        if let Some(doc) = self.doc_mut() {
            if let Some(step) = doc.recipe.steps.iter_mut().find(|s| pred(s)) {
                edit(step);
                self.proc_dirty = true;
            }
        }
    }

    pub fn step(&self, pred: fn(&ProcOp) -> bool) -> Option<&ProcOp> {
        self.doc()?.recipe.steps.iter().find(|s| pred(s))
    }

    pub fn toggle_baseline(&mut self) {
        if self.doc().is_none() {
            return;
        }
        let mut now_off = false;
        if let Some(doc) = self.doc_mut() {
            for s in doc.recipe.steps.iter_mut() {
                if let ProcOp::Baseline { kind, .. } = s {
                    if *kind == mnv_core::BaselineKind::None {
                        *kind = mnv_core::BaselineKind::Whittaker;
                    } else {
                        *kind = mnv_core::BaselineKind::None;
                        now_off = true;
                    }
                }
            }
        }
        self.reprocess(if now_off { "关闭基线校正" } else { "基线校正" });
    }

    pub fn set_phase(&mut self, ph0: f64, ph1: f64, automatic: bool) {
        self.update_step(
            |s| matches!(s, ProcOp::Phase { .. }),
            |s| {
                if let ProcOp::Phase {
                    ph0_deg,
                    ph1_deg,
                    automatic: a,
                } = s
                {
                    *ph0_deg = ph0;
                    *ph1_deg = ph1;
                    *a = automatic;
                }
            },
        );
    }

    pub fn auto_phase_now(&mut self) {
        self.set_phase(0.0, 0.0, true);
        self.reprocess("自动相位");
    }

    // ---------- 分析 ----------

    /// 载入后给一版“粗筛”结果：标峰 → 自动分组多重峰 → 按面积归一积分。
    /// 全部可被用户修改或撤销（与教程里“自动只作粗筛”的做法一致）。
    fn auto_pick_silent(&mut self) {
        let Some(spec) = self.spectrum() else { return };
        let peaks = pick(&spec, &self.pick_params);
        let multiplets = mnv_analysis::multiplets_from_auto(&spec, &peaks, &self.multiplet_params);
        let sw = spec.meta.spectrometer_mhz.max(1e-9);
        let mut integrals: Vec<IntegralRegion> = multiplets
            .iter()
            .map(|m| {
                let widest = m
                    .peaks
                    .iter()
                    .filter_map(|i| peaks.get(*i).map(|p| p.width_hz))
                    .fold(1.0f32, f32::max);
                let pad = (widest as f64 * 8.0 / sw)
                    .max((m.from_ppm - m.to_ppm).abs() * 0.35)
                    .max(0.03);
                IntegralRegion::new(
                    m.from_ppm + pad,
                    m.to_ppm - pad,
                    region_sum(&spec, m.from_ppm + pad, m.to_ppm - pad),
                )
            })
            .collect();
        mnv_analysis::assign_by_area(&mut integrals, &[1.0, 2.0, 3.0, 4.0]);
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                ds.analysis.peaks = peaks;
                ds.analysis.multiplets = multiplets;
                ds.analysis.integrals = integrals;
            }
        }
    }

    pub fn pick_peaks(&mut self) {
        let Some(spec) = self.spectrum() else { return };
        let before = match self.doc().cloned() {
            Some(d) => d,
            None => return,
        };
        let params = self.pick_params.clone();
        let peaks = pick(&spec, &params);
        let n = peaks.len();
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                ds.analysis.peaks = peaks;
                ds.analysis.multiplets.clear();
            }
        }
        self.push_undo("标峰", before);
        self.toast(format!("检出 {n} 个峰（阈值 {:.2}%）", params.threshold_rel * 100.0));
    }

    pub fn clear_peaks(&mut self) {
        let Some(before) = self.doc().cloned() else { return };
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                ds.analysis.peaks.clear();
                ds.analysis.multiplets.clear();
            }
        }
        self.push_undo("清除标峰", before);
        self.toast("已清除标峰");
    }

    /// 在给定 ppm 附近吸附到最近的局部极大，添加一个手动峰。
    pub fn add_peak_near(&mut self, ppm: f64) {
        let Some(spec) = self.spectrum() else { return };
        let before = match self.doc().cloned() {
            Some(d) => d,
            None => return,
        };
        let (a, b) = spec.x.index_range(ppm + 0.02, ppm - 0.02);
        let (mut best_i, mut best_v) = (a, f32::MIN);
        for i in a..=b.min(spec.y.len() - 1) {
            if spec.y[i] > best_v {
                best_v = spec.y[i];
                best_i = i;
            }
        }
        let mut peak = mnv_core::Peak::new(spec.x.ppm_at(best_i), best_v);
        peak.source = mnv_core::PeakSource::Manual;
        if let Some(name) = mnv_core::solvent::match_residual(
            &spec.meta.nucleus,
            peak.ppm,
            self.pick_params.solvent_tolerance_ppm,
        ) {
            peak.kind = mnv_core::PeakKind::Solvent;
            peak.note = Some(name.to_string());
        }
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                ds.analysis.peaks.push(peak);
                ds.analysis
                    .peaks
                    .sort_by(|p, q| q.ppm.partial_cmp(&p.ppm).unwrap_or(std::cmp::Ordering::Equal));
            }
        }
        self.push_undo("手动标峰", before);
    }

    pub fn add_integral(&mut self, a: f64, b: f64) {
        let Some(spec) = self.spectrum() else { return };
        let (hi, lo) = (a.max(b), a.min(b));
        if (hi - lo) < spec.x.ppm_per_point() * 2.0 {
            self.toast("积分区间太窄");
            return;
        }
        let before = match self.doc().cloned() {
            Some(d) => d,
            None => return,
        };
        let raw = region_sum(&spec, hi, lo);
        let mut region = IntegralRegion::new(hi, lo, raw);
        region.value = self.integrate_target_h;
        let value = region.value;
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                ds.analysis.integrals.push(region);
                ds.analysis.integrals.sort_by(|x, y| {
                    y.from_ppm
                        .partial_cmp(&x.from_ppm)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }
        self.push_undo("积分", before);
        self.toast(format!("新增积分区间 {hi:.3}–{lo:.3} ppm（{value:.1}H）"));
    }

    pub fn analyze_multiplet(&mut self, a: f64, b: f64) {
        let Some(spec) = self.spectrum() else { return };
        let (hi, lo) = (a.max(b), a.min(b));
        let before = match self.doc().cloned() {
            Some(d) => d,
            None => return,
        };
        let params = self.multiplet_params.clone();
        let peaks = self
            .doc()
            .and_then(|d| d.datasets.first())
            .map(|d| d.analysis.peaks.clone())
            .unwrap_or_default();
        let (peaks, note) = if peaks.is_empty() {
            let mut p = self.pick_params.clone();
            p.region = Some((hi, lo));
            (pick(&spec, &p), "（已在该区间自动标峰）")
        } else {
            (peaks, "")
        };
        let target_h = self.integrate_target_h;
        let found = analyze_region(&spec, &peaks, hi, lo, &params);
        if found.is_empty() {
            self.toast("该区间内没有可用峰：请先标峰，或调整阈值");
            return;
        }
        let m = found[0].clone();
        let raw = region_sum(&spec, m.from_ppm, m.to_ppm);
        let summary = format!(
            "{} · {} 峰 · J = {} Hz · 残差 {:.2} Hz",
            m.label.symbol(),
            m.peaks.len(),
            m.j_text(1).unwrap_or_else(|| "—".into()),
            m.fit_residual_hz
        );
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                if ds.analysis.peaks.is_empty() {
                    ds.analysis.peaks = peaks.clone();
                }
                let widest_hz = m
                    .peaks
                    .iter()
                    .filter_map(|i| m_peak_width(&peaks, *i))
                    .fold(1.0f32, f32::max);
                let sw = spec.meta.spectrometer_mhz.max(1e-9);
                let pad = (widest_hz as f64 * 8.0 / sw)
                    .max((m.from_ppm - m.to_ppm).abs() * 0.35)
                    .max(0.03);
                let (from_ppm, to_ppm) = (m.from_ppm + pad, m.to_ppm - pad);
                let mut m2 = m;
                m2.integral = raw;
                m2.integral_value = target_h;
                ds.analysis.multiplets.push(m2);
                // 同步建立一个积分区间：报告里的 H 数就来自积分
                let exists = ds.analysis.integrals.iter().any(|r| {
                    (r.from_ppm - from_ppm).abs() < 0.05 && (r.to_ppm - to_ppm).abs() < 0.05
                });
                if !exists {
                    let mut region = IntegralRegion::new(from_ppm, to_ppm, raw);
                    region.value = target_h;
                    ds.analysis.integrals.push(region);
                }
            }
        }
        self.push_undo("多重峰分析", before);
        self.toast(format!("多重峰：{summary}{note}"));
    }

    pub fn calibrate_at(&mut self, clicked_ppm: f64) {
        let target = self.ref_target_ppm;
        let delta = target - clicked_ppm;
        if let Some(doc) = self.doc_mut() {
            let has = doc
                .recipe
                .steps
                .iter()
                .any(|s| matches!(s, ProcOp::Calibrate { .. }));
            if has {
                for s in doc.recipe.steps.iter_mut() {
                    if let ProcOp::Calibrate {
                        reference_ppm,
                        target_ppm,
                        delta_ppm,
                    } = s
                    {
                        *reference_ppm = None;
                        *target_ppm = target;
                        *delta_ppm = delta;
                    }
                }
            } else {
                doc.recipe.steps.push(ProcOp::Calibrate {
                    reference_ppm: None,
                    target_ppm: target,
                    delta_ppm: delta,
                });
            }
            // 定标后原有的峰/积分位置已失效
            for ds in doc.datasets.iter_mut() {
                ds.analysis.clear();
            }
        }
        self.reprocess(&format!("定标 Δ = {delta:+.3} ppm"));
    }

    /// 全谱自动分组多重峰（“先给个初步结果”，再让用户修正）。
    pub fn analyze_all_multiplets(&mut self) {
        let Some(spec) = self.spectrum() else { return };
        let before = match self.doc().cloned() {
            Some(d) => d,
            None => return,
        };
        let mut peaks = self
            .doc()
            .and_then(|d| d.datasets.first())
            .map(|d| d.analysis.peaks.clone())
            .unwrap_or_default();
        if peaks.is_empty() {
            peaks = pick(&spec, &self.pick_params);
        }
        let ms = mnv_analysis::multiplets_from_auto(&spec, &peaks, &self.multiplet_params);
        let n = ms.len();
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                ds.analysis.peaks = peaks;
                ds.analysis.multiplets = ms;
            }
        }
        self.push_undo("全谱多重峰", before);
        self.toast(format!("自动分组出 {n} 组多重峰"));
    }

    /// 按积分面积把氢数凑成整数比（“先积分、再凑整”的标准流程）。
    pub fn normalize_integrals(&mut self) {
        let Some(spec) = self.spectrum() else { return };
        let Some(before) = self.doc().cloned() else { return };
        let multiplets = self
            .doc()
            .and_then(|d| d.datasets.first())
            .map(|d| d.analysis.multiplets.clone())
            .unwrap_or_default();
        let mut scale = 1.0f32;
        let mut count = 0usize;
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                if ds.analysis.integrals.is_empty() {
                    for m in &multiplets {
                        let raw = region_sum(&spec, m.from_ppm, m.to_ppm);
                        ds.analysis
                            .integrals
                            .push(IntegralRegion::new(m.from_ppm, m.to_ppm, raw));
                    }
                }
                count = ds.analysis.integrals.len();
                if count > 0 {
                    scale = mnv_analysis::assign_by_area(
                        &mut ds.analysis.integrals,
                        &[1.0, 2.0, 3.0, 4.0],
                    );
                }
            }
        }
        self.push_undo("积分归一", before);
        self.toast(if count == 0 {
            "没有可归一的积分区间".to_string()
        } else {
            format!("已按面积归一 {count} 个区间（缩放因子 {scale:.0}）")
        });
    }

    pub fn set_integral_value(&mut self, index: usize, value: f32) {
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                if let Some(r) = ds.analysis.integrals.get_mut(index) {
                    r.value = value;
                    r.edited = true;
                }
            }
        }
    }

    pub fn remove_integral(&mut self, index: usize) {
        let Some(before) = self.doc().cloned() else { return };
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                if index < ds.analysis.integrals.len() {
                    ds.analysis.integrals.remove(index);
                }
            }
        }
        self.push_undo("删除积分", before);
    }

    /// 在当前视图中心插入一个局部放大图（inset）。
    pub fn insert_inset(&mut self) {
        let Some(view) = self.view() else { return };
        let Some(before) = self.doc().cloned() else { return };
        let span = view.span() * 0.35;
        let center = view.center();
        let src = ViewRange::new(center + span * 0.5, center - span * 0.5);
        let mut item = mnv_core::InsetItem::new(
            0,
            Rect01::new(0.52, 0.44, 0.40, 0.20),
            src,
            src,
        );
        item.style.show_axis = true;
        if let Some(doc) = self.doc_mut() {
            doc.page.items.push(mnv_core::CanvasItem::Inset(item));
        }
        self.push_undo("插入局部放大", before);
        self.toast("已插入局部放大图（可拖动调整位置）");
    }

    pub fn set_style(&mut self, index: usize, style: mnv_core::SpectrumStyle) {
        if let Some(doc) = self.doc_mut() {
            if let Some(it) = doc.page.items.get_mut(index) {
                if let Some(s) = it.style_mut() {
                    *s = style;
                }
            }
        }
    }

    // ---------- 撤销 ----------

    pub fn undo(&mut self) {
        if let Some(doc) = self.docs.get_mut(self.active) {
            if let Some(label) = self.history.undo(doc) {
                self.toast(format!("已撤销：{label}"));
            } else {
                self.toast("没有可撤销的操作");
            }
        }
    }

    pub fn redo(&mut self) {
        if let Some(doc) = self.docs.get_mut(self.active) {
            if let Some(label) = self.history.redo(doc) {
                self.toast(format!("已重做：{label}"));
            } else {
                self.toast("没有可重做的操作");
            }
        }
    }

    // ---------- 报告 ----------

    pub fn build_report(&mut self) {
        let Some(doc) = self.doc() else { return };
        let Some(ds) = doc.datasets.first() else { return };
        let text = mnv_report::si_block(&ds.spectrum, &ds.analysis, self.journal);
        // 同时把数据行写回页面，实现“分析 → 出图 → 文本”闭环
        let line = mnv_report::data_line(&ds.analysis, self.journal);
        let mut updated = false;
        if let Some(doc) = self.doc_mut() {
            for item in doc.page.items.iter_mut() {
                if let CanvasItem::Text(t) = item {
                    if t.text.starts_with("峰表") || t.text.starts_with("δ ") {
                        t.text = line.clone();
                        updated = true;
                    }
                }
            }
        }
        self.report_text = text;
        self.show_report = true;
        self.toast(if updated {
            "报告已生成，数据行已写入页面"
        } else {
            "报告已生成"
        });
    }

    pub fn recipe_json(&self) -> String {
        self.doc()
            .map(|d| d.recipe.to_json())
            .unwrap_or_else(|| "{}".into())
    }

    // ---------- 快捷键 ----------

    pub fn handle_keys(&mut self, ctx: &egui::Context) {
        use egui::Key;
        // 正在输入文本时（路径框/报告框）不响应单键快捷键
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (undo, redo) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(Key::Z),
                i.modifiers.command && (i.key_pressed(Key::Y) || (i.modifiers.shift && i.key_pressed(Key::Z))),
            )
        });
        if undo {
            self.undo();
            return;
        }
        if redo {
            self.redo();
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.drag = Drag::None;
            self.tool = Tool::Select;
        }
        if ctx.input(|i| i.key_pressed(Key::F)) {
            if let Some(r) = self.data_range() {
                self.set_view(ViewRange::new(
                    r.from_ppm + r.span() * 0.01,
                    r.to_ppm - r.span() * 0.01,
                ));
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Z)) {
            self.tool = Tool::Zoom;
        }
        if ctx.input(|i| i.key_pressed(Key::K)) {
            self.tool = Tool::Peak;
            self.pick_peaks();
        }
        if ctx.input(|i| i.key_pressed(Key::I)) {
            self.tool = Tool::Integral;
        }
        if ctx.input(|i| i.key_pressed(Key::J)) {
            self.tool = Tool::Multiplet;
        }
        if ctx.input(|i| i.key_pressed(Key::L)) {
            self.tool = Tool::Reference;
        }
        if ctx.input(|i| i.key_pressed(Key::B)) {
            self.toggle_baseline();
        }
        if ctx.input(|i| i.modifiers.shift && i.key_pressed(Key::P)) {
            self.auto_phase_now();
        }
        if ctx.input(|i| i.key_pressed(Key::Delete)) {
            self.delete_selected();
        }
    }

    pub fn delete_selected(&mut self) {
        let Some(before) = self.doc().cloned() else { return };
        let sel = self.selected_peak;
        let mut removed = false;
        if let Some(doc) = self.doc_mut() {
            if let Some(ds) = doc.datasets.first_mut() {
                if let Some(p) = sel {
                    if p < ds.analysis.peaks.len() {
                        ds.analysis.peaks.remove(p);
                        ds.analysis.multiplets.clear();
                        removed = true;
                    }
                }
            }
        }
        self.selected_peak = None;
        if removed {
            self.push_undo("删除峰", before);
            self.toast("已删除选中峰");
        }
    }
}

fn m_peak_width(peaks: &[mnv_core::Peak], idx: usize) -> Option<f32> {
    peaks.get(idx).map(|p| p.width_hz)
}

fn spec_len(doc: &Document) -> usize {
    doc.datasets.first().map(|d| d.spectrum.y.len()).unwrap_or(0)
}

fn short_title(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string())
}

/// 建立标准一维版式：标题 + 谱图。
pub fn build_page(view: ViewRange, spec: &Spectrum1D, label: &str) -> mnv_core::Page {
    let mut page = mnv_core::Page::a4_1d();
    if let Some(item) = page.primary_item_mut() {
        item.view = view;
        item.style = mnv_core::SpectrumStyle::default();
    }
    if let Some(CanvasItem::Text(TextItem { text, .. })) = page.items.first_mut() {
        *text = label.to_string();
    }
    // 底部注释：采集信息
    page.items.push(CanvasItem::Text(TextItem::new(
        Rect01::new(0.06, 0.86, 0.88, 0.04),
        &meta_line(&spec.meta),
        8.0,
    )));
    page
}

fn meta_line(meta: &AcquisitionMeta) -> String {
    let solvent = meta.solvent.clone().unwrap_or_else(|| "—".into());
    let pulse = meta.pulse_program.clone().unwrap_or_else(|| "—".into());
    format!(
        "{}  ·  {:.2} MHz  ·  SW {:.0} Hz  ·  溶剂 {}  ·  {}",
        meta.nucleus.display(),
        meta.spectrometer_mhz,
        meta.spectral_width_hz,
        solvent,
        pulse
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 无头验证「载入即出结果」：不依赖窗口与 GPU。
    #[test]
    fn demo_loads_with_initial_analysis() {
        let app = App::build(true);
        let doc = app.doc().expect("应有一个文档");
        let ds = doc.datasets.first().expect("应有一个数据集");

        assert!(ds.spectrum.y.len() > 10_000);
        assert!(ds.analysis.peaks.len() >= 5, "峰数 {}", ds.analysis.peaks.len());
        assert!(
            !ds.analysis.multiplets.is_empty(),
            "应自动分组出多重峰"
        );
        assert!(
            !ds.analysis.integrals.is_empty(),
            "应自动生成积分区间"
        );

        // 乙酸乙酯：一个 q（2H）、一个 s（3H）、一个 t（3H）
        let labels: Vec<&str> = ds
            .analysis
            .multiplets
            .iter()
            .map(|m| m.label.symbol())
            .collect();
        assert!(labels.contains(&"q"), "labels = {labels:?}");
        assert!(labels.contains(&"t"), "labels = {labels:?}");

        let total_h: f32 = ds
            .analysis
            .integrals
            .iter()
            .map(|r| r.value)
            .filter(|v| *v >= 1.0)
            .sum();
        assert!(total_h >= 6.0, "主信号氢数合计 {total_h}");

        // 处理链可复现：JSON 往返
        let json = doc.recipe.to_json();
        let back = ProcRecipe::from_json(&json).expect("处理链应可往返");
        assert_eq!(back.steps.len(), doc.recipe.steps.len());
    }

    #[test]
    fn undo_restores_document() {
        let mut app = App::build(true);
        let before_peaks = app.doc().unwrap().datasets[0].analysis.peaks.len();
        app.clear_peaks();
        assert_eq!(app.doc().unwrap().datasets[0].analysis.peaks.len(), 0);
        app.undo();
        assert_eq!(
            app.doc().unwrap().datasets[0].analysis.peaks.len(),
            before_peaks
        );
    }
}
