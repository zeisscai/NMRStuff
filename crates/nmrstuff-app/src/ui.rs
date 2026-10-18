//! 界面层：面板、页面画布、交互与窗口。
//!
//! 只在 `nmrstuff-app` 里出现 egui 依赖；算法与几何都在下层 crate，便于将来换壳
//! （例如换成 `wgpu` 自绘或导出 SVG）而不动业务代码。

use eframe::egui;
use nmrstuff_core::{
    ApodKind, BaselineKind, CanvasItem, Document, ProcOp, Rect01, SpectrumStyle, ViewRange,
};
use nmrstuff_render::{build_trace, layout_peak_labels, nice_ticks, tick_text, ScreenRect, Transform};

use crate::app::{App, Drag, Tool};
use crate::theme;

// ---------------------------------------------------------------- 顶部工具栏

pub fn top_bar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::top("top").show(ui, |ui| {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("NMRStuff").strong().size(16.0).color(theme::ACCENT));
            ui.label(
                egui::RichText::new("轻量 NMR 工作站 · demo")
                    .small()
                    .color(theme::MUTED),
            );
            ui.separator();

            if ui.button("演示数据").on_hover_text("载入内置乙酸乙酯 ¹H 谱").clicked() {
                app.load_demo();
            }
            if ui.button("打开…").on_hover_text("读取左侧浏览器选中的路径").clicked() {
                let p = app.path_input.clone();
                app.open_path(&p);
            }
            ui.separator();

            for (tool, label) in [
                (Tool::Select, "选择"),
                (Tool::Zoom, "放大 Z"),
                (Tool::Peak, "标峰 K"),
                (Tool::Integral, "积分 I"),
                (Tool::Multiplet, "多重峰 J"),
                (Tool::Reference, "定标 L"),
            ] {
                if ui
                    .selectable_label(app.tool == tool, label)
                    .on_hover_text(tool.hint())
                    .clicked()
                {
                    app.tool = tool;
                    if tool == Tool::Peak && app.doc().is_some() {
                        // 点击标峰工具时先给出自动粗筛，符合“自动仅作粗筛”的实践
                        app.pick_peaks();
                    }
                }
            }
            ui.separator();
            if ui
                .add_enabled(app.history.can_undo(), egui::Button::new("撤销"))
                .clicked()
            {
                app.undo();
            }
            if ui
                .add_enabled(app.history.can_redo(), egui::Button::new("重做"))
                .clicked()
            {
                app.redo();
            }
            ui.separator();
            if ui.button("报告").clicked() {
                app.build_report();
            }
            if ui.button("处理链").clicked() {
                app.show_recipe = true;
            }
            if ui.button("帮助").clicked() {
                app.show_help = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !app.fonts_ok {
                    ui.colored_label(theme::WARN, "未找到中文字体，界面可能显示方块");
                }
                ui.label(
                    egui::RichText::new(format!("{:.1} ms/帧", app.frame_ms))
                        .small()
                        .color(theme::MUTED),
                );
            });
        });
        ui.add_space(3.0);
    });
}

// ---------------------------------------------------------------- 左侧面板

pub fn left_panel(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::left("left")
        .resizable(true)
        .default_size(258.0)
        .size_range(200.0..=420.0)
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("文档").strong());
            ui.add_space(2.0);
            documents_tree(app, ui);

            ui.add_space(8.0);
            ui.separator();
            ui.label(egui::RichText::new("文件浏览器").strong());
            ui.add_space(2.0);
            browser(app, ui);
        });
}

fn documents_tree(app: &mut App, ui: &mut egui::Ui) {
    let Some(doc) = app.doc().cloned() else {
        ui.colored_label(theme::MUTED, "（没有打开的文档）");
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt("doc_tree")
        .max_height(220.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&doc.title).strong());
                if doc.dirty {
                    ui.colored_label(theme::WARN, "•");
                }
            });
            ui.label(
                egui::RichText::new(format!(
                    "{} · {}",
                    doc.datasets.len(),
                    doc.instrument_text()
                ))
                .small()
                .color(theme::MUTED),
            );

            for (di, ds) in doc.datasets.iter().enumerate() {
                ui.push_id(("ds", di), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("▸ 数据集");
                        ui.label(
                            egui::RichText::new(format!(
                                "{} 点 · {}",
                                ds.spectrum.y.len(),
                                ds.spectrum.meta.nucleus.display()
                            ))
                            .small()
                            .color(theme::MUTED),
                        );
                    });
                    ui.indent(("analysis", di), |ui| {
                        let a = &ds.analysis;
                        ui.label(
                            egui::RichText::new(format!(
                                "峰 {} · 积分 {} · 多重峰 {}",
                                a.peaks.len(),
                                a.integrals.len(),
                                a.multiplets.len()
                            ))
                            .small()
                            .color(theme::MUTED),
                        );
                    });
                });
            }

            ui.add_space(4.0);
            ui.label(egui::RichText::new("页面对象").strong().small());
            for (i, item) in doc.page.items.iter().enumerate() {
                let selected = app.selected_item == Some(i);
                let label = match item {
                    CanvasItem::Spectrum(s) => {
                        format!("谱图  {:.2}–{:.2} ppm", s.view.from_ppm, s.view.to_ppm)
                    }
                    CanvasItem::Inset(s) => {
                        format!("局部放大  {:.2}–{:.2} ppm", s.view.from_ppm, s.view.to_ppm)
                    }
                    CanvasItem::Text(t) => {
                        let txt: String = t.text.chars().take(18).collect();
                        format!("文本  {txt}")
                    }
                };
                if ui.selectable_label(selected, label).clicked() {
                    app.selected_item = Some(i);
                }
            }
        });
}

fn browser(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.small_button("↑").on_hover_text("上级目录").clicked() {
            if let Some(p) = app.browser_dir.parent() {
                app.browser_dir = p.to_path_buf();
                app.path_input = app.browser_dir.to_string_lossy().to_string();
            }
        }
        let resp = ui.add(
            egui::TextEdit::singleline(&mut app.path_input)
                .desired_width(f32::INFINITY)
                .hint_text("数据集目录或文件路径"),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let p = app.path_input.clone();
            app.browser_dir = std::path::PathBuf::from(&p);
            app.open_path(&p);
        }
    });
    ui.horizontal(|ui| {
        if ui.small_button("载入").clicked() {
            let p = app.path_input.clone();
            app.open_path(&p);
        }
        ui.label(
            egui::RichText::new("支持 Bruker 目录 / .jdx / .csv")
                .small()
                .color(theme::MUTED),
        );
    });

    let dir = app.browser_dir.clone();
    let mut entries: Vec<(String, bool)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            entries.push((name, is_dir));
        }
    }
    entries.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.0.to_lowercase().cmp(&b.0.to_lowercase()),
    });

    egui::ScrollArea::vertical()
        .id_salt("browser")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if entries.is_empty() {
                ui.colored_label(theme::MUTED, "（空目录或无法读取）");
            }
            for (name, is_dir) in entries {
                let label = if is_dir {
                    format!("📁 {name}")
                } else {
                    format!("·  {name}")
                };
                let resp = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
                if resp.clicked() {
                    let p = dir.join(&name);
                    app.path_input = p.to_string_lossy().to_string();
                    if is_dir {
                        app.browser_dir = p;
                    } else {
                        app.open_path(&p.to_string_lossy());
                    }
                }
            }
        });
}

// ---------------------------------------------------------------- 右侧面板

pub fn right_panel(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::right("right")
        .resizable(true)
        .default_size(320.0)
        .size_range(260.0..=520.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(4.0);
                processing_section(app, ui);
                ui.add_space(6.0);
                ui.separator();
                analysis_section(app, ui);
                ui.add_space(6.0);
                ui.separator();
                view_section(app, ui);
            });
        });
}

fn processing_section(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new("处理").strong());
    let has_fid = app.doc().map(|d| d.fid.is_some()).unwrap_or(false);
    if !has_fid {
        ui.colored_label(
            theme::MUTED,
            "该数据已处理完成（无 FID），处理参数不可用",
        );
    }
    ui.add_enabled_ui(has_fid, |ui| {
        // 窗函数
        let (mut kind, mut lb, mut gb) = match app.step(|s| matches!(s, ProcOp::Apodization { .. })) {
            Some(ProcOp::Apodization { kind, lb_hz, gb_hz, .. }) => (*kind, *lb_hz, *gb_hz),
            _ => (ApodKind::None, 0.3, 0.0),
        };
        egui::ComboBox::from_id_salt("apod")
            .selected_text(kind.label())
            .width(180.0)
            .show_ui(ui, |ui| {
                for k in ApodKind::ALL {
                    ui.selectable_value(&mut kind, k, k.label());
                }
            });
        let mut changed = false;
        changed |= ui
            .add(egui::Slider::new(&mut lb, -2.0..=10.0).text("LB (Hz)").fixed_decimals(2))
            .changed();
        changed |= ui
            .add(egui::Slider::new(&mut gb, 0.0..=0.5).text("GB (Hz)").fixed_decimals(3))
            .changed();
        if changed {
            app.update_step(
                |s| matches!(s, ProcOp::Apodization { .. }),
                |s| {
                    if let ProcOp::Apodization {
                        kind: k,
                        lb_hz,
                        gb_hz,
                        ..
                    } = s
                    {
                        *k = kind;
                        *lb_hz = lb;
                        *gb_hz = gb;
                    }
                },
            );
        }

        // 零填充
        let mut factor = match app.step(|s| matches!(s, ProcOp::ZeroFill { .. })) {
            Some(ProcOp::ZeroFill { factor }) => *factor,
            _ => 2,
        };
        if ui
            .add(egui::Slider::new(&mut factor, 1..=8).text("零填充倍数"))
            .changed()
        {
            app.update_step(
                |s| matches!(s, ProcOp::ZeroFill { .. }),
                |s| {
                    if let ProcOp::ZeroFill { factor: f } = s {
                        *f = factor;
                    }
                },
            );
        }

        // 相位
        ui.add_space(4.0);
        ui.label(egui::RichText::new("相位").small().strong());
        let (mut p0, mut p1, mut auto) = match app.step(|s| matches!(s, ProcOp::Phase { .. })) {
            Some(ProcOp::Phase { ph0_deg, ph1_deg, automatic }) => {
                (*ph0_deg, *ph1_deg, *automatic)
            }
            _ => (0.0, 0.0, false),
        };
        let r0 = ui.add(
            egui::Slider::new(&mut p0, -180.0..=180.0)
                .text("φ0 (°)")
                .fixed_decimals(1),
        );
        let r1 = ui.add(
            egui::Slider::new(&mut p1, -720.0..=720.0)
                .text("φ1 (°)")
                .fixed_decimals(1),
        );
        if r0.drag_stopped() || r1.drag_stopped() || (r0.changed() && !r0.dragged() && !r1.dragged()) {
            app.set_phase(p0, p1, false);
            let before_drag = r0.drag_stopped() || r1.drag_stopped();
            if before_drag {
                app.reprocess("手动相位");
            }
        }
        ui.horizontal(|ui| {
            if ui.button("自动相位").clicked() {
                app.auto_phase_now();
            }
            let mut is_auto = auto;
            if ui.checkbox(&mut is_auto, "自动").changed() {
                auto = is_auto;
                app.set_phase(p0, p1, auto);
                app.reprocess("相位模式");
            }
        });

        // 基线
        ui.add_space(4.0);
        let (mut kind, mut lambda) = match app.step(|s| matches!(s, ProcOp::Baseline { .. })) {
            Some(ProcOp::Baseline { kind, lambda, .. }) => (*kind, *lambda),
            _ => (BaselineKind::None, 1e5),
        };
        let mut changed = false;
        egui::ComboBox::from_id_salt("baseline")
            .selected_text(kind.label())
            .width(180.0)
            .show_ui(ui, |ui| {
                for k in BaselineKind::ALL {
                    changed |= ui.selectable_value(&mut kind, k, k.label()).changed();
                }
            });
        let lr = ui.add(
            egui::Slider::new(&mut lambda, 1e2..=1e8)
                .logarithmic(true)
                .text("λ"),
        );
        changed |= lr.changed() && !lr.dragged();
        if changed {
            app.update_step(
                |s| matches!(s, ProcOp::Baseline { .. }),
                |s| {
                    if let ProcOp::Baseline {
                        kind: k, lambda: l, ..
                    } = s
                    {
                        *k = kind;
                        *l = lambda;
                    }
                },
            );
        }
        if lr.drag_stopped() {
            app.reprocess("基线参数");
        }

        ui.add_space(5.0);
        let dirty = app.proc_dirty;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(dirty, egui::Button::new("应用处理参数"))
                .clicked()
            {
                app.reprocess("应用处理参数");
            }
            if dirty {
                ui.colored_label(theme::WARN, "有未应用的改动");
            }
        });
    });
}

fn analysis_section(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new("分析").strong());
    ui.horizontal(|ui| {
        let mut thr = app.pick_params.threshold_rel * 100.0;
        if ui
            .add(
                egui::Slider::new(&mut thr, 0.05..=5.0)
                    .text("阈值 %")
                    .fixed_decimals(2),
            )
            .changed()
        {
            app.pick_params.threshold_rel = thr / 100.0;
        }
    });
    ui.horizontal(|ui| {
        if ui.button("自动标峰").clicked() {
            app.pick_peaks();
        }
        if ui.button("清除").clicked() {
            app.clear_peaks();
        }
        if ui.button("全谱多重峰").clicked() {
            app.analyze_all_multiplets();
        }
        if ui
            .button("按面积归一")
            .on_hover_text("先积分，再按最小峰为基准凑成整数比")
            .clicked()
        {
            app.normalize_integrals();
        }
    });

    let Some(doc) = app.doc().cloned() else { return };
    let Some(ds) = doc.datasets.first() else { return };

    ui.add_space(4.0);
    if ds.analysis.integrals.is_empty() {
        ui.label(egui::RichText::new("积分：用 I 工具横向拖出区间").small().color(theme::MUTED));
    } else {
        ui.label(egui::RichText::new("积分区间").small().strong());
        for (i, r) in ds.analysis.integrals.iter().enumerate() {
            ui.push_id(("int", i), |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{:.3}–{:.3}", r.from_ppm, r.to_ppm));
                    let mut v = r.value;
                    if ui
                        .add(egui::DragValue::new(&mut v).speed(0.1).range(0.0..=1000.0).suffix(" H"))
                        .changed()
                    {
                        app.set_integral_value(i, v);
                    }
                    if ui.small_button("×").clicked() {
                        app.remove_integral(i);
                    }
                });
            });
        }
    }

    ui.add_space(4.0);
    if ds.analysis.multiplets.is_empty() {
        ui.label(
            egui::RichText::new("多重峰：用 J 工具框选一个峰簇")
                .small()
                .color(theme::MUTED),
        );
    } else {
        ui.label(egui::RichText::new("多重峰").small().strong());
        for m in &ds.analysis.multiplets {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(nmrstuff_report::multiplet_line(&ds.analysis, m))
                        .small()
                        .color(theme::TEXT),
                );
                if m.fit_residual_hz > 0.5 {
                    ui.colored_label(theme::WARN, "⚠");
                }
            });
        }
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("journal")
            .selected_text(app.journal.label())
            .width(90.0)
            .show_ui(ui, |ui| {
                for j in [
                    nmrstuff_report::JournalStyle::Jacs,
                    nmrstuff_report::JournalStyle::Angew,
                    nmrstuff_report::JournalStyle::Rsc,
                ] {
                    ui.selectable_value(&mut app.journal, j, j.label());
                }
            });
        if ui.button("生成报告").clicked() {
            app.build_report();
        }
    });
    if !ds.analysis.multiplets.is_empty() {
        let text = nmrstuff_report::data_line(&ds.analysis, app.journal);
        ui.add_space(3.0);
        ui.add(
            egui::Label::new(egui::RichText::new(text).small().monospace())
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
    }
}

fn view_section(app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new("视图与版面").strong());
    if let Some(v) = app.view() {
        let mut from = v.from_ppm;
        let mut to = v.to_ppm;
        ui.horizontal(|ui| {
            ui.label("ppm");
            let r1 = ui.add(egui::DragValue::new(&mut from).speed(0.01).max_decimals(3));
            ui.label("→");
            let r2 = ui.add(egui::DragValue::new(&mut to).speed(0.01).max_decimals(3));
            if r1.changed() || r2.changed() {
                app.set_view(ViewRange::new(from, to));
            }
        });
    }
    ui.horizontal(|ui| {
        if ui.button("全谱 (F)").clicked() {
            if let Some(r) = app.data_range() {
                app.set_view(ViewRange::new(
                    r.from_ppm + r.span() * 0.01,
                    r.to_ppm - r.span() * 0.01,
                ));
            }
        }
        if ui.button("插入局部放大").clicked() {
            app.insert_inset();
        }
    });
    ui.add(
        egui::Slider::new(&mut app.page_zoom, 0.6..=2.5)
            .text("页面缩放")
            .fixed_decimals(2),
    );

    // 样式
    let Some(doc) = app.doc().cloned() else { return };
    let Some(idx) = doc
        .page
        .items
        .iter()
        .position(|i| matches!(i, CanvasItem::Spectrum(_)))
    else {
        return;
    };
    let mut style = doc
        .page
        .items
        .get(idx)
        .and_then(|i| i.style())
        .cloned()
        .unwrap_or_default();
    let mut changed = false;
    changed |= ui
        .add(egui::Slider::new(&mut style.line_width, 0.5..=4.0).text("线宽"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut style.font_pt, 6.0..=20.0).text("字号 (pt)"))
        .changed();
    ui.horizontal(|ui| {
        changed |= ui.checkbox(&mut style.show_peaks, "峰标注").changed();
        changed |= ui.checkbox(&mut style.show_integrals, "积分曲线").changed();
    });
    ui.horizontal(|ui| {
        changed |= ui.checkbox(&mut style.show_multiplets, "多重峰标签").changed();
        changed |= ui.checkbox(&mut style.show_axis, "坐标轴").changed();
    });
    if changed {
        app.set_style(idx, style);
    }
}

// ---------------------------------------------------------------- 底部状态栏

pub fn bottom_bar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::bottom("bottom").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{} · {}", app.tool.label(), app.tool.hint()))
                    .small()
                    .color(theme::MUTED),
            );
            ui.separator();
            match app.cursor {
                Some(c) => {
                    ui.label(
                        egui::RichText::new(format!(
                            "δ {:.3} ppm   {:.1} Hz   强度 {:.3}",
                            c.ppm, c.hz, c.intensity
                        ))
                        .monospace()
                        .small(),
                    );
                }
                None => {
                    ui.label(egui::RichText::new("δ —").monospace().small().color(theme::MUTED));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(&app.status).small().color(theme::MUTED));
            });
        });
    });
}

// ---------------------------------------------------------------- 页面画布

pub fn canvas(app: &mut App, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        let size = ui.available_size();
        let (resp, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());
        painter.rect_filled(resp.rect, 0.0, theme::BG);

        let Some(doc) = app.doc().cloned() else {
            painter.text(
                resp.rect.center(),
                egui::Align2::CENTER_CENTER,
                "没有打开的文档",
                egui::FontId::proportional(15.0),
                theme::MUTED,
            );
            return;
        };

        let page = fit_page(resp.rect, doc.page.aspect(), app.page_zoom);
        painter.rect_filled(
            page.translate(egui::vec2(5.0, 8.0)),
            2.0,
            egui::Color32::from_black_alpha(110),
        );
        painter.rect_filled(page, 1.0, theme::PAPER);
        painter.rect_stroke(
            page,
            1.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(200)),
            egui::StrokeKind::Inside,
        );

        let scale = page.height() / 842.0; // A4 高 ≈ 842 pt

        for (idx, item) in doc.page.items.iter().enumerate() {
            let rect = item_screen_rect(page, item.rect());
            let selected = app.selected_item == Some(idx);
            match item {
                CanvasItem::Spectrum(s) => {
                    if let Some(ds) = doc.datasets.get(s.dataset) {
                        draw_spectrum(
                            &painter,
                            rect,
                            &ds.spectrum,
                            &ds.analysis,
                            s.view,
                            &s.style,
                            scale,
                            selected,
                            false,
                            None,
                        );
                    }
                }
                CanvasItem::Inset(s) => {
                    if let Some(ds) = doc.datasets.get(s.dataset) {
                        draw_spectrum(
                            &painter,
                            rect,
                            &ds.spectrum,
                            &ds.analysis,
                            s.view,
                            &s.style,
                            scale,
                            selected,
                            true,
                            None,
                        );
                        // 母谱上的取景框
                        if s.linked {
                            if let Some(main) = doc.page.primary_item() {
                                let main_rect = item_screen_rect(page, main.rect);
                                let plot = plot_rect(main_rect, scale, &main.style);
                                let t = Transform::new(
                                    to_screen(plot),
                                    main.view,
                                    -1.0,
                                    1.0,
                                );
                                let x0 = t.x_of_ppm(s.source.from_ppm);
                                let x1 = t.x_of_ppm(s.source.to_ppm);
                                let r = egui::Rect::from_min_max(
                                    egui::pos2(x0.min(x1), plot.top()),
                                    egui::pos2(x0.max(x1), plot.bottom()),
                                );
                                painter.rect_stroke(
                                    r,
                                    0.0,
                                    egui::Stroke::new(1.0, theme::ACCENT),
                                    egui::StrokeKind::Outside,
                                );
                            }
                        }
                    }
                }
                CanvasItem::Text(t) => {
                    painter.text(
                        rect.left_top(),
                        egui::Align2::LEFT_TOP,
                        &t.text,
                        egui::FontId::proportional((t.size_pt * scale).max(7.0)),
                        if t.bold {
                            egui::Color32::BLACK
                        } else {
                            egui::Color32::from_rgb(20, 30, 40)
                        },
                    );
                    if selected {
                        painter.rect_stroke(
                            rect,
                            0.0,
                            egui::Stroke::new(1.0, theme::ACCENT),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
            }
        }

        // 交互
        interact(app, ui, &resp, &doc, page, scale);
    });
}

fn fit_page(area: egui::Rect, aspect: f32, zoom: f32) -> egui::Rect {
    let margin = 18.0;
    let avail = area.shrink(margin);
    let mut h = avail.height() * zoom;
    let mut w = h * aspect;
    if w > avail.width() * zoom {
        w = avail.width() * zoom;
        h = w / aspect;
    }
    let center = area.center();
    egui::Rect::from_center_size(center, egui::vec2(w, h))
}

fn item_screen_rect(page: egui::Rect, r: Rect01) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(page.left() + r.x * page.width(), page.top() + r.y * page.height()),
        egui::pos2(
            page.left() + (r.x + r.w) * page.width(),
            page.top() + (r.y + r.h) * page.height(),
        ),
    )
}

fn to_screen(r: egui::Rect) -> ScreenRect {
    ScreenRect::new(r.left(), r.top(), r.width(), r.height())
}

fn plot_rect(item: egui::Rect, scale: f32, style: &SpectrumStyle) -> egui::Rect {
    let bottom = if style.show_axis { 18.0 * scale.max(0.7) } else { 4.0 };
    egui::Rect::from_min_max(
        egui::pos2(item.left() + 8.0 * scale, item.top() + 10.0 * scale),
        egui::pos2(item.right() - 8.0 * scale, item.bottom() - bottom),
    )
}

/// 绘制一张谱（主谱与 inset 共用）。
#[allow(clippy::too_many_arguments)]
fn draw_spectrum(
    painter: &egui::Painter,
    item: egui::Rect,
    spec: &nmrstuff_core::Spectrum1D,
    analysis: &nmrstuff_core::Analysis,
    view: ViewRange,
    style: &SpectrumStyle,
    scale: f32,
    selected: bool,
    inset: bool,
    highlight: Option<(f64, f64)>,
) {
    let plot = plot_rect(item, scale, style);
    if plot.width() < 8.0 || plot.height() < 8.0 {
        return;
    }
    let (ymin, ymax) = nmrstuff_render::envelope::y_bounds(spec, view);
    let t = Transform::new(to_screen(plot), view, ymin, ymax);

    // 基线
    if style.show_axis {
        painter.line_segment(
            [plot.left_bottom(), plot.right_bottom()],
            egui::Stroke::new(0.8, egui::Color32::from_gray(150)),
        );
        // ppm 刻度
        let ticks = nice_ticks(view.to_ppm, view.from_ppm, if inset { 3 } else { 7 });
        let dec = (view.span() / 100.0).max(1e-6);
        for tk in ticks {
            let x = t.x_of_ppm(tk);
            if x < plot.left() - 1.0 || x > plot.right() + 1.0 {
                continue;
            }
            painter.line_segment(
                [egui::pos2(x, plot.bottom()), egui::pos2(x, plot.bottom() + 4.0 * scale)],
                egui::Stroke::new(0.7, egui::Color32::from_gray(140)),
            );
            painter.text(
                egui::pos2(x, plot.bottom() + 5.0 * scale),
                egui::Align2::CENTER_TOP,
                tick_text(tk, dec),
                egui::FontId::proportional((style.font_pt * scale * 0.82).max(7.0)),
                egui::Color32::from_gray(60),
            );
        }
        // 单位
        painter.text(
            egui::pos2(plot.right() + 2.0, plot.bottom() + 5.0 * scale),
            egui::Align2::LEFT_TOP,
            "ppm",
            egui::FontId::proportional((style.font_pt * scale * 0.75).max(6.5)),
            egui::Color32::from_gray(90),
        );
    }

    // 谱线
    let columns = plot.width().max(4.0) as usize;
    let cols = build_trace(spec, view, columns);
    if !cols.is_empty() {
        let mut pts: Vec<egui::Pos2> = Vec::with_capacity(cols.len() * 2);
        for c in &cols {
            let x = t.x_of_ppm(c.ppm);
            pts.push(egui::pos2(x, t.y_of(c.max)));
        }
        for c in cols.iter().rev() {
            let x = t.x_of_ppm(c.ppm);
            pts.push(egui::pos2(x, t.y_of(c.min)));
        }
        let color = egui::Color32::from_rgba_unmultiplied(
            style.color[0],
            style.color[1],
            style.color[2],
            style.color[3],
        );
        painter.add(egui::Shape::line(
            pts,
            egui::Stroke::new((style.line_width * scale * 1.4).max(0.6), color),
        ));
    }

    // 峰标注
    if style.show_peaks {
        let mut cands: Vec<(f32, f32, f32, f32)> = Vec::new();
        let mut labels: Vec<(String, bool)> = Vec::new();
        let font = egui::FontId::proportional((style.font_pt * scale * 0.8).max(7.0));
        let view_max = analysis
            .peaks
            .iter()
            .filter(|p| view.contains(p.ppm))
            .fold(0.0f32, |m, p| m.max(p.height));
        for p in &analysis.peaks {
            if !view.contains(p.ppm) {
                continue;
            }
            // 小峰不标注：教程里明确“自动标峰会标进一堆噪声/杂质”
            if p.height < view_max * 0.02 {
                continue;
            }
            let x = t.x_of_ppm(p.ppm);
            let y = t.y_of(p.height);
            let text = format!("{:.*}", style.decimals, p.ppm);
            let galley = painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::BLACK);
            cands.push((x, y, galley.size().x, galley.size().y));
            labels.push((text, p.kind == nmrstuff_core::PeakKind::Solvent));
        }
        let placed = layout_peak_labels(&cands, plot.top(), plot.right() - 4.0, 3.0);
        for (i, b) in placed.iter().enumerate() {
            let is_solvent = labels.get(i).map(|(_, s)| *s).unwrap_or(false);
            let color = if is_solvent {
                egui::Color32::from_rgb(190, 60, 60)
            } else {
                egui::Color32::from_rgb(30, 40, 50)
            };
            painter.line_segment(
                [
                    egui::pos2(b.anchor_x, b.anchor_y),
                    egui::pos2(b.x + b.w * 0.5, b.y + b.h),
                ],
                egui::Stroke::new(0.5, egui::Color32::from_gray(130)),
            );
            painter.text(
                egui::pos2(b.x, b.y),
                egui::Align2::LEFT_TOP,
                &labels[i].0,
                font.clone(),
                color,
            );
        }
    }

    // 积分曲线
    if style.show_integrals {
        for r in &analysis.integrals {
            if r.to_ppm > view.from_ppm || r.from_ppm < view.to_ppm {
                continue;
            }
            let pts: Vec<egui::Pos2> = nmrstuff_analysis::curve_points(spec, r, plot.width() as usize)
                .into_iter()
                .map(|(ppm, h)| {
                    // 曲线高度按图高比例
                    let y = plot.bottom() - (h as f32 / (ymax - ymin).max(1e-6)) * plot.height();
                    egui::pos2(t.x_of_ppm(ppm), y)
                })
                .collect();
            if pts.len() > 1 {
                painter.add(egui::Shape::line(
                    pts.clone(),
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 110, 180)),
                ));
                let end = pts[pts.len() - 1];
                painter.text(
                    egui::pos2(end.x + 2.0, end.y - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.1}H", r.value),
                    egui::FontId::proportional((style.font_pt * scale * 0.75).max(7.0)),
                    egui::Color32::from_rgb(40, 80, 140),
                );
            }
        }
    }

    // 多重峰括号
    if style.show_multiplets {
        for m in &analysis.multiplets {
            if m.to_ppm > view.from_ppm || m.from_ppm < view.to_ppm {
                continue;
            }
            let x0 = t.x_of_ppm(m.from_ppm.max(m.to_ppm));
            let x1 = t.x_of_ppm(m.from_ppm.min(m.to_ppm));
            if !(x1 - x0).is_finite() || (x1 - x0).abs() < 2.0 {
                continue;
            }
            let y = plot.top() + 14.0 * scale;
            let stroke = egui::Stroke::new(0.8, egui::Color32::from_rgb(0, 120, 120));
            painter.line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], stroke);
            painter.line_segment([egui::pos2(x0, y), egui::pos2(x0, y + 3.0)], stroke);
            painter.line_segment([egui::pos2(x1, y), egui::pos2(x1, y + 3.0)], stroke);
            let label = match m.j_text(1) {
                Some(j) => format!("{}  J = {} Hz", m.label.symbol(), j),
                None => m.label.symbol().to_string(),
            };
            painter.text(
                egui::pos2((x0 + x1) * 0.5, y - 1.0),
                egui::Align2::CENTER_BOTTOM,
                label,
                egui::FontId::proportional((style.font_pt * scale * 0.75).max(7.0)),
                egui::Color32::from_rgb(0, 100, 100),
            );
        }
    }

    // 高亮（拖动框选）
    if let Some((a, b)) = highlight {
        let x0 = t.x_of_ppm(a);
        let x1 = t.x_of_ppm(b);
        let r = egui::Rect::from_min_max(
            egui::pos2(x0.min(x1), plot.top()),
            egui::pos2(x0.max(x1), plot.bottom()),
        );
        painter.rect_filled(
            r,
            0.0,
            egui::Color32::from_rgba_unmultiplied(79, 163, 255, 40),
        );
        painter.rect_stroke(
            r,
            0.0,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }

    if selected {
        painter.rect_stroke(
            item,
            0.0,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Outside,
        );
    }
    if inset {
        painter.rect_stroke(
            item,
            0.0,
            egui::Stroke::new(0.8, egui::Color32::from_gray(120)),
            egui::StrokeKind::Outside,
        );
    }
}

// ---------------------------------------------------------------- 交互

fn interact(
    app: &mut App,
    ui: &egui::Ui,
    resp: &egui::Response,
    doc: &Document,
    page: egui::Rect,
    scale: f32,
) {
    let Some(main) = doc.page.primary_item() else { return };
    let item_rect = item_screen_rect(page, main.rect);
    let plot = plot_rect(item_rect, scale, &main.style);

    // 光标读数
    let mut hover_ppm = None;
    if let Some(pos) = resp.hover_pos() {
        if plot.contains(pos) {
            let t = Transform::new(
                to_screen(plot),
                main.view,
                -1.0,
                1.0,
            );
            let ppm = t.ppm_of_x(pos.x);
            if let Some(spec) = app.spectrum() {
                let intensity = spec.value_at_ppm(ppm);
                app.cursor = Some(crate::app::CursorInfo {
                    ppm,
                    intensity,
                    hz: ppm * spec.meta.spectrometer_mhz,
                });
            }
            hover_ppm = Some(ppm);
        } else {
            app.cursor = None;
        }
    } else {
        app.cursor = None;
    }

    // 滚轮缩放
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            if let Some(ppm) = hover_ppm {
                let factor = if scroll > 0.0 { 0.85 } else { 1.0 / 0.85 };
                let mut v = main.view.zoomed(factor, ppm);
                if let Some(data) = app.data_range() {
                    v = v.clamped_to(data, 0.02);
                }
                if v.span() > 0.001 {
                    app.set_view(v);
                }
            }
        }
    }

    // 拖动
    let ptr = resp.interact_pointer_pos();
    if resp.drag_started_by(egui::PointerButton::Middle) {
        if let Some(p) = ptr {
            app.drag = Drag::Pan {
                start_x: p.x,
                view: main.view,
            };
        }
    } else if resp.drag_started_by(egui::PointerButton::Primary) {
        if let Some(p) = ptr {
            match app.tool {
                Tool::Select => {
                    let mut hit = None;
                    for (i, it) in doc.page.items.iter().enumerate() {
                        if item_screen_rect(page, it.rect()).contains(p) {
                            hit = Some(i);
                        }
                    }
                    app.selected_item = hit;
                    app.drag = match hit {
                        Some(i) => Drag::MoveItem {
                            item: i,
                            grab: (
                                (p.x - page.left()) / page.width(),
                                (p.y - page.top()) / page.height(),
                            ),
                            rect: doc.page.items[i].rect(),
                        },
                        None => Drag::Pan {
                            start_x: p.x,
                            view: main.view,
                        },
                    };
                }
                Tool::Zoom | Tool::Integral | Tool::Multiplet => {
                    if let Some(ppm) = hover_ppm {
                        app.drag = Drag::Span {
                            start: ppm,
                            current: ppm,
                        };
                    }
                }
                _ => {}
            }
        }
    }

    if resp.dragged() {
        if let Some(p) = ptr {
            let mut new_drag = app.drag.clone();
            match &mut new_drag {
                Drag::Pan { start_x, view } => {
                    let ppm_px = view.span() / plot.width().max(1.0) as f64;
                    let delta = (*start_x - p.x) as f64 * ppm_px;
                    let mut v = view.panned(delta);
                    if let Some(data) = app.data_range() {
                        v = v.clamped_to(data, 0.05);
                    }
                    app.set_view(v);
                }
                Drag::Span { current, .. } => {
                    if let Some(ppm) = hover_ppm {
                        *current = ppm;
                    }
                }
                Drag::MoveItem { item, grab, rect } => {
                    let nx = (p.x - page.left()) / page.width() - grab.0;
                    let ny = (p.y - page.top()) / page.height() - grab.1;
                    let mut r = *rect;
                    r.x = nx;
                    r.y = ny;
                    r.clamp_inside();
                    if let Some(d) = app.doc_mut() {
                        if let Some(it) = d.page.items.get_mut(*item) {
                            *it.rect_mut() = r;
                        }
                    }
                }
                Drag::None => {}
            }
            app.drag = new_drag;
        }
    }

    if resp.drag_stopped() {
        let drag = std::mem::replace(&mut app.drag, Drag::None);
        match drag {
            Drag::Span { start, current } => {
                let (a, b) = (start.max(current), start.min(current));
                match app.tool {
                    Tool::Zoom => {
                        if (a - b) > main.view.span() * 0.001 {
                            app.set_view(ViewRange::new(a, b));
                        }
                    }
                    Tool::Integral => app.add_integral(a, b),
                    Tool::Multiplet => app.analyze_multiplet(a, b),
                    _ => {}
                }
            }
            Drag::MoveItem { .. } => {
                if let Some(before) = app.drag_before.take() {
                    app.push_undo("移动对象", before);
                }
            }
            _ => {}
        }
    }

    // 单击
    if resp.clicked() {
        if let Some(p) = ptr.or_else(|| resp.interact_pointer_pos()) {
            if let Some(ppm) = hover_ppm {
                match app.tool {
                    Tool::Peak => app.add_peak_near(ppm),
                    Tool::Reference => app.calibrate_at(ppm),
                    Tool::Select => {
                        // 选中最近的峰
                        if let Some(ds) = doc.datasets.first() {
                            let nearest = ds
                                .analysis
                                .peaks
                                .iter()
                                .enumerate()
                                .map(|(i, pk)| (i, (pk.ppm - ppm).abs()))
                                .filter(|(_, d)| *d < main.view.span() * 0.01)
                                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                            app.selected_peak = nearest.map(|(i, _)| i);
                        }
                    }
                    _ => {}
                }
            }
            let _ = p;
        }
    }

    // 记录拖动前的文档，供“移动对象”撤销
    if resp.drag_started() {
        app.drag_before = app.doc().cloned();
    }

    // 光标形状
    if resp.hovered() {
        let icon = match app.tool {
            Tool::Select => egui::CursorIcon::Default,
            Tool::Zoom => egui::CursorIcon::ZoomIn,
            _ => egui::CursorIcon::Crosshair,
        };
        ui.ctx().set_cursor_icon(icon);
    }
}

// ---------------------------------------------------------------- 窗口

pub fn windows(app: &mut App, ctx: &egui::Context) {
    if app.show_help {
        let mut open = app.show_help;
        egui::Window::new("帮助")
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(crate::app::HELP).monospace().size(12.0));
            });
        app.show_help = open;
    }

    if app.show_report {
        let mut open = app.show_report;
        egui::Window::new("报告")
            .open(&mut open)
            .default_width(560.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("复制").clicked() {
                        ui.ctx().copy_text(app.report_text.clone());
                        app.toast("报告已复制到剪贴板");
                    }
                    if ui.button("导出到 exports/").clicked() {
                        match export_report(&app.report_text) {
                            Ok(p) => app.toast(format!("已导出：{p}")),
                            Err(e) => app.fail(e),
                        }
                    }
                    if ui.button("刷新").clicked() {
                        app.build_report();
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut app.report_text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(20),
                    );
                });
            });
        app.show_report = open;
    }

    if app.show_recipe {
        let mut open = app.show_recipe;
        let json = app.recipe_json();
        egui::Window::new("处理链（可复现凭据）")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                if ui.button("复制 JSON").clicked() {
                    ui.ctx().copy_text(json.clone());
                    app.toast("处理链 JSON 已复制");
                }
                ui.separator();
                let mut text = json.clone();
                egui::ScrollArea::both().max_height(360.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
            });
        app.show_recipe = open;
    }

    if let Some(err) = app.error.clone() {
        let mut open = true;
        egui::Window::new("出错了")
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.colored_label(theme::ERROR, &err);
                ui.add_space(6.0);
                if ui.button("关闭").clicked() {
                    app.error = None;
                }
            });
        if !open {
            app.error = None;
        }
    }
}

fn export_report(text: &str) -> std::io::Result<String> {
    let dir = std::path::Path::new("exports");
    std::fs::create_dir_all(dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("report-{stamp}.txt"));
    std::fs::write(&path, text)?;
    Ok(path.to_string_lossy().to_string())
}
