#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use gpui::{
    div, prelude::*, px, rgb, size, text, App, Bounds, Context, Render, Window, WindowBounds,
    WindowOptions,
};
#[cfg(target_os = "windows")]
use strop_ui_protocol::Client;

#[cfg(target_os = "windows")]
use strop_gui::bridge::{BridgeEvent, WslBridge, WslSelection};

#[cfg(target_os = "windows")]
struct BackendShell {
    bridge: WslBridge,
    client: Client,
    title: String,
}

#[cfg(target_os = "windows")]

impl Render for BackendShell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        while let Some(event) = self.bridge.next_event() {
            match event {
                BridgeEvent::Message(message) => {
                    let _ = self.client.apply(&message);
                }
                BridgeEvent::Closed(_) => {}
            }
        }
        let generation = self.client.generation();
        div()
            .id("root")
            .role(gpui::Role::Application)
            .aria_label(format!("Strop backend shell, generation {generation}"))
            .size_full()
            .bg(rgb(0x11111b))
            .text_color(rgb(0xcdd6f4))
            .p_4()
            .child(text!(format!(
                "{}\nbackend generation {}",
                self.title, generation
            )))
    }
}

#[cfg(target_os = "windows")]
fn main() {
    let distro = strop_gui::wsl::WslDistro {
        name: std::env::var("STROP_WSL_DISTRO").expect("STROP_WSL_DISTRO"),
    };
    let selection = strop_gui::wsl::select(
        &distro,
        &std::env::var("STROP_WSL_USER").expect("STROP_WSL_USER"),
        &std::env::var("STROP_WSL_WORKSPACE").expect("STROP_WSL_WORKSPACE"),
        std::path::Path::new(&std::env::var_os("STROP_WSL_BACKEND").expect("STROP_WSL_BACKEND")),
    )
    .expect("explicit first-open WSL selection");
    gpui_platform::application().run(move |cx: &mut App| {
        let (bridge, backend) = WslBridge::spawn(&selection).expect("WSL backend handshake");
        let client = Client::new(&backend);
        let title = format!(
            "Strop — {} · {}",
            selection.distribution, selection.workspace
        );
        let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(title.clone().into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| BackendShell {
                    bridge,
                    client,
                    title,
                })
            },
        )
        .expect("native backend shell window");
        cx.activate(true);
    });
}

#[cfg(not(target_os = "windows"))]
fn main() {}
