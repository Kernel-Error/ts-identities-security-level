use crate::i18n::tr;
use crate::window::MainWindow;
use adw::prelude::*;
use gtk::gio;

pub fn on_activate(app: &adw::Application) {
    register_about_action(app);
    let window = MainWindow::new(app);
    window.present();
}

fn register_about_action(app: &adw::Application) {
    if app.lookup_action("about").is_some() {
        return;
    }
    let action = gio::SimpleAction::new("about", None);
    let app_weak = app.downgrade();
    action.connect_activate(move |_, _| {
        let Some(app) = app_weak.upgrade() else { return };
        show_about(&app);
    });
    app.add_action(&action);
}

fn show_about(app: &adw::Application) {
    let parent = app.active_window();
    let about = adw::AboutWindow::builder()
        .application_name(tr("TS3 Identity Level"))
        .application_icon(crate::APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .website("https://www.kernel-error.de")
        .issue_url("https://github.com/kernel-error/ts-identities-security-level/issues")
        .support_url("https://github.com/kernel-error/ts-identities-security-level")
        .developer_name("Sebastian van de Meer aka Kernel-Error")
        .developers(vec![
            "Sebastian van de Meer <kernel-error@kernel-error.com>".to_string(),
        ])
        .copyright("© 2026 Sebastian van de Meer (kernel-error.de)")
        .license_type(gtk::License::MitX11)
        .comments(tr(
"TS3 Identity Level computes the SHA-1 proof-of-work that determines a \
TeamSpeak 3 identity's security level — only much faster than the official \
client, by running on your GPU.

The tool reads the .ini file you export from TeamSpeak 3, finds a counter \
value whose SHA-1 of (public key ‖ counter) has more leading zero bits, \
and writes the new file atomically. A one-shot backup of the original is \
kept next to it.

Not affiliated with TeamSpeak Systems GmbH. The 'TeamSpeak' name is used \
here for descriptive purposes only.",
        ))
        .translator_credits(tr("translator-credits"))
        .build();

    about.add_link(
        &tr("Source code"),
        "https://github.com/kernel-error/ts-identities-security-level",
    );
    about.add_link(
        &tr("Usage guide"),
        "https://github.com/kernel-error/ts-identities-security-level/blob/main/docs/usage.md",
    );
    about.add_link(
        &tr("Algorithm specification"),
        "https://github.com/kernel-error/ts-identities-security-level/blob/main/docs/algorithm.md",
    );

    about.add_acknowledgement_section(
        Some(&tr("Algorithm references")),
        &[
            "landave — TSIdentityTool, TeamSpeakHasher",
            "thissepic — TeamSpeakHasher CUDA fork",
            "ReSpeak — tsdeclarations protocol notes",
            "hashcat — SHA-1 OpenCL kernel reference",
        ],
    );

    about.add_acknowledgement_section(
        Some(&tr("Runtime libraries")),
        &[
            "gtk4-rs & libadwaita-rs",
            "cudarc",
            "nvml-wrapper",
        ],
    );

    if let Some(parent) = parent {
        about.set_transient_for(Some(&parent));
        about.set_modal(true);
    }
    about.present();
}
