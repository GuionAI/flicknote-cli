#[cfg(target_os = "macos")]
mod append;
#[cfg(target_os = "macos")]
mod assets;
#[cfg(target_os = "macos")]
mod launch;
#[cfg(target_os = "macos")]
mod login;
#[cfg(target_os = "macos")]
mod model;
#[cfg(target_os = "macos")]
mod native_input;
#[cfg(target_os = "macos")]
mod organization;
#[cfg(all(test, target_os = "macos"))]
mod selection_tests;
#[cfg(target_os = "macos")]
mod source;
#[cfg(target_os = "macos")]
mod sync_progress;
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
    eprintln!(
        "The experimental GPUI window is macOS-only. Use flicknote daemon run for the headless application host."
    );
}
