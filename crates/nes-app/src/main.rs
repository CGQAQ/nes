//! NES emulator and debugger: desktop entry point (and the web entry point
//! when built for wasm32).

// Release builds on Windows are GUI apps: no console window behind them.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app;
mod audio;
mod console;
mod debugger;
mod input;
mod platform;
mod session;
mod settings;
mod windows;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // nes [ROM] [--debug] [--viewers] [--fresh] [--size WxH] [--capture out.png [--capture-after FRAMES]]
    let mut rom = None;
    let mut debug = false;
    let mut viewers = false;
    let mut fixed_size = false;
    let mut fresh = false;
    let mut capture = None;
    let mut capture_after = 180;
    let mut size = [820.0f32, 720.0];
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--debug" => debug = true,
            "--viewers" => viewers = true,
            "--fresh" => fresh = true,
            "--capture" => capture = args.next(),
            "--capture-after" => {
                capture_after = args.next().and_then(|v| v.parse().ok()).unwrap_or(180)
            }
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|v| v.split_once('x')) {
                    size = [w.parse().unwrap_or(size[0]), h.parse().unwrap_or(size[1])];
                    fixed_size = true;
                }
            }
            _ => rom = Some(a),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("nes")
            .with_inner_size(size)
            .with_min_inner_size([320.0, 280.0])
            .with_drag_and_drop(true),
        // An explicit size wins over the remembered window geometry.
        persist_window: !fixed_size,
        ..Default::default()
    };
    eframe::run_native(
        "nes",
        options,
        Box::new(move |cc| {
            if fixed_size {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
            }
            let mut app = app::NesApp::new(cc);
            if fresh {
                app.reset_layout(&cc.egui_ctx);
            }
            if let Some(path) = rom {
                app.open_path(&path);
            }
            app.show_debugger(debug, viewers);
            if let Some(path) = capture {
                app.set_capture(path.into(), capture_after);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("no document");
        let canvas = document
            .get_element_by_id("nes_canvas")
            .expect("missing #nes_canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("#nes_canvas is not a canvas");
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(app::NesApp::new(cc)))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(()) => loading.remove(),
                Err(e) => loading.set_inner_html(&format!("<p>Failed to start: {e:?}</p>")),
            }
        }
    });
}
