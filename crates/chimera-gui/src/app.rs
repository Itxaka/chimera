use crate::console::Console;
use crate::create_dialog::{CreateDialog, CreateOut};
use crate::dashboard::{Dashboard, DashboardOut};
use crate::detail::{Detail, DetailOut};
use adw::prelude::*;
use chimera_core::console::ConsoleHub;
use relm4::{
    adw, Component, ComponentController, ComponentParts, ComponentSender, Controller,
};
use std::sync::Arc;

#[derive(Debug)]
pub enum AppMsg {
    Open(String),
    OpenConsole(String),
    NewVm,
    Error(String),
}

pub struct App {
    hub: Arc<ConsoleHub>,
    // Kept alive so the dashboard component runtime stays active.
    #[allow(dead_code)]
    dashboard: Controller<Dashboard>,
    toasts: adw::ToastOverlay,
    nav: adw::NavigationView,
    // Kept alive so the dialog component runtime stays active while open.
    #[allow(dead_code)]
    create: Option<Controller<CreateDialog>>,
    // Kept alive so the detail component runtime stays active while pushed.
    #[allow(dead_code)]
    detail: Option<Controller<Detail>>,
    // Kept alive so the console component runtime stays active while pushed.
    #[allow(dead_code)]
    console: Option<Controller<Console>>,
}

#[relm4::component(pub)]
impl Component for App {
    type Init = Arc<ConsoleHub>;
    type Input = AppMsg;
    type Output = ();
    type CommandOutput = ();

    view! {
        adw::ApplicationWindow {
            set_title: Some("Chimera"),
            set_default_width: 1100,
            set_default_height: 720,
        }
    }

    fn init(
        hub: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Load CSS after the display is available (inside startup).
        crate::style::load();

        let dashboard = Dashboard::builder()
            .launch(hub.clone())
            .forward(sender.input_sender(), |out| match out {
                DashboardOut::Open(id) => AppMsg::Open(id),
                DashboardOut::NewVm => AppMsg::NewVm,
                DashboardOut::Error(e) => AppMsg::Error(e),
            });

        let widgets = view_output!();

        // Wire up the full widget hierarchy manually (adw types don't implement
        // relm4's container traits, so we build the tree imperatively).
        let toasts = adw::ToastOverlay::new();
        let nav = adw::NavigationView::new();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(dashboard.widget()));
        let page = adw::NavigationPage::new(&toolbar, "Chimera");
        nav.add(&page);
        toasts.set_child(Some(&nav));
        root.set_content(Some(&toasts));

        let model = App {
            hub,
            dashboard,
            toasts,
            nav,
            create: None,
            detail: None,
            console: None,
        };
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        msg: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match msg {
            AppMsg::Error(e) => self.toasts.add_toast(adw::Toast::new(&e)),
            AppMsg::Open(id) => {
                let detail = Detail::builder()
                    .launch(id.clone())
                    .forward(sender.input_sender(), |out| match out {
                        DetailOut::OpenConsole(id) => AppMsg::OpenConsole(id),
                    });
                self.nav.push(detail.widget());
                self.detail = Some(detail);
            }
            AppMsg::OpenConsole(id) => {
                let console = Console::builder()
                    .launch((self.hub.clone(), id))
                    .detach();
                self.nav.push(console.widget());
                self.console = Some(console);
            }
            AppMsg::NewVm => {
                let dlg = CreateDialog::builder()
                    .launch(())
                    .forward(sender.input_sender(), |out| match out {
                        CreateOut::Created => AppMsg::Error("VM created".into()),
                        CreateOut::Error(e) => AppMsg::Error(e),
                    });
                dlg.widget().present(Some(root));
                self.create = Some(dlg);
            }
        }
    }
}
