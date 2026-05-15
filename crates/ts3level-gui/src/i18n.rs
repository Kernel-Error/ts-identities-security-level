use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};

const DOMAIN: &str = "ts3level";

pub fn init() {
    let _ = setlocale(LocaleCategory::LcAll, "");
    let _ = bindtextdomain(DOMAIN, locale_dir());
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = textdomain(DOMAIN);
}

pub fn tr(msg: &str) -> String {
    gettextrs::gettext(msg)
}

fn locale_dir() -> String {
    std::env::var("TS3LEVEL_LOCALEDIR").unwrap_or_else(|_| "/usr/share/locale".into())
}
