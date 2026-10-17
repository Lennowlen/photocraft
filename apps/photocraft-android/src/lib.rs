use android_activity::AndroidApp;
use photocraft_engine::Session;
use photocraft_ui_egui::{PhotocraftApp, Services};
use jni::objects::JValue;

fn apply_immersive_sticky(app: &AndroidApp) {
    let vm_ptr = app.vm_as_ptr();
    let activity_ptr = app.activity_as_ptr();
    if vm_ptr.is_null() || activity_ptr.is_null() {
        return;
    }

    unsafe {
        let jvm = match jni::JavaVM::from_raw(vm_ptr as *mut jni::sys::JavaVM) {
            Ok(jvm) => jvm,
            Err(_) => return,
        };
        let mut env = match jvm.attach_current_thread() {
            Ok(env) => env,
            Err(_) => return,
        };
        let activity = match jni::objects::JObject::from_raw(activity_ptr as jni::sys::jobject) {
            activity if !activity.as_raw().is_null() => activity,
            _ => return,
        };

        if let Ok(window) = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[]) {
            if let Ok(window_obj) = window.l() {
                if let Ok(decor_view) = env.call_method(&window_obj, "getDecorView", "()Landroid/view/View;", &[]) {
                    if let Ok(decor_obj) = decor_view.l() {
                        // SYSTEM_UI_FLAG_IMMERSIVE_STICKY (4096)
                        // | SYSTEM_UI_FLAG_FULLSCREEN (4)
                        // | SYSTEM_UI_FLAG_HIDE_NAVIGATION (2)
                        // | SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN (1024)
                        // | SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION (512)
                        // | SYSTEM_UI_FLAG_LAYOUT_STABLE (256)
                        let flags: i32 = 4096 | 4 | 2 | 1024 | 512 | 256;
                        let _ = env.call_method(&decor_obj, "setSystemUiVisibility", "(I)V", &[JValue::Int(flags)]);
                    }
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("PhotoCraft"),
    );

    log::info!("Starting PhotoCraft on Android ARM64...");

    apply_immersive_sticky(&app);

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
