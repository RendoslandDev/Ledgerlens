use crate::AppState;
use ll_app::{
    background::Activity, watch::WatchStatus, Answer, App, DocumentDetail, DocumentSummary,
    ExportFormat, ExportPlan, ExportResult, FieldEdit, ImportOutcome, Report, SearchHit, Settings,
    SystemInfo, VendorSummary, CATEGORIES,
};
use ll_core::{DocId, ErrorCode};
use serde::Serialize;
use std::str::FromStr;
use tauri::{
    ipc::{InvokeBody, Request, Response},
    AppHandle, Manager, Runtime, State,
};

/// The error shape every command returns. Storage and I/O details stay in the log.
#[derive(Debug, Serialize, specta::Type)]
pub struct CommandError {
    code: ErrorCode,
    message: String,
}

impl From<ll_core::Error> for CommandError {
    fn from(error: ll_core::Error) -> Self {
        let code = error.code();
        let message = match code {
            ErrorCode::Storage | ErrorCode::Io => {
                eprintln!("ledgerlens: {error}");
                "Ledgerlens couldn't read or write its data. Try again, and restart the app if it keeps happening.".into()
            }
            _ => error.to_string(),
        };
        Self { code, message }
    }
}

impl CommandError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self { code: ErrorCode::Invalid, message: message.into() }
    }
}

type CommandResult<T> = Result<T, CommandError>;

fn parse_id(id: &str) -> CommandResult<DocId> {
    DocId::from_str(id).map_err(|_| CommandError::invalid("unknown document id"))
}

/// Runs `work` against the service on a blocking thread so the UI thread never waits on SQLite
/// or PDF parsing.
async fn with_app<R: Runtime, T: Send + 'static>(
    handle: AppHandle<R>,
    work: impl FnOnce(&mut App) -> ll_core::Result<T> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let mut app = state
            .app
            .lock()
            .map_err(|_| CommandError::invalid("the workspace is unavailable after an earlier failure; restart the app"))?;
        work(&mut app).map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::invalid("the request was interrupted"))?
}

#[tauri::command]
#[specta::specta]
pub async fn list_documents<R: Runtime>(handle: AppHandle<R>) -> CommandResult<Vec<DocumentSummary>> {
    with_app(handle, |app| app.list()).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_document<R: Runtime>(handle: AppHandle<R>, id: String) -> CommandResult<DocumentDetail> {
    let id = parse_id(&id)?;
    with_app(handle, move |app| app.detail(id)).await
}

/// Raw-body command: the file bytes are the request body and the name travels in the
/// `x-file-name` header (URI-encoded). Bytes never become a path on this side.
#[tauri::command]
pub async fn import_document<R: Runtime>(
    handle: AppHandle<R>,
    request: Request<'_>,
) -> CommandResult<ImportOutcome> {
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(body) => base64_body(body)?,
    };
    let name = request
        .headers()
        .get("x-file-name")
        .and_then(|value| value.to_str().ok())
        .map(percent_decode)
        .unwrap_or_else(|| "document".into());
    // Admitting is quick (check, fingerprint, store); reading happens in the background worker.
    let outcome = with_app(handle.clone(), move |app| app.admit(&name, &bytes)).await?;
    handle.state::<AppState>().background.wake();
    Ok(outcome)
}

#[tauri::command]
#[specta::specta]
pub async fn update_field<R: Runtime>(
    handle: AppHandle<R>,
    id: String,
    edit: FieldEdit,
) -> CommandResult<DocumentDetail> {
    let id = parse_id(&id)?;
    if edit.value.len() > 1_000 || edit.field.len() > 40 {
        return Err(CommandError::invalid("value is too long"));
    }
    with_app(handle, move |app| app.update_field(id, &edit)).await
}

#[tauri::command]
#[specta::specta]
pub async fn approve_document<R: Runtime>(handle: AppHandle<R>, id: String) -> CommandResult<DocumentSummary> {
    let id = parse_id(&id)?;
    with_app(handle, move |app| app.approve(id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn reprocess_document<R: Runtime>(handle: AppHandle<R>, id: String) -> CommandResult<DocumentSummary> {
    let id = parse_id(&id)?;
    let summary = with_app(handle.clone(), move |app| app.reprocess(id)).await?;
    handle.state::<AppState>().background.wake();
    Ok(summary)
}

/// What the background worker is doing. Reads a snapshot kept outside the service lock, so it
/// answers at once even while a document is being read.
#[tauri::command]
#[specta::specta]
pub fn activity(state: State<'_, AppState>) -> Activity {
    state.background.activity()
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_import<R: Runtime>(handle: AppHandle<R>, id: String) -> CommandResult<()> {
    let id = parse_id(&id)?;
    with_app(handle, move |app| app.cancel_import(id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_pending_imports<R: Runtime>(handle: AppHandle<R>) -> CommandResult<u32> {
    with_app(handle, |app| app.cancel_pending()).await
}

#[tauri::command]
#[specta::specta]
pub async fn watch_status<R: Runtime>(handle: AppHandle<R>) -> CommandResult<WatchStatus> {
    with_app(handle, |app| app.watch_status()).await
}

/// Opens the system folder dialog and starts watching the chosen folder. The path comes from the
/// dialog only: no command accepts a folder path from the web view. Returns `None` if the person
/// cancels the dialog.
#[tauri::command]
#[specta::specta]
pub async fn choose_watch_folder<R: Runtime>(handle: AppHandle<R>) -> CommandResult<Option<WatchStatus>> {
    let Some(folder) = crate::folder_dialog::choose_folder(&handle).await? else {
        return Ok(None);
    };
    let status = with_app(handle.clone(), move |app| app.set_watch_folder(&folder)).await?;
    handle.state::<AppState>().background.scan_now();
    Ok(Some(status))
}

#[tauri::command]
#[specta::specta]
pub async fn set_watch_enabled<R: Runtime>(handle: AppHandle<R>, enabled: bool) -> CommandResult<WatchStatus> {
    let status = with_app(handle.clone(), move |app| app.set_watch_enabled(enabled)).await?;
    handle.state::<AppState>().background.scan_now();
    Ok(status)
}

#[tauri::command]
#[specta::specta]
pub async fn forget_watch_folder<R: Runtime>(handle: AppHandle<R>) -> CommandResult<WatchStatus> {
    with_app(handle, |app| app.forget_watch_folder()).await
}

/// Asks the worker to scan the watched folder now instead of at the next interval.
#[tauri::command]
#[specta::specta]
pub fn scan_watch_folder(state: State<'_, AppState>) {
    state.background.scan_now();
}

#[tauri::command]
#[specta::specta]
pub async fn delete_document<R: Runtime>(handle: AppHandle<R>, id: String) -> CommandResult<()> {
    let id = parse_id(&id)?;
    with_app(handle, move |app| app.delete(id)).await
}

/// The original file as raw bytes. The UI only renders images inline; PDFs are opened with the
/// system viewer so no document content ever runs in the app's origin.
#[tauri::command]
pub async fn document_file<R: Runtime>(handle: AppHandle<R>, id: String) -> CommandResult<Response> {
    let id = parse_id(&id)?;
    let bytes = with_app(handle, move |app| app.file(id).map(|(_, bytes)| bytes)).await?;
    Ok(Response::new(bytes))
}

/// Opens a verified copy of the original in the operating system's default viewer.
#[tauri::command]
#[specta::specta]
pub async fn open_document_file<R: Runtime>(handle: AppHandle<R>, id: String) -> CommandResult<()> {
    let id = parse_id(&id)?;
    let temp = handle
        .path()
        .app_cache_dir()
        .map_err(|_| CommandError::invalid("no cache directory is available"))?;
    let path = with_app(handle, move |app| {
        let (media, bytes) = app.file(id)?;
        std::fs::create_dir_all(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o700))?;
        }
        let path = temp.join(format!("{id}.{}", media.extension()));
        std::fs::write(&path, bytes)?;
        Ok(path)
    })
    .await?;
    open_with_system(&path).map_err(|_| CommandError::invalid("no app is available to open this file"))
}

#[cfg(mobile)]
fn open_with_system(_path: &std::path::Path) -> std::io::Result<()> {
    // Phones don't let apps launch other programs; the review panel shows images inline instead.
    Err(std::io::Error::other("opening files in another app isn't available on this device"))
}

#[cfg(desktop)]
fn open_with_system(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::Command;
    // The path is built from a validated document id and a fixed extension; it is passed as a
    // single argument, never through a shell string.
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command.arg(path).spawn().map(|_| ())
}

#[tauri::command]
#[specta::specta]
pub async fn ask<R: Runtime>(handle: AppHandle<R>, question: String) -> CommandResult<Answer> {
    with_app(handle, move |app| app.ask(&question)).await
}

#[tauri::command]
#[specta::specta]
pub async fn report<R: Runtime>(handle: AppHandle<R>) -> CommandResult<Report> {
    with_app(handle, |app| app.report()).await
}

#[tauri::command]
#[specta::specta]
pub async fn export_records<R: Runtime>(
    handle: AppHandle<R>,
    format: ExportFormat,
) -> CommandResult<ExportResult> {
    let directory = export_dir(&handle)?;
    with_app(handle, move |app| app.export(format, &directory)).await
}

/// Where an export would be written and how many rows it holds; nothing is written. The UI shows
/// this and asks for confirmation first (threat model: exports need an explicit user action).
#[tauri::command]
#[specta::specta]
pub async fn export_plan<R: Runtime>(handle: AppHandle<R>, format: ExportFormat) -> CommandResult<ExportPlan> {
    let directory = export_dir(&handle)?;
    with_app(handle, move |app| app.export_plan(format, &directory)).await
}

/// Where exports go: Downloads on desktop; on a phone, the app's Documents folder, which the
/// Files app shows (iOS `UIFileSharingEnabled`).
fn export_dir<R: Runtime>(handle: &AppHandle<R>) -> CommandResult<std::path::PathBuf> {
    #[cfg(desktop)]
    let directory = handle.path().download_dir();
    #[cfg(mobile)]
    let directory = handle.path().document_dir();
    directory.map_err(|_| CommandError::invalid("no folder is available for exports"))
}

#[tauri::command]
#[specta::specta]
pub async fn approve_ready<R: Runtime>(handle: AppHandle<R>) -> CommandResult<u32> {
    with_app(handle, |app| app.approve_ready()).await
}

#[tauri::command]
#[specta::specta]
pub async fn vendors<R: Runtime>(handle: AppHandle<R>) -> CommandResult<Vec<VendorSummary>> {
    with_app(handle, |app| app.vendors()).await
}

#[tauri::command]
#[specta::specta]
pub async fn search<R: Runtime>(handle: AppHandle<R>, query: String) -> CommandResult<Vec<SearchHit>> {
    if query.len() > 200 {
        return Err(CommandError::invalid("search text is too long"));
    }
    with_app(handle, move |app| app.search(&query)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_settings<R: Runtime>(handle: AppHandle<R>) -> CommandResult<Settings> {
    with_app(handle, |app| app.settings()).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_settings<R: Runtime>(handle: AppHandle<R>, settings: Settings) -> CommandResult<Settings> {
    with_app(handle, move |app| app.update_settings(&settings)).await
}

#[tauri::command]
#[specta::specta]
pub async fn system_info<R: Runtime>(handle: AppHandle<R>) -> CommandResult<SystemInfo> {
    with_app(handle, |app| app.system_info()).await
}

#[tauri::command]
#[specta::specta]
pub fn categories(_state: State<'_, AppState>) -> Vec<String> {
    CATEGORIES.iter().map(|category| (*category).to_string()).collect()
}

/// Android's WebView can't expose request bodies, so Tauri sends every invoke there as JSON. The
/// frontend then sends the file as `{ "base64": "…" }`: one string, rather than a JSON array that
/// would cost tens of bytes per file byte to parse.
fn base64_body(body: &serde_json::Value) -> CommandResult<Vec<u8>> {
    use base64::Engine as _;
    let encoded = body
        .get("base64")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CommandError::invalid("send the file as raw bytes or base64"))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| CommandError::invalid("the file's base64 is malformed"))
}

/// Decodes `%XX` escapes from `encodeURIComponent`; invalid escapes are kept literally. The
/// result is cleaned again by `ll_app` before it is stored.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_bodies_decode() {
        let body = serde_json::json!({ "base64": "JVBERi0=" });
        assert_eq!(base64_body(&body).unwrap(), b"%PDF-");
        assert!(base64_body(&serde_json::json!({ "base64": "not base64!" })).is_err());
        assert!(base64_body(&serde_json::json!([37, 80, 68, 70])).is_err());
    }

    #[test]
    fn file_names_decode_from_headers() {
        assert_eq!(percent_decode("Re%C3%A7u%20n%C2%BA%201.pdf"), "Reçu nº 1.pdf");
        assert_eq!(percent_decode("100%.pdf"), "100%.pdf");
        assert_eq!(percent_decode("%zz%2"), "%zz%2");
    }

    #[test]
    fn storage_errors_do_not_leak_details() {
        let error = CommandError::from(ll_core::Error::Store("no such table: secret_x".into()));
        assert!(!error.message.contains("secret_x"));
        let error = CommandError::from(ll_core::Error::Invalid("dates must look like 2026-09-14".into()));
        assert!(error.message.contains("2026-09-14"));
    }
}
