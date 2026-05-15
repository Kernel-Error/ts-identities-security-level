//! Main window: file picker → identity details → device → target → start
//! / stop, with a live progress block. The heavy work runs on a worker
//! thread (see `worker.rs`); UI updates arrive on an `async_channel`.

use crate::i18n::tr;
use ts3level_engine::gpu_stats::HISTORY_LEN;
use ts3level_engine::GpuStats;
use crate::worker::{spawn_worker, WorkerCommand, WorkerEvent};
use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::subclass::InitializingObject;
use gtk::{gio, glib, CompositeTemplate};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use ts3level_engine::{DeviceInfo, HashEngine};

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/de/kernel_error/Ts3Level/main_window.ui")]
    pub struct MainWindow {
        #[template_child]
        pub identity_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub pick_file_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub details_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub nickname_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub local_id_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub current_level_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub current_counter_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub fingerprint_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub pubkey_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub device_combo: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub endless_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub target_spin: TemplateChild<adw::SpinRow>,
        #[template_child]
        pub start_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub stop_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub level_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub counter_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub hashrate_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub eta_next_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub eta_target_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub eta_target_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub status_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub gpu_stats_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub util_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub vram_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub temp_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub power_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub util_graph: TemplateChild<gtk::DrawingArea>,

        pub selected_path: RefCell<Option<PathBuf>>,
        pub devices: RefCell<Vec<DeviceInfo>>,
        pub worker_tx: RefCell<Option<Sender<WorkerCommand>>>,
        pub stop_flag: RefCell<Option<Arc<AtomicBool>>>,
        pub gpu_stats: RefCell<Option<GpuStats>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MainWindow {
        const NAME: &'static str = "Ts3LevelMainWindow";
        type Type = super::MainWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MainWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }
    impl WidgetImpl for MainWindow {}
    impl WindowImpl for MainWindow {}
    impl ApplicationWindowImpl for MainWindow {}
    impl AdwApplicationWindowImpl for MainWindow {}
}

glib::wrapper! {
    pub struct MainWindow(ObjectSubclass<imp::MainWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl MainWindow {
    pub fn new(app: &adw::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    fn setup(&self) {
        self.populate_devices();
        self.bind_endless_switch();
        self.connect_pick_file();
        self.connect_start();
        self.connect_stop();
        self.setup_gpu_stats();
    }

    fn setup_gpu_stats(&self) {
        let imp = self.imp();

        // Initial GpuStats for whichever device is selected (default 0).
        let stats = GpuStats::new(imp.device_combo.selected());
        let available = stats.available();
        *imp.gpu_stats.borrow_mut() = Some(stats);
        if !available {
            imp.gpu_stats_group.set_visible(false);
            imp.util_graph.set_visible(false);
            return;
        }

        // Re-sample when the user picks a different GPU.
        let win = self.downgrade();
        imp.device_combo.connect_selected_notify(move |combo| {
            let Some(win) = win.upgrade() else { return };
            let mut stats = win.imp().gpu_stats.borrow_mut();
            if let Some(s) = stats.as_mut() {
                s.set_device(combo.selected());
            }
            drop(stats);
            win.imp().util_graph.queue_draw();
        });

        // Cairo draw callback — reads `imp.gpu_stats.history`.
        let win = self.downgrade();
        imp.util_graph
            .set_draw_func(move |_area, ctx, w, h| {
                let Some(win) = win.upgrade() else { return };
                draw_util_graph(&win, ctx, w, h);
            });

        // 1 Hz polling on the main loop. Stops when the window is dropped.
        let win = self.downgrade();
        glib::timeout_add_seconds_local(1, move || {
            let Some(win) = win.upgrade() else {
                return glib::ControlFlow::Break;
            };
            win.tick_gpu_stats();
            glib::ControlFlow::Continue
        });
    }

    fn tick_gpu_stats(&self) {
        let imp = self.imp();
        let sample = {
            let mut stats = imp.gpu_stats.borrow_mut();
            let Some(s) = stats.as_mut() else { return };
            s.poll()
        };
        imp.util_label.set_text(
            &sample
                .util_pct
                .map(|u| format!("{u} %"))
                .unwrap_or_else(|| "—".into()),
        );
        imp.vram_label.set_text(&match (sample.mem_used_mib, sample.mem_total_mib) {
            (Some(u), Some(t)) => format!(
                "{:.1} / {:.1} GiB",
                u as f64 / 1024.0,
                t as f64 / 1024.0
            ),
            _ => "—".into(),
        });
        imp.temp_label.set_text(
            &sample
                .temp_c
                .map(|c| format!("{c} °C"))
                .unwrap_or_else(|| "—".into()),
        );
        imp.power_label.set_text(
            &sample
                .power_w
                .map(|p| format!("{p:.0} W"))
                .unwrap_or_else(|| "—".into()),
        );
        imp.util_graph.queue_draw();
    }

    fn populate_devices(&self) {
        let imp = self.imp();
        let engine = ts3level_cuda::CudaEngine::new();
        let model = gtk::StringList::new(&[]);
        match engine.enumerate() {
            Ok(devs) if !devs.is_empty() => {
                for d in &devs {
                    model.append(&d.summary());
                }
                *imp.devices.borrow_mut() = devs;
                imp.device_combo.set_sensitive(true);
            }
            Ok(_) => {
                model.append(&tr("No CUDA device found"));
                imp.device_combo.set_sensitive(false);
                imp.start_button.set_sensitive(false);
            }
            Err(e) => {
                model.append(&format!("{}: {e}", tr("Driver error")));
                imp.device_combo.set_sensitive(false);
                imp.start_button.set_sensitive(false);
            }
        }
        imp.device_combo.set_model(Some(&model));
    }

    fn bind_endless_switch(&self) {
        let imp = self.imp();
        let spin = imp.target_spin.clone();
        imp.endless_switch
            .bind_property("active", &spin, "sensitive")
            .invert_boolean()
            .sync_create()
            .build();
        // Also hide the "ETA to target" row when in endless mode.
        let eta_row = imp.eta_target_row.clone();
        imp.endless_switch
            .bind_property("active", &eta_row, "visible")
            .invert_boolean()
            .sync_create()
            .build();
    }

    fn connect_pick_file(&self) {
        let imp = self.imp();
        let win = self.clone();
        imp.pick_file_button.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&tr("TeamSpeak 3 identity (*.ini)")));
            filter.add_pattern("*.ini");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);

            let dialog = gtk::FileDialog::builder()
                .title(tr("Pick a TeamSpeak 3 identity"))
                .filters(&filters)
                .modal(true)
                .build();
            let win_cb = win.clone();
            dialog.open(Some(&win), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        win_cb.set_identity_path(path);
                    }
                }
            });
        });
    }

    fn set_identity_path(&self, path: PathBuf) {
        let imp = self.imp();
        imp.identity_row
            .set_subtitle(&path.display().to_string());
        *imp.selected_path.borrow_mut() = Some(path.clone());

        if let Err(e) = self.load_identity_details(&path) {
            self.reset_identity_details();
            imp.status_label
                .set_text(&format!("{}: {e}", tr("Cannot read identity")));
        }
    }

    fn reset_identity_details(&self) {
        let imp = self.imp();
        imp.nickname_label.set_text("—");
        imp.local_id_label.set_text("—");
        imp.current_level_label.set_text("—");
        imp.current_counter_label.set_text("—");
        imp.fingerprint_label.set_text("—");
        imp.pubkey_label.set_text("—");
    }

    fn load_identity_details(&self, path: &std::path::Path) -> Result<(), String> {
        let imp = self.imp();
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let id = ts3level_core::IdentityFile::parse(&bytes).map_err(|e| e.to_string())?;
        let kp = ts3level_core::pubkey::KeyPair::from_blob_b64(id.blob_b64())
            .map_err(|e| e.to_string())?;
        let pubkey_b64 = kp.public_key_base64();
        let level = ts3level_core::level::compute_level(&pubkey_b64, id.counter());

        imp.nickname_label
            .set_text(id.nickname().unwrap_or("—"));
        imp.local_id_label
            .set_text(id.local_id().unwrap_or("—"));
        imp.current_level_label.set_text(&level.to_string());
        imp.current_counter_label
            .set_text(&id.counter().to_string());
        imp.fingerprint_label.set_text(&kp.fingerprint_b64());
        imp.pubkey_label.set_text(&pubkey_b64);
        // Seed the live progress block as well so the user immediately
        // sees a sensible starting state.
        imp.level_label.set_text(&level.to_string());
        imp.counter_label.set_text(&id.counter().to_string());

        // If the current target would be reached immediately, bump it so
        // the user does not start a no-op run. We only raise — never
        // lower — to preserve a value the user already typed.
        let spin = imp.target_spin.get();
        let adj = spin.adjustment();
        let current_target = spin.value() as u32;
        let suggested = (level as u32).saturating_add(1).min(adj.upper() as u32);
        if current_target <= level as u32 {
            spin.set_value(suggested as f64);
        }
        Ok(())
    }

    fn connect_start(&self) {
        let imp = self.imp();
        let win = self.clone();
        imp.start_button.connect_clicked(move |_| {
            win.on_start_clicked();
        });
    }

    fn connect_stop(&self) {
        let imp = self.imp();
        let win = self.clone();
        imp.stop_button.connect_clicked(move |_| {
            if let Some(flag) = win.imp().stop_flag.borrow().as_ref() {
                flag.store(true, Ordering::SeqCst);
            }
            win.imp().status_label.set_text(&tr("Stopping…"));
        });
    }

    fn on_start_clicked(&self) {
        let imp = self.imp();
        let Some(path) = imp.selected_path.borrow().clone() else {
            imp.status_label.set_text(&tr("Pick an identity file first"));
            return;
        };
        let device_index = imp.device_combo.selected();
        if (device_index as usize) >= imp.devices.borrow().len() {
            imp.status_label.set_text(&tr("No device selected"));
            return;
        }

        let endless = imp.endless_switch.is_active();
        let target = imp.target_spin.value() as u8;

        imp.start_button.set_sensitive(false);
        imp.stop_button.set_sensitive(true);
        imp.hashrate_label.set_text("—");
        imp.eta_next_label.set_text("—");
        imp.eta_target_label.set_text("—");
        imp.status_label.set_text(&tr("Running…"));

        let (sender, receiver) = async_channel::unbounded::<WorkerEvent>();
        let win = Rc::new(self.clone());
        glib::spawn_future_local({
            let win = Rc::clone(&win);
            async move {
                while let Ok(evt) = receiver.recv().await {
                    win.on_event(evt);
                }
            }
        });

        let (worker_tx, stop_flag) =
            spawn_worker(path, device_index as u32, endless, target, sender);
        *imp.worker_tx.borrow_mut() = Some(worker_tx);
        *imp.stop_flag.borrow_mut() = Some(stop_flag);
    }

    fn on_event(&self, evt: WorkerEvent) {
        let imp = self.imp();
        match evt {
            WorkerEvent::Tick {
                hashrate_hps,
                best_level,
                best_counter,
                eta_next_secs,
                eta_target_secs,
            } => {
                imp.level_label.set_text(&best_level.to_string());
                imp.counter_label.set_text(&best_counter.to_string());
                imp.hashrate_label
                    .set_text(&format!("{:.2} GH/s", hashrate_hps / 1e9));
                imp.eta_next_label.set_text(&format_duration(eta_next_secs));
                imp.eta_target_label
                    .set_text(&format_duration(eta_target_secs));
            }
            WorkerEvent::NewBest { level, counter } => {
                imp.level_label.set_text(&level.to_string());
                imp.counter_label.set_text(&counter.to_string());
                imp.current_level_label.set_text(&level.to_string());
                imp.current_counter_label.set_text(&counter.to_string());
                imp.status_label
                    .set_text(&format!("{} {}", tr("New best level:"), level));
            }
            WorkerEvent::Finished { reason } => {
                imp.start_button.set_sensitive(true);
                imp.stop_button.set_sensitive(false);
                imp.status_label.set_text(&reason);
                *imp.worker_tx.borrow_mut() = None;
                *imp.stop_flag.borrow_mut() = None;
            }
            WorkerEvent::Error { message } => {
                imp.start_button.set_sensitive(true);
                imp.stop_button.set_sensitive(false);
                imp.status_label
                    .set_text(&format!("{}: {}", tr("Error"), message));
                *imp.worker_tx.borrow_mut() = None;
                *imp.stop_flag.borrow_mut() = None;
            }
        }
    }
}

/// Draw the GPU utilization graph: gridlines, a filled area, and the
/// current-sample stroke. Reads the history straight off the window's
/// `gpu_stats` cell.
fn draw_util_graph(win: &MainWindow, ctx: &gtk::cairo::Context, w: i32, h: i32) {
    let stats = win.imp().gpu_stats.borrow();
    let Some(stats) = stats.as_ref() else { return };
    let w = w as f64;
    let h = h as f64;
    let history: Vec<f32> = stats.history.iter().copied().collect();

    // Gridlines: 0/25/50/75/100 %.
    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.07);
    ctx.set_line_width(1.0);
    for i in 0..=4 {
        let y = h * (i as f64) / 4.0;
        ctx.move_to(0.0, y);
        ctx.line_to(w, y);
    }
    let _ = ctx.stroke();

    if history.is_empty() {
        return;
    }
    let n = HISTORY_LEN.max(1);
    let dx = if n > 1 {
        w / (n as f64 - 1.0)
    } else {
        w
    };
    let pad = n.saturating_sub(history.len());
    let at = |i: usize| -> (f64, f64) {
        let val = if i < pad { 0.0 } else { history[i - pad] };
        let x = i as f64 * dx;
        let y = h - (val as f64 / 100.0) * h;
        (x, y)
    };

    // Filled area below the curve.
    ctx.set_source_rgba(0.36, 0.66, 0.89, 0.18);
    let (x0, y0) = at(0);
    ctx.move_to(x0, h);
    ctx.line_to(x0, y0);
    for i in 1..n {
        let (x, y) = at(i);
        ctx.line_to(x, y);
    }
    ctx.line_to(w, h);
    ctx.close_path();
    let _ = ctx.fill();

    // Stroke on top.
    ctx.set_source_rgba(0.36, 0.66, 0.89, 1.0);
    ctx.set_line_width(2.0);
    let (x0, y0) = at(0);
    ctx.move_to(x0, y0);
    for i in 1..n {
        let (x, y) = at(i);
        ctx.line_to(x, y);
    }
    let _ = ctx.stroke();
}

/// Human-readable duration. `None` → em-dash, infinity → ∞.
fn format_duration(secs: Option<f64>) -> String {
    let Some(s) = secs else { return "—".into() };
    if !s.is_finite() {
        return "∞".into();
    }
    if s < 1.0 {
        return format!("{:.0} ms", s * 1000.0);
    }
    if s < 60.0 {
        return format!("{s:.1} s");
    }
    let m = s / 60.0;
    if m < 60.0 {
        return format!("{m:.1} min");
    }
    let h = m / 60.0;
    if h < 24.0 {
        return format!("{h:.1} h");
    }
    let d = h / 24.0;
    if d < 365.0 {
        return format!("{d:.1} d");
    }
    format!("{:.1} y", d / 365.0)
}
