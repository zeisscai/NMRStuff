//! 主题与字体：深色界面 + 优先加载系统中文字体。

use eframe::egui;

pub const BG: egui::Color32 = egui::Color32::from_rgb(0x12, 0x14, 0x18);
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x1d, 0x23);
pub const PANEL_HI: egui::Color32 = egui::Color32::from_rgb(0x22, 0x26, 0x2e);
pub const LINE: egui::Color32 = egui::Color32::from_rgb(0x2e, 0x33, 0x3d);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xdc, 0xe1, 0xea);
pub const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x93, 0xa2);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4f, 0xa3, 0xff);
pub const ACCENT_DIM: egui::Color32 = egui::Color32::from_rgb(0x2b, 0x5d, 0x94);
pub const WARN: egui::Color32 = egui::Color32::from_rgb(0xf0, 0xb4, 0x4b);
pub const ERROR: egui::Color32 = egui::Color32::from_rgb(0xff, 0x6b, 0x6b);

/// 纸张颜色（出版物的白纸，不随主题变化）
pub const PAPER: egui::Color32 = egui::Color32::from_rgb(0xff, 0xff, 0xff);

/// 尝试加载系统中文字体作为回退字体。返回是否成功。
pub fn install_fonts(ctx: &egui::Context) -> bool {
    const CANDIDATES: &[&str] = &[
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/Deng.ttf",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simsun.ttc",
        "C:/Windows/Fonts/simkai.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/System/Library/Fonts/PingFang.ttc",
    ];
    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        let name = "cjk".to_owned();
        fonts
            .font_data
            .insert(name.clone(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push(name.clone());
        }
        ctx.set_fonts(fonts);
        return true;
    }
    false
}

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL_HI;
    v.extreme_bg_color = BG;
    v.faint_bg_color = PANEL_HI;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    v.window_stroke = egui::Stroke::new(1.0, LINE);
    v.widgets.noninteractive.bg_fill = PANEL_HI;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE);
    v.widgets.inactive.bg_fill = PANEL_HI;
    v.widgets.inactive.weak_bg_fill = PANEL_HI;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, LINE);
    v.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x2c, 0x33, 0x3e);
    v.widgets.active.bg_fill = ACCENT_DIM;
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(7.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(13.0),
        );
        s.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional(11.0),
        );
        s.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(16.0),
        );
        s.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(13.0),
        );
    });
}
