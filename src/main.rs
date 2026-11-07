#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod animation;
mod app;
mod material;
mod metrics;
mod platform;
mod preferences;
mod presentation;
mod services;
mod settings;
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
