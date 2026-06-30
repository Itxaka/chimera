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
    type Init = (Arc<ConsoleHub>, String);
    type Input = ();
    type Output = ();
    type CommandOutput = ();

    view! {
        adw::NavigationPage {
            set_title: "Console",
        }
    }

    fn init(
        (hub, id): Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // Build child hierarchy imperatively (adw types + vte don't impl relm4 container traits).
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let term = vte::Terminal::new();
        term.set_vexpand(true);
        term.set_hexpand(true);
        style_terminal(&term);
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

        // Tail + live stream: hub bytes -> terminal.feed (on GTK thread via async_channel).
        let (tx, rx) = async_channel::unbounded::<Vec<u8>>();
        let sub_task = {
            let hub = hub.clone();
            let id = id.clone();
            crate::runtime::rt().spawn(async move {
                let tail = hub.tail(&id, 4096).await;
                if !tail.is_empty() {
                    let _ = tx.send(tail).await;
                }
                if let Some(mut sub) = hub.subscribe(&id).await {
                    loop {
                        match sub.recv().await {
                            Ok(bytes) => {
                                if tx.send(bytes).await.is_err() {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    }
                }
                // tx drops here when the task ends/aborts -> the feed loop's rx closes.
            })
        };

        // Receive on the GTK/glib main thread; feed bytes into the terminal widget.
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
