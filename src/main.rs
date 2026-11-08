#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod animation;
mod app;
mod countdown;
mod gestures;
mod hover;
mod launcher;
mod material;
mod metrics;
mod navigation;
mod platform;
mod plugins;
mod preferences;
mod presentation;
mod services;
mod settings;
mod shape;
mod state;
mod verification;
slint::include_modules!();
fn main() {
    if let Err(error) = app::run() {
        eprintln!("Still Island: {error}");
        let _ = std::fs::write("startup-error.log", error.to_string());
        std::process::exit(1);
    }
}
