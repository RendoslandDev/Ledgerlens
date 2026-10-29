#![forbid(unsafe_code)]
//! The application service the desktop shell calls. It owns the store and the document blob
//! directory, and composes ingest -> text -> extraction -> review -> query/report/export.
//!
//! Every value crossing this boundary is validated here; the UI is treated as untrusted input.

pub mod background;
pub mod intent;
pub mod media;
pub mod text;
pub mod watch;

use intent::Intent;
use ll_core::{
    Config, Confidence, Currency, Date, DocId, DocKind, DocStatus, Error, ErrorCode, ExtractedField,
    FieldName, FieldValue, Money, Result, Tier,
};
use ll_export::{ExportTarget, OfxStatement};
use ll_extract::{
    needs_review, notice_deadline, required_fields, totals_consistent, validate_for, Pipeline, Severity,
};
use ll_jobs::{JobState, Queue};
use ll_query::{Breakdown, SpendQuery};
use ll_store::{run_migrations, Migration, Store};
use media::{clean_file_name, sniff, MediaType};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};
use text::{TextOutcome, TextSource};

/// Categories a person can assign. A closed set keeps reports and queries well-formed.
pub const CATEGORIES: [&str; 12] = [
    "Software",
    "Services",
    "Travel",
    "Meals",
    "Supplies",
    "Equipment",
    "Communications",
    "Utilities",
    "Rent",
    "Fees",
    "Taxes",
    "Other",
];
const MAX_TEXT_FIELD_CHARS: usize = 200;
/// Integer fields cross to TypeScript as `number` (see the `specta(type = specta_typescript::Number)` attributes), which
/// is exact up to 2^53 - 1. Amounts are minor units of one document, so this bound (about 90
/// trillion in a two-decimal currency) is enforced on entry rather than silently rounded later.
pub const MAX_SAFE_MINOR: i64 = (1 << 53) - 1;

/// Reading a document runs as a durable `ll-jobs` job: admitted files survive a crash or restart
/// and are read in the background, one at a time.
const READ_JOB: &str = "read-document";
/// Attempts before a document that keeps failing (storage errors, not unreadable content) is
/// marked failed.
const READ_ATTEMPTS: u32 = 3;
const READ_BACKOFF_SECS: u64 = 5;
/// Longer than the worst case of a text-layer parse plus OCR (see `text`), so a live read is
/// never reclaimed by a second worker.
const READ_LEASE_SECS: i64 = 300;

const APP_MIGRATIONS: &[Migration] = &[
Migration {
    version: 1,
    name: "imported files",
    sql: "
        CREATE TABLE app_files (
            document_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
            file_name TEXT NOT NULL,
            media_type TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            text TEXT,
            note TEXT,
            imported_at INTEGER NOT NULL
        );
    ",
},
Migration {
    version: 2,
    name: "text source",
    sql: "ALTER TABLE app_files ADD COLUMN text_source TEXT;",
},
Migration {
    version: 3,
    name: "settings and full-text search",
    sql: "
        CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE VIRTUAL TABLE document_search USING fts5(
            document_id UNINDEXED, file_name, text, tokenize = 'unicode61 remove_diacritics 2'
        );
        INSERT INTO document_search (document_id, file_name, text)
            SELECT document_id, file_name, coalesce(text, '') FROM app_files;
    ",
},
Migration {
    version: 4,
    name: "watched folder",
    sql: "
        CREATE TABLE watch_files (
            path TEXT PRIMARY KEY,
            size_bytes INTEGER NOT NULL,
            modified INTEGER NOT NULL,
            document_id TEXT,
            outcome TEXT NOT NULL,
            seen_at INTEGER NOT NULL
        );
    ",
},
Migration {
    version: 5,
    name: "document type chosen by a person",
    sql: "ALTER TABLE app_files ADD COLUMN kind_override TEXT;",
}];

// ---- boundary types --------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub id: String,
    pub file_name: String,
    pub media_type: MediaType,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub size_bytes: u64,
    pub kind: DocKind,
    pub status: DocStatus,
    pub vendor: Option<String>,
    pub invoice_number: Option<String>,
    pub date: Option<String>,
    pub currency: Option<String>,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub total_minor: Option<i64>,
    pub category: Option<String>,
    /// Lowest confidence across the required fields, 0..=100; a missing field counts as 0.
    pub confidence: u8,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub imported_at: i64,
    /// An earlier document this one appears to repeat (same vendor and number, or same vendor,
    /// date, and total).
    pub duplicate_of: Option<String>,
    /// Where the text came from: `text_layer`, `ocr`, or `null` when nothing could be read.
    pub text_source: Option<TextSource>,
    /// Why the document could not be read automatically, or what to check after OCR.
    pub note: Option<String>,
    /// For a contract with an end date and a notice period: the last day to give notice.
    pub notice_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FieldView {
    pub name: FieldName,
    /// Editable form: ISO date, plain decimal amount, or text.
    pub value: String,
    pub confidence: u8,
    pub tier: Tier,
    pub needs_review: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct LineItemView {
    pub description: String,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub quantity: f64,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub unit_minor: i64,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub amount_minor: i64,
    pub currency: String,
}

/// One statement row. `amount_minor` is signed: money in is positive, money out negative.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct TransactionView {
    pub date: String,
    pub description: String,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub amount_minor: i64,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub balance_minor: Option<i64>,
    pub currency: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct IssueView {
    pub severity: IssueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DocumentDetail {
    pub summary: DocumentSummary,
    pub fields: Vec<FieldView>,
    pub line_items: Vec<LineItemView>,
    /// Statement rows, in the order printed.
    pub transactions: Vec<TransactionView>,
    pub issues: Vec<IssueView>,
    pub text: Option<String>,
    /// Changes people made to fields, oldest first.
    pub history: Vec<CorrectionView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CorrectionView {
    pub field: FieldName,
    pub old: Option<String>,
    /// Who produced the old value: `rules`, `human`, ...
    pub old_source: Option<Tier>,
    pub new: Option<String>,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub changed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub workspace_name: String,
    pub default_currency: String,
    /// Fields below this confidence (0.5..=0.99) are sent to review.
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub review_threshold: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub version: String,
    /// `macos`, `ios`, `android`, `windows`, or `linux`.
    pub platform: String,
    /// Whether a watched folder can be chosen here (it needs the macOS folder dialog).
    pub folder_watching: bool,
    pub ocr_available: bool,
    pub ocr_engine: Option<String>,
    pub data_dir: Option<String>,
    pub documents: u32,
    /// Statement and contract reading and OFX export are available (Ledgerlens Pro).
    pub pro: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct VendorSummary {
    pub name: String,
    pub documents: u32,
    pub needs_review: u32,
    /// Reviewed spend per currency.
    pub totals: Vec<AmountRow>,
    pub last_date: Option<String>,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: String,
    /// Matching excerpt; matched terms are wrapped in U+0001 ... U+0002 markers.
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub document: DocumentSummary,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FieldEdit {
    /// A field name (`vendor`, `doc_date`, `total`, ...) or `kind`.
    pub field: String,
    /// The new value; empty clears the field.
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AmountRow {
    pub label: Option<String>,
    pub currency: String,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub total_minor: i64,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub answer: String,
    pub rows: Vec<AmountRow>,
    /// The parameterised SQL that produced the rows, when the answer came from a query.
    pub sql: Option<String>,
    pub document_ids: Vec<String>,
    /// Records excluded because they are still waiting for review.
    pub pending_review: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StatusCounts {
    pub total: u32,
    pub queued: u32,
    pub needs_review: u32,
    pub extracted: u32,
    pub approved: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub counts: StatusCounts,
    pub by_currency: Vec<AmountRow>,
    pub by_category: Vec<AmountRow>,
    pub by_vendor: Vec<AmountRow>,
    pub by_month: Vec<AmountRow>,
    /// Share of machine-read documents whose required fields all reached the review threshold.
    pub high_confidence_percent: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Canonical,
    Quickbooks,
    Xero,
    /// Reviewed bank statements as OFX, one file per account, for QuickBooks Online and Xero.
    Ofx,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    /// Every file written: one for a CSV, one per account for OFX.
    pub paths: Vec<String>,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub rows: usize,
}

/// What an export would write, shown to the person before anything is written.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    pub paths: Vec<String>,
    /// Records for a CSV; transactions for OFX.
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub rows: usize,
}

// ---- service ---------------------------------------------------------------------------------

pub struct App {
    store: Store,
    blobs: PathBuf,
    data_dir: Option<PathBuf>,
    config: Config,
    pipeline: Pipeline,
    review_threshold: f32,
    default_currency: Currency,
    pinned_today: Option<Date>,
    /// The latest folder scan, for the settings page; not persisted.
    watch_last: Option<watch::WatchScan>,
    /// Statement and contract reading and OFX export (Ledgerlens Pro). On in builds with the
    /// `pro` feature; off everywhere else until the paid version ships.
    pro: bool,
}

const DEFAULT_WORKSPACE_NAME: &str = "My workspace";

impl App {
    /// Opens (creating if needed) the database and blob directory under `data_dir`.
    pub fn open(data_dir: &Path) -> Result<Self> {
        create_private_dir(data_dir)?;
        let blobs = data_dir.join("blobs");
        create_private_dir(&blobs)?;
        let database = data_dir.join("ledgerlens.db");
        let store = Store::open(&database)?;
        restrict_file(&database)?;
        let mut app = Self::with_store(store, blobs, Config::with_data_dir(data_dir.to_path_buf()))?;
        app.data_dir = Some(data_dir.to_path_buf());
        Ok(app)
    }

    pub fn with_store(mut store: Store, blobs: PathBuf, config: Config) -> Result<Self> {
        run_migrations(store.conn_mut(), "app", APP_MIGRATIONS)?;
        store
            .conn()
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(Error::storage)?;
        let mut app = Self {
            store,
            blobs,
            data_dir: None,
            pipeline: Pipeline::new(&config)?.with_pro_kinds(cfg!(feature = "pro")),
            review_threshold: config.pipeline.escalate_below,
            default_currency: Currency::new(&config.pipeline.default_currency)?,
            config,
            pinned_today: None,
            watch_last: None,
            pro: cfg!(feature = "pro"),
        };
        let settings = app.settings()?;
        app.apply_settings(&settings)?;
        app.reconcile_duplicates()?;
        app.recover_reads()?;
        Ok(app)
    }

    /// Sends suspected duplicates that are still "Ready" back to review, so totals never count
    /// the same receipt twice (covers data imported before duplicate checks existed).
    fn reconcile_duplicates(&mut self) -> Result<()> {
        for document in self.store.documents()? {
            if document.status == DocStatus::Extracted && self.duplicate_of(document.id)?.is_some() {
                self.store.set_status(document.id, DocStatus::NeedsReview)?;
            }
        }
        Ok(())
    }

    // -- settings ----------------------------------------------------------------------------

    pub fn settings(&self) -> Result<Settings> {
        let get = |key: &str| -> Result<Option<String>> {
            self.store
                .conn()
                .query_row("SELECT value FROM app_settings WHERE key=?1", [key], |row| row.get(0))
                .optional()
                .map_err(Error::storage)
        };
        Ok(Settings {
            workspace_name: get("workspace_name")?.unwrap_or_else(|| DEFAULT_WORKSPACE_NAME.into()),
            default_currency: get("default_currency")?
                .unwrap_or_else(|| self.config.pipeline.default_currency.clone()),
            review_threshold: get("review_threshold")?
                .and_then(|value| value.parse().ok())
                .unwrap_or(self.config.pipeline.escalate_below),
        })
    }

    /// Validates and saves settings, then rebuilds the extraction pipeline with them. Existing
    /// documents keep their status until they are read again or edited.
    pub fn update_settings(&mut self, settings: &Settings) -> Result<Settings> {
        let name = settings.workspace_name.trim();
        if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
            return Err(Error::Invalid("workspace name must be 1 to 60 characters".into()));
        }
        let currency = Currency::new(&settings.default_currency)?;
        if !(0.5..=0.99).contains(&settings.review_threshold) {
            return Err(Error::Invalid("review threshold must be between 50% and 99%".into()));
        }
        let clean = Settings {
            workspace_name: name.into(),
            default_currency: currency.code().into(),
            review_threshold: (settings.review_threshold * 100.0).round() / 100.0,
        };
        self.apply_settings(&clean)?;
        for (key, value) in [
            ("workspace_name", clean.workspace_name.clone()),
            ("default_currency", clean.default_currency.clone()),
            ("review_threshold", clean.review_threshold.to_string()),
        ] {
            self.store
                .conn()
                .execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![key, value],
                )
                .map_err(Error::storage)?;
        }
        Ok(clean)
    }

    fn apply_settings(&mut self, settings: &Settings) -> Result<()> {
        let mut config = self.config.clone();
        config.pipeline.default_currency = settings.default_currency.clone();
        config.pipeline.escalate_below = settings.review_threshold;
        let mut pipeline = Pipeline::new(&config)?.with_pro_kinds(self.pro);
        if let Some(today) = self.pinned_today {
            pipeline = pipeline.with_today(today);
        }
        self.pipeline = pipeline;
        self.review_threshold = settings.review_threshold;
        self.default_currency = Currency::new(&settings.default_currency)?;
        self.config = config;
        Ok(())
    }

    pub fn system_info(&self) -> Result<SystemInfo> {
        Ok(SystemInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            platform: std::env::consts::OS.into(),
            folder_watching: cfg!(target_os = "macos"),
            ocr_available: ll_vision::is_available(),
            ocr_engine: ll_vision::is_available().then(|| "Apple Vision (on-device)".into()),
            data_dir: self.data_dir.as_ref().map(|dir| dir.display().to_string()),
            documents: self.store.documents()?.len() as u32,
            pro: self.pro,
        })
    }

    /// Turns Ledgerlens Pro (statements, contracts, OFX export) on or off, whatever the build.
    pub fn with_pro(mut self, on: bool) -> Self {
        self.pro = on;
        self.pipeline = self.pipeline.with_pro_kinds(on);
        self
    }

    /// The kind whose rules apply to a document of `kind` (see [`Pipeline::rules_kind`]).
    fn rules_kind(&self, kind: DocKind) -> DocKind {
        self.pipeline.rules_kind(kind)
    }

    /// Pins "today" for extraction validation and date questions (tests, replays).
    pub fn with_today(mut self, today: Date) -> Self {
        self.pipeline = self.pipeline.with_today(today);
        self.pinned_today = Some(today);
        self
    }

    fn today(&self) -> Date {
        self.pinned_today
            .unwrap_or_else(|| ll_extract::pipeline::date_from_unix(ll_core::now_unix()))
    }

    // -- import ------------------------------------------------------------------------------

    /// Imports and reads a document before returning: [`App::admit`] followed by
    /// [`App::run_pending`]. The desktop app admits instead and lets the background worker read.
    pub fn import(&mut self, file_name: &str, bytes: &[u8]) -> Result<ImportOutcome> {
        let outcome = self.admit(file_name, bytes)?;
        if outcome.duplicate {
            return Ok(outcome);
        }
        self.run_pending()?;
        let id = DocId::from_str(&outcome.document.id).map_err(|e| Error::Invalid(e.to_string()))?;
        Ok(ImportOutcome { document: self.summary(id)?, duplicate: false })
    }

    /// Checks and stores a document, then queues it to be read. Returns at once with the document
    /// `queued`; a file already in the workspace returns the existing document instead.
    pub fn admit(&mut self, file_name: &str, bytes: &[u8]) -> Result<ImportOutcome> {
        let media = sniff(bytes)?;
        let file_name = clean_file_name(file_name);
        let sha256 = hex::encode(Sha256::digest(bytes));
        let (document, inserted) =
            self.store
                .insert_document(&file_name, &sha256, default_kind(media))?;
        if !inserted {
            return Ok(ImportOutcome { document: self.summary(document.id)?, duplicate: true });
        }

        let stored = self.write_blob(&sha256, media, bytes).and_then(|()| {
            self.store
                .conn()
                .execute(
                    "INSERT INTO app_files (document_id, file_name, media_type, size_bytes, imported_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![document.id.to_string(), file_name, media.mime(), bytes.len() as i64, ll_core::now_unix()],
                )
                .map_err(Error::storage)?;
            self.set_text(document.id, None, None, None)?;
            self.queue_read(document.id)
        });
        if let Err(error) = stored {
            let _ = self.store.delete_document(document.id);
            return Err(error);
        }
        Ok(ImportOutcome { document: self.summary(document.id)?, duplicate: false })
    }

    fn queue(&self) -> Queue<'_> {
        Queue::attach(&self.store, READ_ATTEMPTS, READ_BACKOFF_SECS)
    }

    fn queue_read(&mut self, id: DocId) -> Result<()> {
        let key = read_key(id);
        self.queue().enqueue(READ_JOB, &id.to_string(), Some(&key), ll_core::now_unix())?;
        self.store.set_status(id, DocStatus::Queued)
    }

    /// Claims the next document due to be read and loads its file. Reading itself happens in
    /// [`ReadJob::read`], which needs no access to the workspace, so a caller can release its
    /// lock on the service while OCR runs.
    pub fn claim_read(&mut self) -> Result<Option<ReadJob>> {
        loop {
            let now = ll_core::now_unix();
            let Some(job) = self.queue().claim(now, READ_LEASE_SECS)? else {
                return Ok(None);
            };
            let loaded = DocId::from_str(&job.payload)
                .map_err(|e| Error::Invalid(e.to_string()))
                .and_then(|id| {
                    let (media, _) = self.file_info(id)?;
                    let bytes = self.read_blob(id)?;
                    self.store.set_status(id, DocStatus::Processing)?;
                    Ok((id, media, bytes))
                });
            match loaded {
                Ok((document, media, bytes)) => {
                    return Ok(Some(ReadJob { job, document, media, bytes }));
                }
                // Deleted before its turn: nothing left to read.
                Err(Error::NotFound(_)) => {
                    self.queue().complete(job.id)?;
                }
                Err(error) => self.job_failed(&job, &error)?,
            }
        }
    }

    /// Stores what [`ReadJob::read`] found. Returns the document, or `None` when the import was
    /// cancelled or deleted while it was being read (the result is then discarded).
    pub fn finish_read(&mut self, job: ReadJob, outcome: TextRead) -> Result<Option<DocId>> {
        if !self.queue().is_current(&job.job)? || self.store.get_document(job.document).is_err() {
            return Ok(None);
        }
        match self.apply_text(job.document, job.media, outcome.0) {
            Ok(()) => {
                self.queue().complete(job.job.id)?;
                Ok(Some(job.document))
            }
            Err(error) => {
                self.job_failed(&job.job, &error)?;
                Ok(Some(job.document))
            }
        }
    }

    /// Reads every document that is due, in this thread. Returns how many were read.
    pub fn run_pending(&mut self) -> Result<u32> {
        let mut read = 0;
        while let Some(job) = self.claim_read()? {
            let outcome = job.read();
            if self.finish_read(job, outcome)?.is_some() {
                read += 1;
            }
        }
        Ok(read)
    }

    /// Records a failed attempt. Only the error's code is kept: messages can quote document
    /// content, and the jobs table outlives the document.
    fn job_failed(&mut self, job: &ll_jobs::Job, error: &Error) -> Result<()> {
        let now = ll_core::now_unix();
        // Storage and I/O hiccups are worth retrying; a file that no longer matches its
        // fingerprint or an invalid record will fail the same way every time.
        let state = if error.is_retryable() || error.code() == ErrorCode::Storage {
            self.queue().fail(job, error.code().as_str(), now)?
        } else {
            self.queue().bury(job, error.code().as_str(), now)?
        };
        let Ok(id) = DocId::from_str(&job.payload) else { return Ok(()) };
        if self.store.get_document(id).is_err() {
            return Ok(());
        }
        match state {
            JobState::Dead => {
                let note = if error.code() == ErrorCode::Validation {
                    "The stored copy of this file is damaged. Delete it and import the original again.".to_string()
                } else {
                    format!(
                        "Ledgerlens couldn't finish reading this file after {} attempts. Use Read again to retry.",
                        job.attempts
                    )
                };
                self.store.conn()
                    .execute("UPDATE app_files SET note=?2 WHERE document_id=?1", params![id.to_string(), note])
                    .map_err(Error::storage)?;
                self.store.set_status(id, DocStatus::Failed)
            }
            JobState::Queued => self.store.set_status(id, DocStatus::Queued),
            _ => Ok(()),
        }
    }

    fn apply_text(&mut self, id: DocId, media: MediaType, outcome: TextOutcome) -> Result<()> {
        match outcome {
            TextOutcome::Text(text, source) => {
                let note = (source == TextSource::Ocr).then_some(
                    "Read with on-device OCR. Check amounts and dates against the original.",
                );
                self.set_text(id, Some(&text), note, Some(source))?;
                self.extract_text(id, &text, source)?;
            }
            TextOutcome::NoText => {
                let note = if ll_vision::is_available() {
                    "No text was found in this document. Enter the details by hand."
                } else if media.is_image() {
                    "Images need OCR, which this system doesn't have. Enter the details by hand."
                } else {
                    "This PDF has no text layer (it's probably a scan). Enter the details by hand."
                };
                self.set_text(id, None, Some(note), None)?;
                self.store.set_status(id, DocStatus::NeedsReview)?;
            }
            TextOutcome::Unreadable(reason) => {
                self.set_text(id, None, Some(&capitalize_first(&reason)), None)?;
                self.store.set_status(id, DocStatus::Failed)?;
            }
        }
        Ok(())
    }

    /// Runs extraction over a document's text and stores the result. A type a person chose wins
    /// over the classifier.
    fn extract_text(&mut self, id: DocId, text: &str, source: TextSource) -> Result<()> {
        let chosen: Option<String> = self
            .store
            .conn()
            .query_row("SELECT kind_override FROM app_files WHERE document_id=?1", [id.to_string()], |row| row.get(0))
            .map_err(Error::storage)?;
        let extraction = match chosen.as_deref().and_then(|kind| DocKind::from_str(kind).ok()) {
            Some(kind) => self.pipeline.run_as(text, kind),
            None => self.pipeline.run(text),
        };
        let status = self.pipeline.persist(&mut self.store, id, &extraction)?;
        // OCR can misread digits. Only skip review when the amounts corroborate each other: on
        // an invoice subtotal + tax = total; a statement that reached "extracted" has already
        // reconciled its balances. Contracts have no such check, so a person confirms them.
        let confirmed = match self.rules_kind(extraction.kind) {
            DocKind::Statement => true,
            DocKind::Contract => false,
            DocKind::Invoice | DocKind::Receipt | DocKind::Other => totals_consistent(&self.store.fields(id)?),
        };
        let unconfirmed_ocr = source == TextSource::Ocr && !confirmed;
        if status == DocStatus::Extracted && (unconfirmed_ocr || self.duplicate_of(id)?.is_some()) {
            self.store.set_status(id, DocStatus::NeedsReview)?;
        }
        Ok(())
    }

    /// Queues the document to be read again. Human-entered values are kept; approved documents
    /// are left as they are.
    pub fn reprocess(&mut self, id: DocId) -> Result<DocumentSummary> {
        let document = self.store.get_document(id)?;
        if document.status != DocStatus::Approved {
            self.queue_read(id)?;
        }
        self.summary(id)
    }

    /// Stops an import that has not been read yet. A document that was never read is removed;
    /// one queued to be read again keeps its values and goes back to review.
    pub fn cancel_import(&mut self, id: DocId) -> Result<()> {
        let document = self.store.get_document(id)?;
        if !matches!(document.status, DocStatus::Queued | DocStatus::Processing) {
            return Err(Error::Invalid("only documents waiting to be read can be cancelled".into()));
        }
        if let Some((job, _)) = self.queue().find(&read_key(id))? {
            self.queue().cancel(job)?;
        }
        let never_read: bool = self
            .store
            .conn()
            .query_row(
                "SELECT text IS NULL AND note IS NULL AND text_source IS NULL FROM app_files WHERE document_id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .map_err(Error::storage)?;
        if never_read && self.store.fields(id)?.is_empty() {
            self.delete(id)
        } else {
            self.store.set_status(id, DocStatus::NeedsReview)
        }
    }

    /// Cancels every import still waiting to be read. Returns how many were cancelled.
    pub fn cancel_pending(&mut self) -> Result<u32> {
        let pending: Vec<DocId> = self
            .store
            .documents()?
            .into_iter()
            .filter(|document| matches!(document.status, DocStatus::Queued | DocStatus::Processing))
            .map(|document| document.id)
            .collect();
        for id in &pending {
            self.cancel_import(*id)?;
        }
        Ok(pending.len() as u32)
    }

    /// Documents waiting to be read and being read, for progress display.
    pub fn pending_reads(&self) -> Result<(u32, u32)> {
        let documents = self.store.documents()?;
        let count = |status: DocStatus| documents.iter().filter(|d| d.status == status).count() as u32;
        Ok((count(DocStatus::Queued), count(DocStatus::Processing)))
    }

    /// When the queue next has work, as a Unix time (`None` when it is empty).
    pub fn next_read_at(&self) -> Result<Option<i64>> {
        self.queue().next_run_at()
    }

    /// Crash recovery: reads interrupted by a crash or quit are queued again, and documents left
    /// mid-read by versions that read inline get a job.
    fn recover_reads(&mut self) -> Result<()> {
        Queue::migrate(&mut self.store)?;
        self.queue().recover(ll_core::now_unix())?;
        for document in self.store.documents()? {
            if matches!(document.status, DocStatus::Queued | DocStatus::Processing) {
                let active = matches!(
                    self.queue().find(&read_key(document.id))?,
                    Some((_, JobState::Queued | JobState::Running))
                );
                if !active {
                    self.queue_read(document.id)?;
                } else if document.status == DocStatus::Processing {
                    self.store.set_status(document.id, DocStatus::Queued)?;
                }
            }
        }
        Ok(())
    }

    pub fn delete(&mut self, id: DocId) -> Result<()> {
        let document = self.store.get_document(id)?;
        let (media, _) = self.file_info(id)?;
        if let Some((job, _)) = self.queue().find(&read_key(id))? {
            self.queue().cancel(job)?;
        }
        self.store
            .conn()
            // A watched file the person removed is not imported again unless it changes.
            .execute(
                "UPDATE watch_files SET document_id=NULL, outcome='removed' WHERE document_id=?1",
                [id.to_string()],
            )
            .map_err(Error::storage)?;
        self.store.delete_document(id)?;
        self.store
            .conn()
            .execute("DELETE FROM document_search WHERE document_id=?1", [id.to_string()])
            .map_err(Error::storage)?;
        let path = self.blob_path(&document.sha256, media)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    // -- reading -----------------------------------------------------------------------------

    pub fn list(&self) -> Result<Vec<DocumentSummary>> {
        self.store
            .documents()?
            .into_iter()
            .map(|document| self.summary(document.id))
            .collect()
    }

    pub fn summary(&self, id: DocId) -> Result<DocumentSummary> {
        let document = self.store.get_document(id)?;
        type FileRow = (String, String, i64, Option<String>, i64, Option<String>);
        let (file_name, media, size, note, imported_at, text_source): FileRow = self
            .store
            .conn()
            .query_row(
                "SELECT file_name, media_type, size_bytes, note, imported_at, text_source
                 FROM app_files WHERE document_id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .map_err(Error::storage)?;
        let fields = self.store.fields(id)?;
        let text = |name: FieldName| {
            fields.iter().find(|f| f.name == name).and_then(|f| f.value.as_text()).map(str::to_string)
        };
        let money = |name: FieldName| fields.iter().find(|f| f.name == name).and_then(|f| f.value.as_money());
        let date = |name: FieldName| fields.iter().find(|f| f.name == name).and_then(|f| f.value.as_date());
        // The headline amount and date of each kind: a statement's closing balance and period end.
        let rules = self.rules_kind(document.kind);
        let (total, when) = match rules {
            DocKind::Statement => (money(FieldName::ClosingBalance), date(FieldName::PeriodEnd).or(date(FieldName::DocDate))),
            _ => (money(FieldName::Total), date(FieldName::DocDate)),
        };
        let notice_by = match (rules, date(FieldName::EndDate), text(FieldName::NoticePeriod)) {
            (DocKind::Contract, Some(end), Some(notice)) => notice_deadline(end, &notice).map(|d| d.to_string()),
            _ => None,
        };
        let confidence = required_fields(rules)
            .iter()
            .map(|name| fields.iter().find(|f| f.name == *name).map_or(0.0, effective_confidence))
            .fold(1.0f32, f32::min);
        Ok(DocumentSummary {
            id: id.to_string(),
            file_name,
            media_type: MediaType::from_mime(&media)?,
            size_bytes: size.max(0) as u64,
            kind: document.kind,
            status: document.status,
            vendor: match rules {
                DocKind::Contract => text(FieldName::Parties),
                _ => text(FieldName::Vendor),
            },
            invoice_number: text(FieldName::InvoiceNumber),
            date: when.map(|date| date.to_string()),
            currency: text(FieldName::Currency).or_else(|| total.map(|m| m.currency.code().to_string())),
            total_minor: total.map(|m| m.minor),
            category: text(FieldName::Category),
            confidence: (confidence * 100.0).round() as u8,
            imported_at,
            duplicate_of: self.duplicate_of(id)?,
            text_source: text_source.as_deref().and_then(TextSource::from_stored),
            note,
            notice_by,
        })
    }

    pub fn detail(&self, id: DocId) -> Result<DocumentDetail> {
        let summary = self.summary(id)?;
        let fields = self.store.fields(id)?;
        let items = self.store.line_items(id)?;
        let transactions = self.store.transactions(id)?;
        let rules = self.rules_kind(summary.kind);
        let review = needs_review(&fields, required_fields(rules), self.review_threshold);
        let mut issues: Vec<IssueView> = Vec::new();
        if let Some(original) = &summary.duplicate_of {
            let other = self.summary(DocId::from_str(original).map_err(|e| Error::Invalid(e.to_string()))?)?;
            issues.push(IssueView {
                severity: IssueSeverity::Warning,
                message: format!(
                    "This looks like a duplicate of {} ({}). Delete one of them, or approve both if they are separate charges.",
                    other.vendor.as_deref().unwrap_or(&other.file_name),
                    other.file_name
                ),
            });
        }
        issues.extend(validate_for(rules, &fields, &items, &transactions, self.today())
            .into_iter()
            .map(|issue| IssueView {
                severity: match issue.severity {
                    Severity::Error => IssueSeverity::Error,
                    Severity::Warning => IssueSeverity::Warning,
                },
                message: capitalize_first(&readable_fields(&issue.message)),
            }));
        let text: Option<String> = self
            .store
            .conn()
            .query_row("SELECT text FROM app_files WHERE document_id=?1", [id.to_string()], |row| row.get(0))
            .map_err(Error::storage)?;
        Ok(DocumentDetail {
            summary,
            fields: fields
                .iter()
                .map(|field| FieldView {
                    name: field.name,
                    value: editable_value(&field.value),
                    confidence: (effective_confidence(field) * 100.0).round() as u8,
                    tier: field.tier,
                    needs_review: review.contains(&field.name),
                })
                .collect(),
            line_items: items
                .into_iter()
                .map(|item| LineItemView {
                    description: item.description,
                    quantity: item.quantity,
                    unit_minor: item.unit_price.minor,
                    amount_minor: item.amount.minor,
                    currency: item.amount.currency.code().into(),
                })
                .collect(),
            transactions: transactions
                .into_iter()
                .map(|row| TransactionView {
                    date: row.date.to_string(),
                    description: row.description,
                    amount_minor: row.amount.minor,
                    balance_minor: row.balance.map(|balance| balance.minor),
                    currency: row.amount.currency.code().into(),
                })
                .collect(),
            issues,
            text,
            history: self
                .store
                .corrections(id)?
                .into_iter()
                .map(|change| CorrectionView {
                    field: change.field,
                    old: change.old.as_ref().map(editable_value),
                    old_source: change.old_tier,
                    new: change.new.as_ref().map(editable_value),
                    changed_at: change.changed_at,
                })
                .collect(),
        })
    }

    /// The original file and its media type.
    pub fn file(&self, id: DocId) -> Result<(MediaType, Vec<u8>)> {
        let (media, _) = self.file_info(id)?;
        Ok((media, self.read_blob(id)?))
    }

    // -- review ------------------------------------------------------------------------------

    pub fn update_field(&mut self, id: DocId, edit: &FieldEdit) -> Result<DocumentDetail> {
        let document = self.store.get_document(id)?;
        if edit.field == "kind" {
            let kind = DocKind::from_str(edit.value.trim())
                .map_err(|_| Error::Invalid("type must be invoice, receipt, statement, contract, or other".into()))?;
            self.store.conn()
                .execute("UPDATE app_files SET kind_override=?2 WHERE document_id=?1", params![id.to_string(), kind.as_str()])
                .map_err(Error::storage)?;
            self.store.set_kind(id, kind)?;
            // Read the stored text again with the new type's rules (statement rows, contract
            // terms). Values a person entered are kept.
            let stored: Option<(Option<String>, Option<String>)> = self
                .store
                .conn()
                .query_row("SELECT text, text_source FROM app_files WHERE document_id=?1", [id.to_string()], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .optional()
                .map_err(Error::storage)?;
            if let Some((Some(text), source)) = stored {
                let source = source.as_deref().and_then(TextSource::from_stored).unwrap_or(TextSource::TextLayer);
                self.store.set_status(id, DocStatus::NeedsReview)?;
                self.extract_text(id, &text, source)?;
                return self.detail(id);
            }
        } else {
            let name = FieldName::from_str(&edit.field)
                .map_err(|_| Error::Invalid(format!("unknown field: {}", edit.field)))?;
            let value = edit.value.trim();
            let before = self.store.fields(id)?.into_iter().find(|f| f.name == name);
            if value.is_empty() {
                if before.is_some() {
                    self.store.record_correction(id, name, before.as_ref(), None)?;
                    self.store.delete_field(id, name)?;
                }
            } else {
                let parsed = self.parse_value(id, name, value)?;
                if before.as_ref().map(|f| &f.value) != Some(&parsed) {
                    self.store.record_correction(id, name, before.as_ref(), Some(&parsed))?;
                }
                self.store.upsert_field(
                    id,
                    &ExtractedField { name, value: parsed, confidence: Confidence::ONE, tier: Tier::Human },
                )?;
            }
        }
        self.store.refresh_invoice(id)?;
        // Re-categorising an approved record keeps its approval; any other change re-opens review.
        let keeps_approval = edit.field == FieldName::Category.as_str();
        if document.status != DocStatus::Approved || !keeps_approval {
            self.revalidate(id)?;
        }
        self.detail(id)
    }

    fn parse_value(&self, id: DocId, name: FieldName, value: &str) -> Result<FieldValue> {
        match name {
            FieldName::DocDate | FieldName::DueDate | FieldName::PeriodStart | FieldName::PeriodEnd | FieldName::EndDate => Date::parse(value)
                .map(FieldValue::Date)
                .map_err(|_| Error::Invalid("dates must look like 2026-09-14".into())),
            FieldName::Subtotal | FieldName::Tax | FieldName::Total | FieldName::OpeningBalance | FieldName::ClosingBalance => {
                if !value.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | ' ')) {
                    return Err(Error::Invalid("amounts may only contain digits, separators, and a minus sign".into()));
                }
                let currency = self.document_currency(id)?;
                let money = Money::parse(value, currency)
                    .map_err(|error| Error::Invalid(format!("invalid amount: {error}")))?;
                if money.minor.unsigned_abs() > MAX_SAFE_MINOR as u64 {
                    return Err(Error::Invalid("that amount is too large".into()));
                }
                Ok(FieldValue::Money(money))
            }
            FieldName::Currency => Ok(FieldValue::Text(Currency::new(value)?.code().into())),
            FieldName::Category => CATEGORIES
                .iter()
                .find(|category| category.eq_ignore_ascii_case(value))
                .map(|category| FieldValue::Text((*category).into()))
                .ok_or_else(|| Error::Invalid(format!("category must be one of: {}", CATEGORIES.join(", ")))),
            FieldName::AccountNumber => {
                // Only the last four digits are kept, as when the statement is read.
                let digits: String = value.chars().filter(char::is_ascii_digit).collect();
                if digits.len() < 4 || value.chars().any(|c| !(c.is_ascii_digit() || " -•*xX".contains(c))) {
                    return Err(Error::Invalid("an account number needs at least four digits".into()));
                }
                Ok(FieldValue::Text(format!("••••{}", &digits[digits.len() - 4..])))
            }
            FieldName::NoticePeriod => {
                let mut parts = value.split_whitespace();
                let count = parts.next().and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=999).contains(n));
                let unit = parts.next().map(|u| u.to_ascii_lowercase());
                match (count, unit.as_deref().map(|u| u.trim_end_matches('s')), parts.next()) {
                    (Some(count), Some(unit @ ("day" | "week" | "month")), None) => {
                        let plural = if count == 1 { "" } else { "s" };
                        Ok(FieldValue::Text(format!("{count} {unit}{plural}")))
                    }
                    _ => Err(Error::Invalid("notice must look like 30 days, 2 weeks, or 1 month".into())),
                }
            }
            FieldName::AutoRenew => match value.to_ascii_lowercase().as_str() {
                "yes" | "no" => Ok(FieldValue::Text(value.to_ascii_lowercase())),
                _ => Err(Error::Invalid("auto-renew must be yes or no".into())),
            },
            FieldName::Vendor | FieldName::InvoiceNumber | FieldName::PaymentMethod | FieldName::Parties => {
                if value.chars().count() > MAX_TEXT_FIELD_CHARS || value.chars().any(char::is_control) {
                    return Err(Error::Invalid(format!(
                        "text must be at most {MAX_TEXT_FIELD_CHARS} characters with no control characters"
                    )));
                }
                Ok(FieldValue::Text(value.into()))
            }
        }
    }

    fn document_currency(&self, id: DocId) -> Result<Currency> {
        let fields = self.store.fields(id)?;
        let declared = fields
            .iter()
            .find(|f| f.name == FieldName::Currency)
            .and_then(|f| f.value.as_text())
            .and_then(|code| Currency::new(code).ok());
        let from_total = fields.iter().find_map(|f| f.value.as_money()).map(|m| m.currency);
        Ok(declared.or(from_total).unwrap_or(self.default_currency))
    }

    /// Recomputes status from stored values after a person changed them.
    fn revalidate(&mut self, id: DocId) -> Result<()> {
        let kind = self.rules_kind(self.store.get_document(id)?.kind);
        let fields = self.store.fields(id)?;
        let items = self.store.line_items(id)?;
        let transactions = self.store.transactions(id)?;
        let issues = validate_for(kind, &fields, &items, &transactions, self.today());
        let review = needs_review(&fields, required_fields(kind), self.review_threshold);
        let status = if review.is_empty()
            && !issues.iter().any(|issue| issue.severity == Severity::Error)
            && self.duplicate_of(id)?.is_none()
        {
            DocStatus::Extracted
        } else {
            DocStatus::NeedsReview
        };
        self.store.set_status(id, status)
    }

    /// Marks a document approved. It must have its kind's required fields: vendor, date, and
    /// total for an invoice or receipt; bank, period end, and both balances for a statement;
    /// parties and start date for a contract.
    pub fn approve(&mut self, id: DocId) -> Result<DocumentSummary> {
        let document = self.store.get_document(id)?;
        if document.kind == DocKind::Other {
            return Err(Error::Validation("set the document type before approving".into()));
        }
        if self.rules_kind(document.kind) != document.kind {
            return Err(Error::Validation("statements and contracts are part of Ledgerlens Pro, which isn't available yet".into()));
        }
        let fields = self.store.fields(id)?;
        let missing: Vec<&str> = required_fields(document.kind)
            .iter()
            .filter(|name| !fields.iter().any(|f| f.name == **name))
            .map(|name| label(*name))
            .collect();
        if !missing.is_empty() {
            return Err(Error::Validation(format!("add the {} before approving", missing.join(", "))));
        }
        self.store.refresh_invoice(id)?;
        self.store.set_status(id, DocStatus::Approved)?;
        self.summary(id)
    }

    /// Approves every document that validated cleanly ("Ready"). Returns how many were approved.
    pub fn approve_ready(&mut self) -> Result<u32> {
        let ready: Vec<DocId> = self
            .store
            .documents()?
            .into_iter()
            .filter(|doc| doc.status == DocStatus::Extracted)
            .map(|doc| doc.id)
            .collect();
        let ready: Vec<DocId> = ready
            .into_iter()
            .filter(|id| self.duplicate_of(*id).is_ok_and(|duplicate| duplicate.is_none()))
            .collect();
        let mut approved = 0;
        for id in ready {
            if self.approve(id).is_ok() {
                approved += 1;
            }
        }
        Ok(approved)
    }

    /// The earlier document this one repeats, if any.
    fn duplicate_of(&self, id: DocId) -> Result<Option<String>> {
        self.store
            .conn()
            .query_row(
                "SELECT other.document_id FROM records AS this
                 JOIN records AS other
                   ON other.document_id < this.document_id
                  AND other.vendor = this.vendor COLLATE NOCASE
                  AND (
                        (this.invoice_number IS NOT NULL AND other.invoice_number = this.invoice_number)
                     OR (this.invoice_date IS NOT NULL AND other.invoice_date = this.invoice_date
                         AND this.total_minor IS NOT NULL AND other.total_minor = this.total_minor
                         AND other.currency IS this.currency)
                  )
                 WHERE this.document_id = ?1
                 ORDER BY other.document_id LIMIT 1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::storage)
    }

    pub fn vendors(&self) -> Result<Vec<VendorSummary>> {
        let mut vendors: Vec<VendorSummary> = Vec::new();
        // Statements and contracts are not purchases: a closing balance or a contract's value
        // would be counted as spend.
        for doc in self.list()?.into_iter().filter(|doc| !matches!(doc.kind, DocKind::Statement | DocKind::Contract)) {
            let Some(name) = doc.vendor.clone() else { continue };
            let index = match vendors.iter().position(|v| v.name.eq_ignore_ascii_case(&name)) {
                Some(index) => index,
                None => {
                    vendors.push(VendorSummary {
                        name,
                        documents: 0,
                        needs_review: 0,
                        totals: vec![],
                        last_date: None,
                        categories: vec![],
                    });
                    vendors.len() - 1
                }
            };
            let vendor = &mut vendors[index];
            vendor.documents += 1;
            if doc.status == DocStatus::NeedsReview {
                vendor.needs_review += 1;
            }
            if doc.date > vendor.last_date {
                vendor.last_date = doc.date.clone();
            }
            if let Some(category) = &doc.category
                && !vendor.categories.contains(category) {
                    vendor.categories.push(category.clone());
                }
            if matches!(doc.status, DocStatus::Extracted | DocStatus::Approved)
                && let (Some(minor), Some(currency)) = (doc.total_minor, doc.currency.clone()) {
                    match vendor.totals.iter_mut().find(|row| row.currency == currency) {
                        Some(row) => {
                            row.total_minor = row.total_minor.saturating_add(minor);
                            row.count += 1;
                        }
                        None => vendor.totals.push(AmountRow { label: None, currency, total_minor: minor, count: 1 }),
                    }
                }
        }
        vendors.sort_by(|a, b| {
            let total = |v: &VendorSummary| v.totals.iter().map(|row| row.total_minor).max().unwrap_or(0);
            total(b).cmp(&total(a)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(vendors)
    }

    /// Full-text search over document text and file names. Every term is matched as a quoted
    /// prefix, so the input can never be read as FTS5 query syntax.
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let terms: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|term| !term.is_empty())
            .take(8)
            .map(|term| format!("\"{}\"*", term.chars().take(40).collect::<String>()))
            .collect();
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let mut statement = self
            .store
            .conn()
            .prepare(
                "SELECT document_id, snippet(document_search, 2, char(1), char(2), '…', 12)
                 FROM document_search WHERE document_search MATCH ?1 ORDER BY rank LIMIT 50",
            )
            .map_err(Error::storage)?;
        let rows = statement
            .query_map([terms.join(" ")], |row| Ok(SearchHit { id: row.get(0)?, snippet: row.get(1)? }))
            .map_err(Error::storage)?;
        rows.map(|row| row.map_err(Error::storage)).collect()
    }

    // -- questions, reports, export ----------------------------------------------------------

    pub fn ask(&self, question: &str) -> Result<Answer> {
        let question = question.trim();
        if question.is_empty() || question.chars().count() > 500 {
            return Err(Error::Invalid("questions must be between 1 and 500 characters".into()));
        }
        let vendors = self.distinct("vendor")?;
        let categories = CATEGORIES.iter().map(|c| (*c).to_string()).collect::<Vec<_>>();
        match intent::parse(question, self.today(), &vendors, &categories) {
            Intent::ReviewList => {
                let pending: Vec<DocumentSummary> =
                    self.list()?.into_iter().filter(|doc| doc.status == DocStatus::NeedsReview).collect();
                let names: Vec<String> = pending
                    .iter()
                    .map(|doc| doc.vendor.clone().unwrap_or_else(|| doc.file_name.clone()))
                    .collect();
                let answer = match names.len() {
                    0 => "Nothing needs review right now.".to_string(),
                    1 => format!("1 document needs review: {}.", names[0]),
                    n => format!("{n} documents need review: {}.", names.join(", ")),
                };
                Ok(Answer {
                    answer,
                    rows: vec![],
                    sql: None,
                    document_ids: pending.into_iter().map(|doc| doc.id).collect(),
                    pending_review: 0,
                })
            }
            Intent::Spend { query, scope } => {
                let plan = query.plan()?;
                let rows: Vec<AmountRow> = ll_query::execute(self.store.conn(), &plan)?
                    .into_iter()
                    .map(|row| AmountRow { label: row.label, currency: row.currency, total_minor: row.total_minor, count: row.count })
                    .collect();
                let mut unreviewed = query.clone();
                unreviewed.reviewed_only = false;
                let everything: u32 = ll_query::execute(self.store.conn(), &unreviewed.plan()?)?
                    .iter()
                    .map(|row| row.count)
                    .sum();
                let counted: u32 = rows.iter().map(|row| row.count).sum();
                let answer = describe(&scope, query.breakdown, &rows);
                Ok(Answer {
                    answer,
                    sql: Some(plan.sql().to_string()),
                    rows,
                    document_ids: vec![],
                    pending_review: everything.saturating_sub(counted),
                })
            }
        }
    }

    pub fn report(&self) -> Result<Report> {
        let mut counts = StatusCounts { total: 0, queued: 0, needs_review: 0, extracted: 0, approved: 0, failed: 0 };
        let documents = self.store.documents()?;
        let mut machine_read = 0u32;
        let mut confident = 0u32;
        for document in &documents {
            counts.total += 1;
            match document.status {
                DocStatus::Queued | DocStatus::Processing => counts.queued += 1,
                DocStatus::NeedsReview => counts.needs_review += 1,
                DocStatus::Extracted | DocStatus::Done => counts.extracted += 1,
                DocStatus::Approved => counts.approved += 1,
                DocStatus::Failed => counts.failed += 1,
            }
            let fields = self.store.fields(document.id)?;
            if fields.iter().any(|f| f.tier != Tier::Human) {
                machine_read += 1;
                let all_confident = required_fields(self.rules_kind(document.kind)).iter().all(|name| {
                    fields.iter().any(|f| f.name == *name && f.tier != Tier::Human && f.confidence.get() >= self.review_threshold)
                });
                confident += u32::from(all_confident);
            }
        }
        let breakdown = |breakdown: Breakdown| -> Result<Vec<AmountRow>> {
            let query = SpendQuery {
                from: None,
                through: None,
                vendor: None,
                category: None,
                currency: None,
                breakdown,
                reviewed_only: true,
                limit: 24,
            };
            Ok(ll_query::execute(self.store.conn(), &query.plan()?)?
                .into_iter()
                .map(|row| AmountRow { label: row.label, currency: row.currency, total_minor: row.total_minor, count: row.count })
                .collect())
        };
        let mut by_month = breakdown(Breakdown::Month)?;
        by_month.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(Report {
            counts,
            by_currency: breakdown(Breakdown::Total)?,
            by_category: breakdown(Breakdown::Category)?,
            by_vendor: breakdown(Breakdown::Vendor)?,
            by_month,
            high_confidence_percent: (machine_read > 0)
                .then(|| ((f64::from(confident) / f64::from(machine_read)) * 100.0).round() as u8),
        })
    }

    /// The files an export would create and how many rows they would hold. Nothing is written.
    pub fn export_plan(&self, format: ExportFormat, directory: &Path) -> Result<ExportPlan> {
        let files = self.export_contents(format)?;
        let mut paths: Vec<PathBuf> = Vec::new();
        for file in &files {
            paths.push(self.free_export_path(format, file, directory, &paths)?);
        }
        Ok(ExportPlan {
            paths: paths.iter().map(|path| path.display().to_string()).collect(),
            rows: files.iter().map(|file| file.rows).sum(),
        })
    }

    /// Writes reviewed records (extracted or approved) into `directory`: invoices and receipts as
    /// CSV, statements as OFX. Existing files are never overwritten.
    pub fn export(&self, format: ExportFormat, directory: &Path) -> Result<ExportResult> {
        let files = self.export_contents(format)?;
        let mut written: Vec<PathBuf> = Vec::new();
        for file in &files {
            let path = self.write_export(format, file, directory, &written)?;
            written.push(path);
        }
        Ok(ExportResult {
            paths: written.iter().map(|path| path.display().to_string()).collect(),
            rows: files.iter().map(|file| file.rows).sum(),
        })
    }

    fn reviewed(&self, kind: impl Fn(DocKind) -> bool) -> Result<Vec<ll_store::Document>> {
        Ok(self
            .store
            .documents()?
            .into_iter()
            .filter(|doc| kind(doc.kind) && matches!(doc.status, DocStatus::Extracted | DocStatus::Approved))
            .collect())
    }

    fn reviewed_invoices(&self) -> Result<Vec<ll_store::InvoiceRow>> {
        let reviewed: Vec<DocId> = self.reviewed(|kind| matches!(kind, DocKind::Invoice | DocKind::Receipt))?
            .into_iter().map(|doc| doc.id).collect();
        Ok(self.store.invoices()?.into_iter().filter(|row| reviewed.contains(&row.document_id)).collect())
    }

    fn export_contents(&self, format: ExportFormat) -> Result<Vec<ExportFile>> {
        let target = match format {
            ExportFormat::Canonical => ExportTarget::Canonical,
            ExportFormat::Quickbooks => ExportTarget::QuickBooks,
            ExportFormat::Xero => ExportTarget::Xero,
            ExportFormat::Ofx if !self.pro => {
                return Err(Error::Validation("statement export is part of Ledgerlens Pro, which isn't available yet".into()));
            }
            ExportFormat::Ofx => return self.ofx_files(),
        };
        let mut bytes = Vec::new();
        let rows = ll_export::export_invoices(&self.reviewed_invoices()?, target, &mut bytes)?;
        Ok(vec![ExportFile { suffix: String::new(), extension: "csv", bytes, rows }])
    }

    /// One OFX file per account and currency, holding every reviewed statement for it.
    fn ofx_files(&self) -> Result<Vec<ExportFile>> {
        struct Source { key: String, start: Option<Date>, end: Option<Date>, closing: Option<Money>, rows: Vec<ll_core::Transaction> }
        let mut groups: Vec<((String, Currency), Vec<Source>)> = Vec::new();
        for document in self.reviewed(|kind| kind == DocKind::Statement)? {
            let fields = self.store.fields(document.id)?;
            let field = |name: FieldName| fields.iter().find(|f| f.name == name).map(|f| f.value.clone());
            let rows = self.store.transactions(document.id)?;
            let closing = field(FieldName::ClosingBalance).and_then(|v| v.as_money());
            let currency = closing.map(|m| m.currency)
                .or_else(|| rows.first().map(|t| t.amount.currency))
                .unwrap_or(self.document_currency(document.id)?);
            let account: String = field(FieldName::AccountNumber)
                .and_then(|v| v.as_text().map(|t| t.chars().filter(char::is_ascii_digit).collect()))
                .unwrap_or_default();
            let source = Source {
                key: document.sha256[..16].to_string(),
                start: field(FieldName::PeriodStart).and_then(|v| v.as_date()),
                end: field(FieldName::PeriodEnd).and_then(|v| v.as_date()),
                closing,
                rows,
            };
            match groups.iter_mut().find(|(key, _)| *key == (account.clone(), currency)) {
                Some((_, sources)) => sources.push(source),
                None => groups.push(((account, currency), vec![source])),
            }
        }
        let accounts = groups.len();
        groups
            .into_iter()
            .map(|((account, currency), sources)| {
                let statements: Vec<OfxStatement<'_>> = sources
                    .iter()
                    .map(|s| OfxStatement {
                        document_key: &s.key, period_start: s.start, period_end: s.end,
                        closing_balance: s.closing, transactions: &s.rows,
                    })
                    .collect();
                let mut bytes = Vec::new();
                let id = if account.is_empty() { "unknown" } else { account.as_str() };
                let rows = ll_export::export_ofx(id, currency, &statements, self.today(), &mut bytes)?;
                let suffix = match (account.is_empty(), accounts > 1) {
                    (false, _) => format!("-{account}-{}", currency.code().to_ascii_lowercase()),
                    (true, true) => format!("-{}", currency.code().to_ascii_lowercase()),
                    (true, false) => String::new(),
                };
                Ok(ExportFile { suffix, extension: "ofx", bytes, rows })
            })
            .collect()
    }

    fn free_export_path(&self, format: ExportFormat, file: &ExportFile, directory: &Path, taken: &[PathBuf]) -> Result<PathBuf> {
        let stem = format!("ledgerlens-{}-{}{}", export_slug(format), self.today(), file.suffix);
        (0..100)
            .map(|attempt| directory.join(export_name(&stem, attempt, file.extension)))
            .find(|path| !path.exists() && !taken.contains(path))
            .ok_or_else(|| Error::Io("too many exports with the same name today".into()))
    }

    fn write_export(&self, format: ExportFormat, file: &ExportFile, directory: &Path, taken: &[PathBuf]) -> Result<PathBuf> {
        let stem = format!("ledgerlens-{}-{}{}", export_slug(format), self.today(), file.suffix);
        for attempt in 0..100 {
            let path = directory.join(export_name(&stem, attempt, file.extension));
            if taken.contains(&path) { continue; }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut handle) => {
                    handle.write_all(&file.bytes)?;
                    handle.sync_all()?;
                    return Ok(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::Io("too many exports with the same name today".into()))
    }

    // -- internals ---------------------------------------------------------------------------

    fn distinct(&self, column: &'static str) -> Result<Vec<String>> {
        let sql = match column {
            "vendor" => "SELECT DISTINCT vendor FROM records WHERE vendor IS NOT NULL",
            _ => return Err(Error::Invalid("unsupported column".into())),
        };
        let mut statement = self.store.conn().prepare(sql).map_err(Error::storage)?;
        let rows = statement.query_map([], |row| row.get(0)).map_err(Error::storage)?;
        rows.map(|row| row.map_err(Error::storage)).collect()
    }

    fn set_text(
        &self,
        id: DocId,
        text: Option<&str>,
        note: Option<&str>,
        source: Option<TextSource>,
    ) -> Result<()> {
        self.store
            .conn()
            .execute(
                "UPDATE app_files SET text=?2, note=?3, text_source=?4 WHERE document_id=?1",
                params![id.to_string(), text, note, source.map(TextSource::as_str)],
            )
            .map_err(Error::storage)?;
        self.store
            .conn()
            .execute("DELETE FROM document_search WHERE document_id=?1", [id.to_string()])
            .map_err(Error::storage)?;
        self.store
            .conn()
            .execute(
                "INSERT INTO document_search (document_id, file_name, text)
                 SELECT document_id, file_name, coalesce(text, '') FROM app_files WHERE document_id=?1",
                [id.to_string()],
            )
            .map_err(Error::storage)?;
        Ok(())
    }

    fn file_info(&self, id: DocId) -> Result<(MediaType, String)> {
        let row: Option<(String, String)> = self
            .store
            .conn()
            .query_row(
                "SELECT media_type, file_name FROM app_files WHERE document_id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(Error::storage)?;
        let (media, name) = row.ok_or_else(|| Error::NotFound(format!("document {id}")))?;
        Ok((MediaType::from_mime(&media)?, name))
    }

    fn blob_path(&self, sha256: &str, media: MediaType) -> Result<PathBuf> {
        // The key comes from our own hash, but check it anyway: it becomes a path.
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid("corrupt document key".into()));
        }
        Ok(self.blobs.join(&sha256[..2]).join(format!("{sha256}.{}", media.extension())))
    }

    fn write_blob(&self, sha256: &str, media: MediaType, bytes: &[u8]) -> Result<()> {
        let path = self.blob_path(sha256, media)?;
        if path.exists() {
            return Ok(());
        }
        let parent = path.parent().ok_or_else(|| Error::Io("invalid blob path".into()))?;
        create_private_dir(parent)?;
        let temporary = parent.join(format!(".{sha256}.partial"));
        {
            let mut file = File::create(&temporary)?;
            restrict_file(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        fs::rename(&temporary, &path)?;
        Ok(())
    }

    fn read_blob(&self, id: DocId) -> Result<Vec<u8>> {
        let document = self.store.get_document(id)?;
        let (media, _) = self.file_info(id)?;
        let bytes = fs::read(self.blob_path(&document.sha256, media)?)?;
        if hex::encode(Sha256::digest(&bytes)) != document.sha256 {
            return Err(Error::Validation("the stored file no longer matches its fingerprint".into()));
        }
        Ok(bytes)
    }
}

fn read_key(id: DocId) -> String {
    format!("read:{id}")
}

/// A document claimed for reading, with its verified file bytes.
pub struct ReadJob {
    job: ll_jobs::Job,
    document: DocId,
    media: MediaType,
    bytes: Vec<u8>,
}

/// What reading found; pass it back to [`App::finish_read`].
pub struct TextRead(TextOutcome);

impl ReadJob {
    pub fn document(&self) -> DocId {
        self.document
    }

    /// Extracts the text layer or runs OCR. Parsers run isolated and time-limited (see `text`).
    pub fn read(&self) -> TextRead {
        TextRead(text::extract(self.media, &self.bytes))
    }
}

/// One file of an export, ready to write.
struct ExportFile {
    /// Added to the file name, e.g. the account's last digits.
    suffix: String,
    extension: &'static str,
    bytes: Vec<u8>,
    rows: usize,
}

fn export_slug(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Canonical => "records",
        ExportFormat::Quickbooks => "quickbooks",
        ExportFormat::Xero => "xero",
        ExportFormat::Ofx => "statements",
    }
}

fn export_name(stem: &str, attempt: u32, extension: &str) -> String {
    if attempt == 0 { format!("{stem}.{extension}") } else { format!("{stem}-{attempt}.{extension}") }
}

fn default_kind(media: MediaType) -> DocKind {
    if media.is_image() { DocKind::Receipt } else { DocKind::Invoice }
}

fn effective_confidence(field: &ExtractedField) -> f32 {
    if field.tier == Tier::Human { 1.0 } else { field.confidence.get() }
}

fn editable_value(value: &FieldValue) -> String {
    match value {
        FieldValue::Text(text) => text.clone(),
        FieldValue::Date(date) => date.to_string(),
        FieldValue::Money(money) => plain_amount(*money),
    }
}

/// `1234.50`: no grouping, so it round-trips through `Money::parse`.
fn plain_amount(money: Money) -> String {
    let exponent = u32::from(money.currency.exponent());
    let sign = if money.minor < 0 { "-" } else { "" };
    let absolute = money.minor.unsigned_abs();
    if exponent == 0 {
        return format!("{sign}{absolute}");
    }
    let factor = 10u64.pow(exponent);
    format!("{sign}{}.{:0width$}", absolute / factor, absolute % factor, width = exponent as usize)
}

fn display_amount(currency: &str, minor: i64) -> String {
    match Currency::new(currency) {
        Ok(code) => format!("{currency} {}", Money::new(minor, code).to_decimal_string()),
        Err(_) => format!("{currency} {minor}"),
    }
}

/// Replaces internal field ids (`doc_date`) in a validation message with their labels (`date`).
fn readable_fields(message: &str) -> String {
    let mut out = message.to_string();
    for name in FieldName::ALL {
        if name.as_str().contains('_') {
            out = out.replace(name.as_str(), label(name));
        }
    }
    out
}

fn label(name: FieldName) -> &'static str {
    match name {
        FieldName::Vendor => "vendor",
        FieldName::InvoiceNumber => "invoice number",
        FieldName::DocDate => "date",
        FieldName::DueDate => "due date",
        FieldName::Currency => "currency",
        FieldName::Subtotal => "subtotal",
        FieldName::Tax => "tax",
        FieldName::Total => "total",
        FieldName::PaymentMethod => "payment method",
        FieldName::Category => "category",
        FieldName::AccountNumber => "account number",
        FieldName::PeriodStart => "period start",
        FieldName::PeriodEnd => "period end",
        FieldName::OpeningBalance => "opening balance",
        FieldName::ClosingBalance => "closing balance",
        FieldName::Parties => "parties",
        FieldName::EndDate => "end date",
        FieldName::NoticePeriod => "notice period",
        FieldName::AutoRenew => "auto-renew",
    }
}

fn describe(scope: &str, breakdown: Breakdown, rows: &[AmountRow]) -> String {
    if rows.is_empty() {
        return format!("{scope}: no reviewed records match.");
    }
    let records = |count: u32| if count == 1 { "1 record".to_string() } else { format!("{count} records") };
    match breakdown {
        Breakdown::Total => {
            let parts: Vec<String> = rows
                .iter()
                .map(|row| format!("{} across {}", display_amount(&row.currency, row.total_minor), records(row.count)))
                .collect();
            format!("{scope}: {}.", parts.join("; "))
        }
        _ => {
            let top = &rows[0];
            let name = top.label.clone().unwrap_or_else(|| "Uncategorised".into());
            format!(
                "{scope}, {} groups. Largest: {name} at {}.",
                rows.len(),
                display_amount(&top.currency, top.total_minor)
            )
        }
    }
}

fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn restrict_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
