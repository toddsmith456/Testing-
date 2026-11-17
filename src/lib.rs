pub mod core;
pub mod storage;
pub mod ui;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    let data_dir = app.internal_data_path();
    let opts = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "Nostur",
        opts,
        Box::new(move |cc| Ok(Box::new(ui::NosturApp::new(cc, data_dir.clone())))),
    );
}
