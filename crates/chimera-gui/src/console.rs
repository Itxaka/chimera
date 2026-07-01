use chimera_core::console::ConsoleHub;
use relm4::{adw, Component, ComponentParts, ComponentSender};
use std::sync::Arc;
use vte::prelude::*;

/// Give the serial console a readable monospace font, a dark 16-colour palette,
/// generous scrollback, and a blinking cursor.
fn style_terminal(term: &vte::Terminal) {
    use vte::prelude::TerminalExt;
    term.set_font(Some(&gtk::pango::FontDescription::from_string(
        "monospace 11",
    )));
    term.set_scrollback_lines(10_000);
    term.set_mouse_autohide(true);
    term.set_cursor_blink_mode(vte::CursorBlinkMode::On);

    let c = |s: &str| gtk::gdk::RGBA::parse(s).expect("valid colour literal");
    let fg = c("#d8dee9");
    let bg = c("#1b1d23");
    let palette = [
        c("#21222c"),
        c("#ff5555"),
        c("#50fa7b"),
        c("#f1fa8c"),
        c("#6f8cff"),
        c("#ff79c6"),
        c("#8be9fd"),
        c("#f8f8f2"),
        c("#6272a4"),
        c("#ff6e6e"),
        c("#69ff94"),
        c("#ffffa5"),
        c("#9bb0ff"),
        c("#ff92df"),
        c("#a4ffff"),
        c("#ffffff"),
    ];
    let palette_refs: Vec<&gtk::gdk::RGBA> = palette.iter().collect();
    term.set_colors(Some(&fg), Some(&bg), &palette_refs);
}

pub struct Console {
    _hub: Arc<ConsoleHub>,
    /// The broadcast-forwarding task. Aborted on drop (when the page is popped)
    /// so the subscription + its sender end, which closes the channel and ends
    /// the GTK-side feed loop too — no per-open task/subscription leak.
    sub_task: tokio::task::JoinHandle<()>,
}

impl Drop for Console {
    fn drop(&mut self) {
        self.sub_task.abort();
    }
}

#[relm4::component(pub)]
impl Component for Console {
    type Init = (Arc<ConsoleHub>, String, String);
    type Input = ();
    type Output = ();
    type CommandOutput = ();

    view! {
        adw::NavigationPage {
            set_title: "Console",
        }
    }

    fn init(
        (hub, id, name): Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // Build child hierarchy imperatively (adw types + vte don't impl relm4 container traits).
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        // Show the VM name (id as subtitle) so multiple consoles are distinguishable.
        header.set_title_widget(Some(&adw::WindowTitle::new(&name, &id)));
        toolbar.add_top_bar(&header);
        let term = vte::Terminal::new();
        term.set_vexpand(true);
        term.set_hexpand(true);
        style_terminal(&term);

        // Clipboard shortcuts: Ctrl+Shift+C copies the selection, Ctrl+Shift+V
        // pastes (the paste is delivered as input via the `commit` signal, so it
        // reaches the guest). Mouse drag selects to PRIMARY and middle-click
        // pastes PRIMARY — both VTE/GTK defaults, no wiring needed.
        {
            let kc = gtk::EventControllerKey::new();
            let t = term.clone();
            kc.connect_key_pressed(move |_, keyval, _keycode, state| {
                let ctrl_shift = state.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                    && state.contains(gtk::gdk::ModifierType::SHIFT_MASK);
                if ctrl_shift {
                    match keyval.to_lower() {
                        gtk::gdk::Key::c => {
                            t.copy_clipboard_format(vte::Format::Text);
                            return gtk::glib::Propagation::Stop;
                        }
                        gtk::gdk::Key::v => {
                            t.paste_clipboard();
                            return gtk::glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
                gtk::glib::Propagation::Proceed
            });
            term.add_controller(kc);
        }

        toolbar.set_content(Some(&term));

        // adw::NavigationPage::set_child wraps a widget as the page body.
        use adw::prelude::NavigationPageExt;
        root.set_child(Some(&toolbar));

        // Input: typed bytes -> guest (commit signal: &Self, &str, u32).
        {
            let hub = hub.clone();
            let id = id.clone();
            term.connect_commit(move |_t, text, _size| {
                let bytes = text.as_bytes().to_vec();
                let hub = hub.clone();
                let id = id.clone();
                crate::runtime::rt().spawn(async move {
                    hub.write(&id, bytes).await;
                });
            });
        }

        // Live stream: subscribe (retry for an in-flight attach on a just-started
        // VM) and forward only NEW bytes. The captured backlog is fed separately
        // on `map` (below) so it renders even if the terminal wasn't realized when
        // the window opened.
        let (tx, rx) = async_channel::unbounded::<Vec<u8>>();
        let sub_task = {
            let hub = hub.clone();
            let id = id.clone();
            crate::runtime::rt().spawn(async move {
                let mut sub = None;
                for _ in 0..50 {
                    if let Some(s) = hub.subscribe(&id).await {
                        sub = Some(s);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                match sub {
                    Some(mut sub) => loop {
                        match sub.recv().await {
                            Ok(bytes) => {
                                if tx.send(bytes).await.is_err() {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    },
                    None => {
                        let _ = tx
                            .send(b"[console: no active session for this VM]\r\n".to_vec())
                            .await;
                    }
                }
                // tx drops here when the task ends/aborts -> the feed loop's rx closes.
            })
        };

        // Feed the captured backlog once the terminal is actually mapped (shown).
        // Feeding during init races widget realization on a fresh window; for an
        // idle guest (no new serial bytes) that left the terminal blank.
        {
            let hub = hub.clone();
            let id = id.clone();
            let term_for_tail = term.clone();
            let done = std::rc::Rc::new(std::cell::Cell::new(false));
            term.connect_map(move |_| {
                if done.replace(true) {
                    return;
                }
                let hub = hub.clone();
                let id = id.clone();
                let term = term_for_tail.clone();
                relm4::spawn_local(async move {
                    let tail = hub.tail(&id, 65536).await;
                    if !tail.is_empty() {
                        term.feed(&tail);
                    }
                });
            });
        }

        // Live feed loop: new bytes -> terminal widget (on the glib main thread).
        // Ends when `sub_task` drops its `tx` (task completes or is aborted on drop).
        relm4::spawn_local(async move {
            while let Ok(bytes) = rx.recv().await {
                term.feed(&bytes);
            }
        });

        let model = Console {
            _hub: hub,
            sub_task,
        };
        ComponentParts { model, widgets }
    }
}
