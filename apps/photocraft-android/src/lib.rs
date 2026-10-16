use android_activity::AndroidApp;
use photocraft_engine::Session;
use photocraft_ui_egui::{PhotocraftApp, Services};

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("PhotoCraft"),
    );

    log::info!("Starting PhotoCraft on Android ARM64...");

    let mut options = eframe::NativeOptions::default();
    options.android_app = Some(app);

    let _ = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(|cc| {
            let session = Session::default();
            let services = Services::default();
            let mut app = PhotocraftApp::new(session, services);
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.set_wgpu(rs);
            }
            Ok(Box::new(app))
        }),
    );
}
