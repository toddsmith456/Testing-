fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([420.0, 840.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Nostur Rust",
        options,
        Box::new(|cc| {
            Ok(Box::new(nostur_rust::ui::NosturApp::new(
                cc,
                desktop_data_dir(),
            )))
        }),
    )
}

fn desktop_data_dir() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return Some(std::path::PathBuf::from(path).join("nostur-rust"));
    }
    std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".local/share/nostur-rust"))
}
