struct Min;
impl eframe::App for Min {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.label("hello");
    }
}
fn main() -> eframe::Result {
    eprintln!("[min] entry");
    eframe::run_native(
        "min",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(|_cc| {
            eprintln!("[min] creating app");
            Ok(Box::new(Min))
        }),
    )
}
