#[cfg(windows)]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    if let Err(e) = res.compile() {
        eprintln!("Failed to compile Windows resource: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {}
