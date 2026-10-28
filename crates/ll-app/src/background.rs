//! The background worker: reads queued documents and scans the watched folder, off the UI's
//! request path. Slow work (PDF parsing, OCR, reading files from the folder) runs without the
//! service lock, so commands stay responsive while a scan is read.
//!
//! The worker keeps an [`Activity`] snapshot outside the service lock; the UI polls it cheaply and
//! refreshes when `revision` changes.

use crate::{watch::WATCH_INTERVAL_SECS, App};
use serde::Serialize;
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread,
    time::Duration,
};

/// Longest the worker sleeps without a wake-up; bounds how late a retry or scan can be.
const MAX_IDLE: Duration = Duration::from_secs(30);
/// Pause after an unexpected error, so a persistent fault does not spin.
const ERROR_PAUSE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Changes whenever the worker changed documents; the UI reloads when it differs.
    pub revision: u32,
    /// Documents waiting to be read.
    pub queued: u32,
    /// The document being read now.
    pub reading: Option<String>,
    pub watching: bool,
    pub scanning: bool,
    /// Set when the worker stopped because the workspace became unusable.
    pub stopped: bool,
}

#[derive(Default)]
struct Control {
    stop: bool,
    wake: bool,
    scan_now: bool,
}

struct Shared {
    app: Arc<Mutex<App>>,
    control: Mutex<Control>,
    signal: Condvar,
    activity: Mutex<Activity>,
}

/// Handle to the worker thread. Dropping it asks the thread to stop after its current step.
pub struct Background {
    shared: Arc<Shared>,
}

impl Background {
    pub fn start(app: Arc<Mutex<App>>) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            app,
            control: Mutex::new(Control { scan_now: true, ..Control::default() }),
            signal: Condvar::new(),
            activity: Mutex::new(Activity::default()),
        });
        let worker = Arc::clone(&shared);
        thread::Builder::new().name("ll-background".into()).spawn(move || run(&worker))?;
        Ok(Self { shared })
    }

    /// New work was queued: read it now rather than at the next timeout.
    pub fn wake(&self) {
        self.signal(|control| control.wake = true);
    }

    /// Scan the watched folder now (after it was chosen or resumed, or on request).
    pub fn scan_now(&self) {
        self.signal(|control| control.scan_now = true);
    }

    pub fn activity(&self) -> Activity {
        lock(&self.shared.activity).clone()
    }

    fn signal(&self, change: impl FnOnce(&mut Control)) {
        change(&mut lock(&self.shared.control));
        self.shared.signal.notify_all();
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        self.signal(|control| control.stop = true);
    }
}

/// The snapshot and control mutexes guard plain data, so a poisoned lock is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn run(shared: &Shared) {
    let mut next_scan = 0i64;
    loop {
        let scan_requested = {
            let mut control = lock(&shared.control);
            if control.stop {
                return;
            }
            control.wake = false;
            std::mem::take(&mut control.scan_now)
        };
        match step(shared, scan_requested, &mut next_scan) {
            Ok(Step::Busy) => continue,
            Ok(Step::Idle(wait)) => sleep(shared, wait),
            Err(Fault::Workspace) => {
                lock(&shared.activity).stopped = true;
                return;
            }
            Err(Fault::Error(error)) => {
                // Codes only: messages can quote document content.
                eprintln!("ledgerlens: background work failed ({})", error.code().as_str());
                sleep(shared, ERROR_PAUSE);
            }
        }
    }
}

enum Step {
    /// Did work; look for more straight away.
    Busy,
    /// Nothing to do for this long.
    Idle(Duration),
}

enum Fault {
    /// The service lock is poisoned: a command panicked mid-change, so stop touching the data.
    Workspace,
    Error(ll_core::Error),
}

impl From<ll_core::Error> for Fault {
    fn from(error: ll_core::Error) -> Self {
        Fault::Error(error)
    }
}

fn app(shared: &Shared) -> Result<MutexGuard<'_, App>, Fault> {
    shared.app.lock().map_err(|_| Fault::Workspace)
}

fn step(shared: &Shared, scan_requested: bool, next_scan: &mut i64) -> Result<Step, Fault> {
    let now = ll_core::now_unix();
    if scan_requested || now >= *next_scan {
        *next_scan = now + WATCH_INTERVAL_SECS;
        let plan = app(shared)?.watch_plan()?;
        lock(&shared.activity).watching = plan.is_some();
        if let Some(plan) = plan {
            lock(&shared.activity).scanning = true;
            let batch = plan.collect(now);
            let scan = app(shared)?.apply_watch(batch);
            lock(&shared.activity).scanning = false;
            if scan?.imported > 0 {
                bump(shared);
                return Ok(Step::Busy);
            }
        }
    }

    let job = app(shared)?.claim_read()?;
    if let Some(job) = job {
        {
            let mut activity = lock(&shared.activity);
            activity.reading = Some(job.document().to_string());
        }
        bump(shared);
        let outcome = job.read();
        let finished = app(shared)?.finish_read(job, outcome);
        lock(&shared.activity).reading = None;
        finished?;
        bump(shared);
        return Ok(Step::Busy);
    }

    let (queued, next_read) = {
        let app = app(shared)?;
        let (queued, _) = app.pending_reads()?;
        (queued, app.next_read_at()?)
    };
    lock(&shared.activity).queued = queued;
    let now = ll_core::now_unix();
    let until = next_read.map_or(*next_scan, |at| at.min(*next_scan));
    let wait = Duration::from_secs(until.saturating_sub(now).max(1) as u64).min(MAX_IDLE);
    Ok(Step::Idle(wait))
}

fn bump(shared: &Shared) {
    let queued = shared.app.lock().ok().and_then(|app| app.pending_reads().ok());
    let mut activity = lock(&shared.activity);
    activity.revision = activity.revision.wrapping_add(1);
    if let Some((queued, _)) = queued {
        activity.queued = queued;
    }
}

fn sleep(shared: &Shared, wait: Duration) {
    let control = lock(&shared.control);
    if control.stop || control.wake || control.scan_now {
        return;
    }
    let _ = shared
        .signal
        .wait_timeout_while(control, wait, |control| !(control.stop || control.wake || control.scan_now));
}
