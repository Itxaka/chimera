#![allow(dead_code)]

/// Install status-pill styling into the default display.
pub fn load() {
    let css = "
        .pill { padding: 2px 8px; border-radius: 8px; font-size: 0.85em; }
        .running  { background: #d1f7d1; color: #145214; }
        .stopped  { background: #e6e6e6; color: #444; }
        .paused   { background: #fff0c0; color: #6b4e00; }
        .failed   { background: #f7d1d1; color: #7a1414; }
        .creating { background: #d1e7f7; color: #0d4a73; }
        .vm-error { color: #b00; font-size: 0.85em; }
    ";
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
