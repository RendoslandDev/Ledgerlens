//! Durable reading through the `ll-jobs` queue, crash recovery, cancellation, opt-in folder
//! watching, and the background worker, over real files.

use ll_app::{background::Background, App};
use ll_core::{Date, DocId};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

const INVOICE_PDF: &[u8] = include_bytes!("fixtures/invoice-cups.pdf");
const PNG: &[u8] = include_bytes!("fixtures/blank.png");

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ll-bg-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn data(&self) -> PathBuf {
        self.dir.join("data")
    }

    fn app(&self) -> App {
        App::open(&self.data()).unwrap().with_today(Date::new(2026, 10, 27).unwrap())
    }

    /// A folder of paperwork to watch, outside the data folder.
    fn inbox(&self) -> PathBuf {
        let inbox = self.dir.join("Receipts");
        fs::create_dir_all(&inbox).unwrap();
        inbox
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn id(raw: &str) -> DocId {
    DocId::from_str(raw).unwrap()
}

/// Writes a file that looks finished being written (modified a minute ago).
fn put(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(60)).unwrap();
}

#[test]
fn admitted_documents_wait_in_the_queue_until_read() {
    let scratch = Scratch::new("queue");
    let mut app = scratch.app();
    let admitted = app.admit("invoice.pdf", INVOICE_PDF).unwrap();
    assert!(!admitted.duplicate);
    assert_eq!(admitted.document.status.as_str(), "queued");
    assert_eq!(admitted.document.vendor, None);
    assert_eq!(app.pending_reads().unwrap(), (1, 0));
    assert!(app.next_read_at().unwrap().is_some());

    assert_eq!(app.run_pending().unwrap(), 1);
    let read = app.summary(id(&admitted.document.id)).unwrap();
    assert_eq!(read.status.as_str(), "extracted");
    assert_eq!(read.total_minor, Some(172_500));
    assert_eq!(app.pending_reads().unwrap(), (0, 0));
    assert_eq!(app.next_read_at().unwrap(), None);
    assert_eq!(app.run_pending().unwrap(), 0, "a read document is not read twice");
}

#[test]
fn reads_interrupted_by_a_crash_resume_on_restart() {
    let scratch = Scratch::new("crash");
    let doc = {
        let mut app = scratch.app();
        let doc = app.admit("invoice.pdf", INVOICE_PDF).unwrap().document;
        let claimed = app.claim_read().unwrap().expect("claimed");
        assert_eq!(app.summary(claimed.document()).unwrap().status.as_str(), "processing");
        // The process exits mid-read: the claim is never finished.
        std::mem::forget(claimed);
        doc
    };
    let mut app = scratch.app();
    assert_eq!(app.summary(id(&doc.id)).unwrap().status.as_str(), "queued");
    assert_eq!(app.run_pending().unwrap(), 1);
    assert_eq!(app.summary(id(&doc.id)).unwrap().status.as_str(), "extracted");
}

#[test]
fn imports_can_be_cancelled_before_and_while_they_are_read() {
    let scratch = Scratch::new("cancel");
    let mut app = scratch.app();

    let waiting = app.admit("invoice.pdf", INVOICE_PDF).unwrap().document;
    app.cancel_import(id(&waiting.id)).unwrap();
    assert!(app.summary(id(&waiting.id)).is_err(), "a never-read import is removed");
    assert_eq!(app.run_pending().unwrap(), 0);

    let reading = app.admit("invoice.pdf", INVOICE_PDF).unwrap().document;
    let claimed = app.claim_read().unwrap().unwrap();
    let outcome = claimed.read();
    app.cancel_import(id(&reading.id)).unwrap();
    assert_eq!(app.finish_read(claimed, outcome).unwrap(), None, "the late result is discarded");
    assert!(app.list().unwrap().is_empty());

    // Re-reading a document that has values: cancelling keeps it, back in review.
    let kept = app.import("invoice.pdf", INVOICE_PDF).unwrap().document;
    app.reprocess(id(&kept.id)).unwrap();
    app.cancel_import(id(&kept.id)).unwrap();
    let kept = app.summary(id(&kept.id)).unwrap();
    assert_eq!(kept.status.as_str(), "needs_review");
    assert_eq!(kept.total_minor, Some(172_500));
    assert!(app.cancel_import(id(&kept.id)).is_err(), "only waiting imports can be cancelled");

    app.admit("blank.png", PNG).unwrap();
    assert_eq!(app.cancel_pending().unwrap(), 1);
    assert_eq!(app.list().unwrap().len(), 1);
}

#[test]
fn a_damaged_stored_file_fails_once_instead_of_retrying() {
    let scratch = Scratch::new("damaged");
    let mut app = scratch.app();
    let doc = app.admit("invoice.pdf", INVOICE_PDF).unwrap().document;
    let blob = fs::read_dir(scratch.data().join("blobs"))
        .unwrap()
        .flat_map(|dir| fs::read_dir(dir.unwrap().path()).unwrap())
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "pdf"))
        .unwrap();
    fs::write(&blob, b"%PDF-1.7 tampered").unwrap();

    assert_eq!(app.run_pending().unwrap(), 0);
    let failed = app.summary(id(&doc.id)).unwrap();
    assert_eq!(failed.status.as_str(), "failed");
    assert!(failed.note.unwrap().contains("damaged"));
    assert_eq!(app.next_read_at().unwrap(), None, "not retried");
}

#[test]
fn watching_is_opt_in_and_imports_new_files_once() {
    let scratch = Scratch::new("watch");
    let mut app = scratch.app();
    let inbox = scratch.inbox();
    put(&inbox.join("invoice.pdf"), INVOICE_PDF);

    let off = app.watch_status().unwrap();
    assert_eq!((off.folder, off.enabled), (None, false));
    assert!(app.scan_watch_folder().unwrap().is_none(), "nothing is scanned until a folder is chosen");
    assert!(app.set_watch_enabled(true).is_err());

    let on = app.set_watch_folder(&inbox).unwrap();
    assert!(on.enabled);
    assert_eq!(on.folder.as_deref(), Some(fs::canonicalize(&inbox).unwrap().to_str().unwrap()));
    assert_eq!((on.scope.max_depth, on.scope.max_file_mb), (3, 25));

    let first = app.scan_watch_folder().unwrap().unwrap();
    assert_eq!((first.found, first.imported), (1, 1));
    assert_eq!(app.list().unwrap()[0].status.as_str(), "queued", "watched files go through the queue");
    app.run_pending().unwrap();
    let doc = app.list().unwrap().remove(0);
    assert_eq!((doc.file_name.as_str(), doc.total_minor), ("invoice.pdf", Some(172_500)));

    let again = app.scan_watch_folder().unwrap().unwrap();
    assert_eq!((again.found, again.imported, again.duplicates), (1, 0, 0), "unchanged files are not re-read");

    // A copy is recognised by content; a fake PDF and a file still being written are not taken.
    put(&inbox.join("copy.pdf"), INVOICE_PDF);
    put(&inbox.join("fake.pdf"), b"MZ\x90\0 not a document");
    fs::write(inbox.join("new.png"), PNG).unwrap();
    let mixed = app.scan_watch_folder().unwrap().unwrap();
    assert_eq!((mixed.imported, mixed.duplicates, mixed.rejected, mixed.waiting), (0, 1, 1, 1));
    assert_eq!(app.list().unwrap().len(), 1);

    // Removing the document does not bring it back while the file is unchanged.
    app.delete(id(&doc.id)).unwrap();
    let after_delete = app.scan_watch_folder().unwrap().unwrap();
    assert_eq!(after_delete.imported, 0);
    assert_eq!(app.watch_status().unwrap().imported, 0);

    let paused = app.set_watch_enabled(false).unwrap();
    assert!(!paused.enabled && paused.folder.is_some());
    assert!(app.scan_watch_folder().unwrap().is_none());

    let forgotten = app.forget_watch_folder().unwrap();
    assert_eq!((forgotten.folder, forgotten.enabled), (None, false));
}

#[test]
fn watching_refuses_links_broad_folders_and_its_own_data() {
    let scratch = Scratch::new("scope");
    let mut app = scratch.app();
    let inbox = scratch.inbox();

    assert!(app.set_watch_folder(Path::new("/")).is_err());
    assert!(app.set_watch_folder(Path::new("/Users")).is_err());
    assert!(app.set_watch_folder(Path::new("/System/Library")).is_err());
    if let Some(home) = std::env::var_os("HOME") {
        assert!(app.set_watch_folder(Path::new(&home)).is_err(), "the whole home folder is too broad");
        let library = Path::new(&home).join("Library");
        if library.is_dir() {
            assert!(app.set_watch_folder(&library).is_err());
        }
    }
    assert!(app.set_watch_folder(&scratch.data()).is_err(), "its own data folder");
    assert!(app.set_watch_folder(&scratch.data().join("blobs")).is_err());
    assert!(app.set_watch_folder(&scratch.dir).is_err(), "a folder containing its data");
    assert!(app.set_watch_folder(Path::new("Receipts")).is_err(), "relative paths");
    assert!(app.set_watch_folder(&inbox.join("missing")).is_err());

    #[cfg(unix)]
    {
        let link = scratch.dir.join("receipts-link");
        std::os::unix::fs::symlink(&inbox, &link).unwrap();
        assert!(app.set_watch_folder(&link).is_err(), "a linked folder is refused");

        let outside = scratch.dir.join("private");
        fs::create_dir_all(&outside).unwrap();
        put(&outside.join("secret.pdf"), INVOICE_PDF);
        std::os::unix::fs::symlink(outside.join("secret.pdf"), inbox.join("linked.pdf")).unwrap();
        std::os::unix::fs::symlink(&outside, inbox.join("linked-folder")).unwrap();
        app.set_watch_folder(&inbox).unwrap();
        let scan = app.scan_watch_folder().unwrap().unwrap();
        assert_eq!((scan.found, scan.imported, scan.skipped_links), (0, 0, 2));
        assert!(app.list().unwrap().is_empty(), "nothing outside the folder is read");

        // The folder is swapped for a link after it was chosen: the scan stops.
        fs::rename(&inbox, scratch.dir.join("Receipts-moved")).unwrap();
        std::os::unix::fs::symlink(&outside, &inbox).unwrap();
        let swapped = app.scan_watch_folder().unwrap().unwrap();
        assert!(swapped.error.is_some());
        assert_eq!(swapped.imported, 0);
    }
}

#[test]
fn the_background_worker_reads_imports_and_watches_the_folder() {
    let scratch = Scratch::new("worker");
    let inbox = scratch.inbox();
    let app = Arc::new(Mutex::new(scratch.app()));
    let worker = Background::start(Arc::clone(&app)).unwrap();

    let doc = app.lock().unwrap().admit("invoice.pdf", INVOICE_PDF).unwrap().document;
    worker.wake();
    let status = wait_for(|| {
        let summary = app.lock().unwrap().summary(id(&doc.id)).unwrap();
        (summary.status.as_str() == "extracted").then_some(summary.status)
    });
    assert_eq!(status.as_str(), "extracted");
    let activity = wait_for(|| {
        let activity = worker.activity();
        (activity.reading.is_none() && activity.queued == 0).then_some(activity)
    });
    assert!(activity.revision > 0);
    assert!(!activity.watching);

    put(&inbox.join("scan.png"), PNG);
    app.lock().unwrap().set_watch_folder(&inbox).unwrap();
    worker.scan_now();
    let watched = wait_for(|| {
        let list = app.lock().unwrap().list().unwrap();
        list.into_iter()
            .find(|document| document.file_name == "scan.png" && document.status.as_str() == "needs_review")
    });
    assert!(watched.note.is_some());
    assert!(worker.activity().watching);
    drop(worker);
}

fn wait_for<T>(mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for the background worker");
        std::thread::sleep(Duration::from_millis(50));
    }
}
