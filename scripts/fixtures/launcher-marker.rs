//! Owned launch fixture: writes only the marker explicitly supplied by its probe.
#![windows_subsystem = "windows"]
fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 2 {
        return;
    }
    let Ok(directory) = std::env::current_dir() else {
        return;
    };
    let result = format!(
        "argument={}\ncwd={}\npid={}\n",
        arguments[1],
        directory.display(),
        std::process::id()
    );
    let _ = std::fs::write(&arguments[0], result);
}
