//! Narrow Win32 bridge. All operations run on the Slint UI thread.
mod windows;
pub use windows::*;
mod display;
pub use display::{MonitorInfo, monitors};

pub mod capture;
