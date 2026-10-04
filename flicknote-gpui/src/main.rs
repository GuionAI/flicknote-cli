#[cfg(target_os = "macos")]
mod assets;
#[cfg(target_os = "macos")]
mod model;
#[cfg(target_os = "macos")]
mod ui;
#[cfg(target_os = "macos")]
mod workspace;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    ui::run()
}
#[cfg(not(target_os = "macos"))]
#[allow(clippy::print_stderr)]
fn main() {
    eprintln!("The experimental GPUI window is macOS-only. Use flicknote-spike for headless.");
}
