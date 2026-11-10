// A Windows release build is a window, not a console. Debug builds keep the
// console so `--selftest` and the screenshot messages stay visible.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod chart;
mod engine;
mod fmt;
mod presets;
mod store;
mod theme;

use std::path::{Path, PathBuf};

pub fn save_png(image: &egui::ColorImage, path: &Path) -> bool {
    let (w, h) = (image.size[0] as u32, image.size[1] as u32);
    if w == 0 || h == 0 {
        return false;
    }
    let mut buf: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
    for p in &image.pixels {
        buf.extend_from_slice(&p.to_array());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    match ::image::RgbaImage::from_raw(w, h, buf) {
        Some(img) => img.save(path).is_ok(),
        None => false,
    }
}

fn print_help() {
    println!("Inventory Simulator — 104-week cover and replenishment plan");
    println!();
    println!("Usage: inventory-simulator [--screenshot <file.png>] [--selftest]");
    println!();
    println!("  --screenshot <file.png>   draw the window, save a picture and exit");
    println!("  --selftest                print the calculation results as JSON");
    println!("  --pending-demo            draw the \"changes waiting\" state instead");
}

/// The little window/taskbar mark, drawn into the executable so a copy of the
/// app is still the app wherever it is moved.
fn window_icon() -> Option<egui::IconData> {
    const ICON: &[u8] = include_bytes!("../assets/icon.png");
    let img = ::image::load_from_memory(ICON).ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    Some(egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

fn main() -> eframe::Result<()> {
    let mut shot: Option<PathBuf> = None;
    let mut selftest = false;
    let mut pending_demo = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => {
                let p = args.next().unwrap_or_else(|| "screenshot.png".to_owned());
                shot = Some(PathBuf::from(p));
            }
            "--selftest" => selftest = true,
            "--pending-demo" => pending_demo = true,
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            _ => {}
        }
    }

    if selftest {
        println!("{}", app::selftest());
        return Ok(());
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1560.0, 976.0])
        .with_min_inner_size([1120.0, 720.0])
        .with_title("Inventory Simulator");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    if shot.is_some() {
        // Never steal focus while a check is taking its picture.
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    let shot_for_app = shot.clone();
    eframe::run_native(
        "Inventory Simulator",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::App::new(
                cc,
                shot_for_app.clone(),
                pending_demo,
            )))
        }),
    )
}
