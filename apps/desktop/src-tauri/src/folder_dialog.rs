//! The system folder dialog, the only way a watched folder is chosen. The web view can ask for
//! the dialog but never supplies a path, so a compromised page cannot point the watcher at a
//! folder the person did not pick.

use crate::commands::CommandError;
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};

/// Shows the dialog on the main thread and waits for the choice; `None` when cancelled.
pub async fn choose_folder<R: Runtime>(handle: &AppHandle<R>) -> Result<Option<PathBuf>, CommandError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    handle
        .run_on_main_thread(move || {
            let _ = sender.send(show());
        })
        .map_err(|_| CommandError::invalid("the folder dialog couldn't be opened"))?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv())
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| CommandError::invalid("the folder dialog closed unexpectedly"))
}

#[cfg(target_os = "macos")]
fn show() -> Option<PathBuf> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::NSString;

    let marker = MainThreadMarker::new()?;
    let panel = NSOpenPanel::openPanel(marker);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(true);
    panel.setTitle(Some(&NSString::from_str("Watch a folder")));
    panel.setMessage(Some(&NSString::from_str(
        "Ledgerlens will copy new PDFs and images from this folder into your inbox. It never changes or deletes files there.",
    )));
    panel.setPrompt(Some(&NSString::from_str("Watch Folder")));
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URLs().firstObject()?;
    let path = url.path()?;
    Some(PathBuf::from(path.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn show() -> Option<PathBuf> {
    // Folder watching is built for macOS first; other platforms get a dialog with their shell.
    None
}
