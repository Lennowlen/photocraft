use android_activity::AndroidApp;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest, PhotocraftApp, Services};
use jni::objects::JValue;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

#[derive(Clone, Debug, PartialEq)]
enum DialogMode {
    Open,
    Save,
}

struct PendingDialog {
    mode: DialogMode,
    current_dir: PathBuf,
    selected_file: Option<PathBuf>,
    save_filename: String,
    reply: Option<FileDialogReply>,
}

static PENDING_DIALOG: Mutex<Option<PendingDialog>> = Mutex::new(None);

fn show_android_file_dialog(request: FileDialogRequest, _parent: Option<&eframe::Frame>, reply: FileDialogReply) {
    let base_dir = ensure_workspace_dir();
    match request {
        FileDialogRequest::Open { initial_dir, .. } => {
            let scan_dir = initial_dir.map(PathBuf::from).unwrap_or(base_dir);
            let mut dlg = PENDING_DIALOG.lock().unwrap();
            *dlg = Some(PendingDialog {
                mode: DialogMode::Open,
                current_dir: scan_dir,
                selected_file: None,
                save_filename: String::new(),
                reply: Some(reply),
            });
        }
        FileDialogRequest::Folder { initial_dir } => {
            let dir = initial_dir.map(PathBuf::from).unwrap_or(base_dir);
            reply.send(Some(FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned())));
        }
        FileDialogRequest::Save { suggested } => {
            let path = Path::new(&suggested);
            let filename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| suggested.clone());
            let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).map(PathBuf::from).unwrap_or(base_dir);
            let mut dlg = PENDING_DIALOG.lock().unwrap();
            *dlg = Some(PendingDialog {
                mode: DialogMode::Save,
                current_dir: dir,
                selected_file: None,
                save_filename: filename,
                reply: Some(reply),
            });
        }
    }
}

fn render_pending_file_dialog(ctx: &egui::Context) {
    let mut dlg_guard = PENDING_DIALOG.lock().unwrap();
    let Some(ref mut dlg) = *dlg_guard else { return };

    let is_open = dlg.mode == DialogMode::Open;
    let title = if is_open { "Buka Berkas (Open File)" } else { "Simpan Berkas (Save As)" };

    let mut send_answer: Option<Option<FileDialogAnswer>> = None;
    let mut navigate_to: Option<PathBuf> = None;

    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .default_size([720.0, 520.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(4.0);

            // Quick jump folders
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Lokasi:").strong());
                if ui.button("ArtCraft Workspace").clicked() {
                    navigate_to = Some(ensure_workspace_dir());
                }
                if ui.button("Download").clicked() {
                    navigate_to = Some(PathBuf::from("/sdcard/Download"));
                }
                if ui.button("Pictures").clicked() {
                    navigate_to = Some(PathBuf::from("/sdcard/Pictures"));
                }
                if ui.button("Documents").clicked() {
                    navigate_to = Some(PathBuf::from("/sdcard/Documents"));
                }
            });

            ui.add_space(4.0);
            ui.separator();

            // Current Directory display & Up button
            ui.horizontal(|ui| {
                if let Some(parent) = dlg.current_dir.parent() {
                    if ui.button("â¬† Naung / Up").clicked() {
                        navigate_to = Some(parent.to_path_buf());
                    }
                }
                ui.label(egui::RichText::new(dlg.current_dir.to_string_lossy()).monospace());
            });

            ui.separator();
            ui.add_space(4.0);

            // File & Directory list
            let mut entries_list = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&dlg.current_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let is_dir = path.is_dir();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                    if !name.starts_with('.') {
                        entries_list.push((name, path, is_dir));
                    }
                }
            }
            entries_list.sort_by(|a, b| {
                if a.2 == b.2 {
                    a.0.to_lowercase().cmp(&b.0.to_lowercase())
                } else if a.2 {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            });

            egui::ScrollArea::vertical()
                .max_height(300.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if entries_list.is_empty() {
                        ui.label(egui::RichText::new("Direktori ini kosong.").italics());
                    }
                    for (name, path, is_dir) in entries_list {
                        let is_selected = dlg.selected_file.as_ref() == Some(&path);
                        ui.horizontal(|ui| {
                            let icon = if is_dir { "ðŸ“" } else { "ðŸ–¼ï¸" };
                            let label = format!("{} {}", icon, name);
                            let resp = ui.selectable_label(is_selected, label);
                            if resp.clicked() {
                                if is_dir {
                                    navigate_to = Some(path.clone());
                                } else {
                                    dlg.selected_file = Some(path.clone());
                                    if !is_open {
                                        dlg.save_filename = name.clone();
                                    }
                                }
                            }
                            if resp.double_clicked() {
                                if is_dir {
                                    navigate_to = Some(path);
                                } else if is_open {
                                    send_answer = Some(Some(FileDialogAnswer::Paths(vec![path.to_string_lossy().into_owned()])));
                                }
                            }
                        });
                    }
                });

            ui.separator();
            ui.add_space(6.0);

            // Save filename input if in Save mode
            if !is_open {
                ui.horizontal(|ui| {
                    ui.label("Nama Berkas:");
                    ui.text_edit_singleline(&mut dlg.save_filename);
                });
                ui.add_space(4.0);
            }

            // Action buttons
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("Batal (Cancel)").color(egui::Color32::from_rgb(220, 80, 80))).clicked() {
                    send_answer = Some(None);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_open {
                        let can_open = dlg.selected_file.is_some();
                        let btn = ui.add_enabled(can_open, egui::Button::new(egui::RichText::new("Buka (Open)").strong()));
                        if btn.clicked() {
                            if let Some(ref sel) = dlg.selected_file {
                                send_answer = Some(Some(FileDialogAnswer::Paths(vec![sel.to_string_lossy().into_owned()])));
                            }
                        }
                    } else {
                        let can_save = !dlg.save_filename.trim().is_empty();
                        let btn = ui.add_enabled(can_save, egui::Button::new(egui::RichText::new("Simpan (Save)").strong()));
                        if btn.clicked() {
                            let target = dlg.current_dir.join(dlg.save_filename.trim());
                            send_answer = Some(Some(FileDialogAnswer::SaveTo(target.to_string_lossy().into_owned())));
                        }
                    }
                });
            });
        });

    if let Some(new_dir) = navigate_to {
        dlg.current_dir = new_dir;
        dlg.selected_file = None;
    }

    if let Some(answer) = send_answer {
        if let Some(reply) = dlg.reply.take() {
            reply.send(answer);
        }
        *dlg_guard = None;
    }
}

struct PhotoCraftAndroidApp {
    inner: PhotocraftApp,
}

impl eframe::App for PhotoCraftAndroidApp {
    fn on_exit(&mut self) {
        self.inner.on_exit();
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.inner.logic(ctx, frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.inner.raw_input_hook(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        render_pending_file_dialog(ui.ctx());
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
            let mut session = Session::default();
            session.color.settings.ask_on_mismatch = false;
            session.color.settings.ask_on_missing = false;
            session.color.settings.ask_on_paste = false;
            let services = create_android_services();
            let mut app = PhotocraftApp::new(session, services);
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.set_wgpu(rs);
            }
            Ok(Box::new(PhotoCraftAndroidApp { inner: app }))
        }),
    );
}
