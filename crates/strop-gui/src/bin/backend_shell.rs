#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use gpui::{
    prelude::*, px, size, App, Bounds, Context, FocusHandle, Render, Window, WindowBounds,
    WindowOptions,
};
#[cfg(target_os = "windows")]
use strop_ui_protocol::Client;

#[cfg(target_os = "windows")]
use strop_gui::bridge::{BridgeEvent, WslBridge};

#[cfg(target_os = "windows")]
struct BackendShell {
    bridge: WslBridge,
    client: Client,
    focus_handle: FocusHandle,
}
#[cfg(target_os = "windows")]
impl Render for BackendShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        while let Some(event) = self.bridge.next_event() {
            match event {
                BridgeEvent::Message(message) => {
                    let _ = self.client.apply(&message);
                }
                BridgeEvent::Closed(_) => {}
            }
        }
        // The shell owns exactly one source surface; it keeps native focus so
        // key delivery reaches the admitted-action route below.
        window.focus(&self.focus_handle, cx);
        strop_gui::surface::view_surface(self.client.view())
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event, _window, _cx| {
                let _ = strop_gui::routing::admit_key_down(&this.bridge, event);
            }))
            .on_key_up(cx.listener(|this, event, _window, _cx| {
                let _ = strop_gui::routing::admit_key_up(&this.bridge, event);
            }))
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
        &std::env::var("STROP_WSL_BACKEND").expect("STROP_WSL_BACKEND"),
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
                cx.new(|cx| BackendShell {
                    bridge,
                    client,
                    focus_handle: cx.focus_handle(),
                })
            },
        )
        .expect("native backend shell window");
        cx.activate(true);
    });
}

#[cfg(not(target_os = "windows"))]
fn main() {}
