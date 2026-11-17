pub mod core;
pub mod publish;
pub mod session;
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

#[cfg(test)]
mod android_packaging_tests {
    #[test]
    fn native_activity_manifest_declares_no_dex_code() {
        let manifest = include_str!("../android/AndroidManifest.xml");
        assert!(
            manifest.contains("android:hasCode=\"false\""),
            "NativeActivity APK without DEX must declare hasCode=false"
        );
        assert!(manifest.contains("android.app.NativeActivity"));
    }
}
