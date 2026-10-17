use android_activity::AndroidApp;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest, PhotocraftApp, Services};
use jni::objects::JValue;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const WORKSPACE_DIR: &str = "/sdcard/Download/ArtCraft";

fn ensure_workspace_dir() -> PathBuf {
    let p = PathBuf::from(WORKSPACE_DIR);
    let _ = std::fs::create_dir_all(&p);
    p
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    photocraft_format::atomic_write(path, bytes).map_err(|e| e.to_string())
}

fn write_atomic_progress(path: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> Result<(), String> {
    photocraft_format::atomic_write_progress(path, bytes, progress).map_err(|e| e.to_string())
}

const ENCODE_SHARE: f32 = 0.85;

fn save_through(
    doc: &Document,
    path: &str,
    settings: &photocraft_ui_egui::ExportSettings,
    ctl: &photocraft_ui_egui::jobs_ui::SaveCtl,
    write: impl Fn(&str, &[u8], &dyn Fn(f32)) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    ctl.progress(0.0, "Encoding");
    let opts = settings.io_options();
    let cancelled = || ctl.check().is_err();
    let progress = |f: f32| ctl.progress(f * ENCODE_SHARE, "");
    let interrupt = photocraft_raster::Interrupt::new(&cancelled, &progress);
    let r = photocraft_io::export_with(doc, path, &opts, &interrupt).map_err(|e| e.to_string())?;
    ctl.commit()?;
    ctl.progress(ENCODE_SHARE, "Writing");
    write(path, &r.bytes, &|f| ctl.progress(ENCODE_SHARE + (0.99 - ENCODE_SHARE) * f, ""))?;
    ctl.progress(1.0, "");
    Ok(r.warnings)
}

fn save_file(
    doc: &Document,
    path: &str,
    settings: &photocraft_ui_egui::ExportSettings,
    ctl: &photocraft_ui_egui::jobs_ui::SaveCtl,
) -> Result<Vec<String>, String> {
    save_through(doc, path, settings, ctl, |p, b, progress| write_atomic_progress(Path::new(p), b, progress))
}

fn show_android_file_dialog(request: FileDialogRequest, _parent: Option<&eframe::Frame>, reply: FileDialogReply) {
    let base_dir = ensure_workspace_dir();
    match request {
        FileDialogRequest::Open { multiple: _, initial_dir, extensions: _ } => {
            let scan_dir = initial_dir.map(PathBuf::from).unwrap_or(base_dir);
            let mut candidates = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&scan_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        candidates.push(path.to_string_lossy().into_owned());
                    }
                }
            }
            candidates.sort();
            if !candidates.is_empty() {
                reply.send(Some(FileDialogAnswer::Paths(candidates)));
            } else {
                reply.send(None);
            }
        }
        FileDialogRequest::Folder { initial_dir } => {
            let dir = initial_dir.map(PathBuf::from).unwrap_or(base_dir);
            reply.send(Some(FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned())));
        }
        FileDialogRequest::Save { suggested } => {
            let path = Path::new(&suggested);
            let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| suggested.clone());
            let target = base_dir.join(file_name);
            reply.send(Some(FileDialogAnswer::SaveTo(target.to_string_lossy().into_owned())));
        }
    }
}

fn create_android_services() -> Services {
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], max_svg_group_depth: usize| {
            photocraft_io::import_with_svg_group_depth(name, bytes, max_svg_group_depth)
                .map(|r| (r.document, r.warnings))
                .map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let opts = settings.io_options();
            photocraft_io::export(doc, path, &opts)
                .map(|r| (r.bytes, r.warnings))
                .map_err(|e| e.to_string())
        })),
        save_file: Some(Arc::new(save_file)),
        file_dialog: Some(Box::new(show_android_file_dialog)),
        write: Some(Box::new(|path: &str, bytes: &[u8]| write_atomic(Path::new(path), bytes))),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        ..Default::default()
    }
}

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
            let services = create_android_services();
            let mut app = PhotocraftApp::new(session, services);
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.set_wgpu(rs);
            }
            Ok(Box::new(app))
        }),
    );
}
