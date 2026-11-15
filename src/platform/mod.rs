//! Narrow Win32 bridge. Overlay operations use the Slint UI thread; Shell file
//! dialogs use their own short-lived STA thread.
mod windows;
pub use windows::*;
mod display;
pub use display::{MonitorInfo, monitors};

pub mod capture;
pub mod file_dialog;
pub mod file_search;
pub mod launcher;
pub mod power;
pub mod startup;
