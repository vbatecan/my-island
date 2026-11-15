fn main() {
    let executable = std::env::current_exe().unwrap();
    std::fs::write(executable.with_extension("started"), b"started").unwrap();
}
