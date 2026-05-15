mod app;
mod i18n;
mod window;
mod worker;

use adw::prelude::*;
use gtk::gio;

pub const APP_ID: &str = "de.kernel_error.Ts3Level";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    i18n::init();

    gio::resources_register_include!("ts3level.gresource")
        .expect("failed to register compiled gresources");

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|app| {
        // Make the icon embedded in our GResource discoverable through the
        // standard icon-name lookup that AdwAboutWindow uses.
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::IconTheme::for_display(&display)
                .add_resource_path("/de/kernel_error/Ts3Level/icons");
        }
        let _ = app;
    });
    app.connect_activate(app::on_activate);
    app.run()
}
