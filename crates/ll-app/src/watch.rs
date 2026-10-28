//! Opt-in folder watching (threat model: "watcher permissions, symlink policy, file-size limits,
//! and import cancellation"). Off until a person chooses a folder; the scope is fixed and shown.
//!
//! - **Permission scope:** one folder, chosen in the operating system's folder dialog, never a
//!   path typed into or sent by the web view. Broad or system locations and the app's own data
//!   folder are refused. Files are copied in; nothing in the folder is changed or deleted.
//! - **Symlinks:** never followed, as the root, inside the tree, or when a file is opened.
//! - **Limits:** [`WATCH_POLICY`] bounds depth, files per scan, and file size, and each scan reads
//!   at most [`BATCH_FILES`] files / [`BATCH_BYTES`] bytes.
//! - **Cancellation:** watching can be paused or forgotten at any time, and imports it queued are
//!   cancelled like any other import (`App::cancel_import`, `App::cancel_pending`).
//!
//! The folder is polled rather than subscribed to, so no file-notification dependency is needed
//! and a scan's cost is bounded by the policy.

use crate::{media::MAX_FILE_BYTES, App};
use ll_core::{Error, Result};
use ll_ingest::{check_folder, read_regular_file, scan_folder, FolderPolicy, FoundFile, Skipped};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub const WATCH_POLICY: FolderPolicy = FolderPolicy {
    max_depth: 3,
    max_files: 2_000,
    max_bytes: MAX_FILE_BYTES as u64,
};
/// Seconds between scans while watching.
pub const WATCH_INTERVAL_SECS: i64 = 15;
/// A file changed more recently than this may still be being written; it waits a scan.
const SETTLE_SECS: i64 = 3;
/// Files and bytes read per scan; the rest are picked up by the following scans.
pub const BATCH_FILES: usize = 10;
pub const BATCH_BYTES: u64 = 100 * 1024 * 1024;
const FILE_TYPES: [&str; 4] = ["PDF", "PNG", "JPEG", "TIFF"];

/// System and application locations a person would not keep paperwork in, refused as a watched
/// folder (or any folder inside them).
const SYSTEM_PREFIXES: [&str; 11] = [
    "/System", "/Library", "/Applications", "/usr", "/bin", "/sbin", "/etc", "/dev", "/opt",
    "/private/etc", "/private/var/db",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WatchStatus {
    pub folder: Option<String>,
    pub enabled: bool,
    /// Documents imported from the folder so far.
    pub imported: u32,
    pub last_scan: Option<WatchScan>,
    pub scope: WatchScope,
}

/// The fixed limits, so the UI can state exactly what watching may read.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WatchScope {
    pub max_depth: u32,
    pub max_file_mb: u32,
    pub max_files: u32,
    pub file_types: Vec<String>,
    pub interval_secs: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WatchScan {
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub at: i64,
    /// Supported files found in scope.
    pub found: u32,
    pub imported: u32,
    pub duplicates: u32,
    /// Files whose content is not a supported document, or that could not be read.
    pub rejected: u32,
    /// New files left for a later scan (still being written, or over the batch limit).
    pub waiting: u32,
    pub skipped_links: u32,
    pub skipped_large: u32,
    pub skipped_deep: u32,
    /// More files exist than one scan lists.
    pub truncated: bool,
    /// Why the folder could not be scanned (moved, renamed, or access denied).
    pub error: Option<String>,
}

/// What a scan needs from the workspace, taken under the service lock.
pub struct WatchPlan {
    root: PathBuf,
    known: HashMap<PathBuf, (u64, i64)>,
}

/// Files read from the folder, ready to be admitted under the service lock.
pub struct WatchBatch {
    root: PathBuf,
    files: Vec<(FoundFile, Result<Vec<u8>>)>,
    found: u32,
    waiting: u32,
    skipped: Skipped,
    truncated: bool,
    error: Option<String>,
}

impl WatchPlan {
    /// Lists the folder and reads new or changed files, within the policy. Needs no access to the
    /// workspace, so callers run it without holding the service lock.
    pub fn collect(self, now: i64) -> WatchBatch {
        let mut batch = WatchBatch {
            root: self.root.clone(),
            files: Vec::new(),
            found: 0,
            waiting: 0,
            skipped: Skipped::default(),
            truncated: false,
            error: None,
        };
        // The folder may have been replaced by a link or removed since it was chosen.
        let scan = check_folder(&self.root).and_then(|root| {
            if root != self.root {
                return Err(Error::Invalid("the watched folder has moved".into()));
            }
            scan_folder(&root, &WATCH_POLICY)
        });
        let scan = match scan {
            Ok(scan) => scan,
            Err(error) => {
                batch.error = Some(folder_problem(&error));
                return batch;
            }
        };
        batch.found = scan.files.len() as u32;
        batch.skipped = scan.skipped;
        batch.truncated = scan.truncated;
        let mut bytes_read = 0u64;
        for file in scan.files {
            let unchanged = self.known.get(&file.path) == Some(&(file.size_bytes, file.modified_unix));
            if unchanged {
                continue;
            }
            let settling = now - file.modified_unix < SETTLE_SECS;
            let over_batch = batch.files.len() >= BATCH_FILES || bytes_read + file.size_bytes > BATCH_BYTES;
            if settling || over_batch {
                batch.waiting += 1;
                continue;
            }
            bytes_read += file.size_bytes;
            let bytes = read_regular_file(&file.path, WATCH_POLICY.max_bytes);
            batch.files.push((file, bytes));
        }
        batch
    }
}

fn folder_problem(error: &Error) -> String {
    match error {
        Error::NotFound(_) => "The watched folder no longer exists. Choose it again or stop watching.".into(),
        Error::Invalid(message) => format!("The watched folder can't be scanned: {message}."),
        _ => "The watched folder can't be read. Check that Ledgerlens still has access to it.".into(),
    }
}

impl App {
    pub fn watch_status(&self) -> Result<WatchStatus> {
        let imported: u32 = self
            .store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM watch_files WHERE outcome='imported' AND document_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .map_err(Error::storage)?;
        Ok(WatchStatus {
            folder: self.setting("watch_folder")?,
            enabled: self.setting("watch_enabled")?.as_deref() == Some("1"),
            imported,
            last_scan: self.watch_last.clone(),
            scope: WatchScope {
                max_depth: WATCH_POLICY.max_depth as u32,
                max_file_mb: (WATCH_POLICY.max_bytes / (1024 * 1024)) as u32,
                max_files: WATCH_POLICY.max_files as u32,
                file_types: FILE_TYPES.iter().map(|kind| (*kind).to_string()).collect(),
                interval_secs: WATCH_INTERVAL_SECS as u32,
            },
        })
    }

    /// Starts watching `folder`. Choosing a folder is the opt-in; it must come from the person
    /// (the desktop app takes it only from the system folder dialog).
    pub fn set_watch_folder(&mut self, folder: &Path) -> Result<WatchStatus> {
        let folder = check_folder(folder)?;
        self.check_watch_scope(&folder)?;
        let display = folder
            .to_str()
            .ok_or_else(|| Error::Invalid("folder names must be valid Unicode".into()))?
            .to_string();
        if self.setting("watch_folder")?.as_deref() != Some(display.as_str()) {
            self.forget_watched_files()?;
            self.watch_last = None;
        }
        self.put_setting("watch_folder", &display)?;
        self.put_setting("watch_enabled", "1")?;
        self.watch_status()
    }

    /// Pauses or resumes watching the chosen folder.
    pub fn set_watch_enabled(&mut self, enabled: bool) -> Result<WatchStatus> {
        if enabled && self.setting("watch_folder")?.is_none() {
            return Err(Error::Invalid("choose a folder to watch first".into()));
        }
        self.put_setting("watch_enabled", if enabled { "1" } else { "0" })?;
        self.watch_status()
    }

    /// Stops watching and forgets the folder. Documents already imported stay in the inbox.
    pub fn forget_watch_folder(&mut self) -> Result<WatchStatus> {
        for key in ["watch_folder", "watch_enabled"] {
            self.store
                .conn()
                .execute("DELETE FROM app_settings WHERE key=?1", [key])
                .map_err(Error::storage)?;
        }
        self.forget_watched_files()?;
        self.watch_last = None;
        self.watch_status()
    }

    /// What the next scan needs, or `None` when watching is off.
    pub fn watch_plan(&self) -> Result<Option<WatchPlan>> {
        let status = self.watch_status()?;
        let (Some(folder), true) = (status.folder, status.enabled) else {
            return Ok(None);
        };
        let mut statement = self
            .store
            .conn()
            .prepare("SELECT path, size_bytes, modified FROM watch_files")
            .map_err(Error::storage)?;
        let known = statement
            .query_map([], |row| {
                Ok((PathBuf::from(row.get::<_, String>(0)?), (row.get::<_, i64>(1)? as u64, row.get(2)?)))
            })
            .map_err(Error::storage)?
            .collect::<rusqlite::Result<_>>()
            .map_err(Error::storage)?;
        Ok(Some(WatchPlan { root: PathBuf::from(folder), known }))
    }

    /// Admits the files a scan read and records each so it is not read again until it changes.
    /// A batch for a folder that is no longer watched is discarded.
    pub fn apply_watch(&mut self, batch: WatchBatch) -> Result<WatchScan> {
        let current = self.watch_plan()?;
        if current.as_ref().map(|plan| &plan.root) != Some(&batch.root) {
            return Ok(WatchScan::default());
        }
        let mut scan = WatchScan {
            at: ll_core::now_unix(),
            found: batch.found,
            waiting: batch.waiting,
            skipped_links: batch.skipped.symlinks,
            skipped_large: batch.skipped.too_large,
            skipped_deep: batch.skipped.too_deep,
            truncated: batch.truncated,
            error: batch.error,
            ..WatchScan::default()
        };
        for (file, bytes) in batch.files {
            let name = file.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let (outcome, document) = match bytes.and_then(|bytes| self.admit(&name, &bytes)) {
                Ok(admitted) if admitted.duplicate => {
                    scan.duplicates += 1;
                    ("duplicate", Some(admitted.document.id))
                }
                Ok(admitted) => {
                    scan.imported += 1;
                    ("imported", Some(admitted.document.id))
                }
                // Not a supported document by content, unreadable, or over the limit: remembered
                // so it is not retried until the file changes.
                Err(Error::Invalid(_) | Error::Io(_)) => {
                    scan.rejected += 1;
                    ("rejected", None)
                }
                Err(error) => return Err(error),
            };
            let path = file.path.to_str().ok_or_else(|| Error::Invalid("file names must be valid Unicode".into()));
            let Ok(path) = path else {
                continue;
            };
            self.store
                .conn()
                .execute(
                    "INSERT INTO watch_files (path, size_bytes, modified, document_id, outcome, seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(path) DO UPDATE SET size_bytes=excluded.size_bytes, modified=excluded.modified,
                        document_id=excluded.document_id, outcome=excluded.outcome, seen_at=excluded.seen_at",
                    params![path, file.size_bytes as i64, file.modified_unix, document, outcome, scan.at],
                )
                .map_err(Error::storage)?;
        }
        self.watch_last = Some(scan.clone());
        Ok(scan)
    }

    /// One scan, entirely in this thread (tests and command-line tools).
    pub fn scan_watch_folder(&mut self) -> Result<Option<WatchScan>> {
        let Some(plan) = self.watch_plan()? else {
            return Ok(None);
        };
        let batch = plan.collect(ll_core::now_unix());
        self.apply_watch(batch).map(Some)
    }

    fn check_watch_scope(&self, folder: &Path) -> Result<()> {
        let too_broad = Err(Error::Invalid(
            "that folder is too broad to watch; choose the folder your documents are saved in".into(),
        ));
        if folder.components().count() < 3 {
            return too_broad; // "/", "/Users", "/Volumes", ...
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).and_then(|home| home.canonicalize().ok())
            && (folder == home || folder.starts_with(home.join("Library")))
        {
            return too_broad;
        }
        if SYSTEM_PREFIXES.iter().any(|prefix| folder.starts_with(prefix)) {
            return too_broad;
        }
        if let Some(data) = self.data_dir.as_ref().and_then(|dir| dir.canonicalize().ok())
            && (folder.starts_with(&data) || data.starts_with(folder))
        {
            return Err(Error::Invalid("Ledgerlens can't watch the folder it keeps its own data in".into()));
        }
        Ok(())
    }

    fn forget_watched_files(&mut self) -> Result<()> {
        self.store.conn().execute("DELETE FROM watch_files", []).map_err(Error::storage)?;
        Ok(())
    }

    pub(crate) fn setting(&self, key: &str) -> Result<Option<String>> {
        self.store
            .conn()
            .query_row("SELECT value FROM app_settings WHERE key=?1", [key], |row| row.get(0))
            .optional()
            .map_err(Error::storage)
    }

    pub(crate) fn put_setting(&self, key: &str, value: &str) -> Result<()> {
        self.store
            .conn()
            .execute(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map_err(Error::storage)?;
        Ok(())
    }
}
