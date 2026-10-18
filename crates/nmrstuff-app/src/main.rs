//! NMRStuff —— 纯 Rust + 原生 GUI 的轻量 NMR 工作站（demo）。
//!
//! 当前处于早期框架开发阶段：
//! 数据模型、处理链、分析、渲染几何、编排层与 GUI 已分层落地，
//! 重点验证“框架能不能撑住后续功能”，而不是功能数量。

mod app;
mod theme;
mod ui;

use app::App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t0 = std::time::Instant::now();
        let ctx = ui.ctx().clone();
        self.handle_keys(&ctx);

        ui::top_bar(self, ui);
        ui::left_panel(self, ui);
        ui::right_panel(self, ui);
        ui::bottom_bar(self, ui);
        ui::canvas(self, ui);
        ui::windows(self, &ctx);

        self.frame_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = theme::BG;
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            1.0,
        ]
    }
}

fn main() -> eframe::Result {
    // 渲染后端：默认 glow(OpenGL)，可用环境变量切换。
    // 原因：部分远程桌面/老显卡驱动上 wgpu(DX12) 会在初始化阶段崩溃，
    //       而 OpenGL 1.1-only 的环境则会提示 “requires opengl 2.0+”。
    // 用法：NMRSTUFF_RENDERER=wgpu nmrstuff.exe
    let renderer = match std::env::var("NMRSTUFF_RENDERER").unwrap_or_default().as_str() {
        "wgpu" => eframe::Renderer::Wgpu,
        _ => eframe::Renderer::Glow,
    };
    let options = eframe::NativeOptions {
        renderer,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([1080.0, 680.0])
            .with_title("NMRStuff — 轻量 NMR 工作站"),
        ..Default::default()
    };
    eframe::run_native(
        "NMRStuff",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
