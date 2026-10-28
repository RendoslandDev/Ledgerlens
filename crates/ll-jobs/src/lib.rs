//! A small durable job queue backed by the shared SQLite database.
//!
//! Semantics: at-least-once delivery. A claimed job holds a lease; if the worker dies the lease
//! expires and the job becomes claimable again. Failures retry with exponential backoff until
//! `max_attempts`, after which the job is marked `dead`. A queued or running job can be
//! `cancelled`; a cancelled running job's result is discarded (`complete`/`fail` only touch
//! running jobs).

use ll_core::{Error, JobId, Result};
use ll_store::{run_migrations, Migration, Store};
use rusqlite::{params, OptionalExtension};
use std::str::FromStr;

const NS: &str = "jobs";
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "jobs",
    sql: "
    CREATE TABLE jobs (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        payload TEXT NOT NULL,
        dedupe_key TEXT UNIQUE,
        state TEXT NOT NULL DEFAULT 'queued',   -- queued | running | done | dead | cancelled
        attempts INTEGER NOT NULL DEFAULT 0,
        max_attempts INTEGER NOT NULL,
        run_at INTEGER NOT NULL,
        lease_until INTEGER,
        last_error TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX idx_jobs_ready ON jobs(state, run_at);
    ",
}];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState { Queued, Running, Done, Dead, Cancelled }

impl JobState {
    fn parse(s: &str) -> JobState {
        match s {
            "running" => JobState::Running, "done" => JobState::Done, "dead" => JobState::Dead,
            "cancelled" => JobState::Cancelled, _ => JobState::Queued,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: JobId,
    pub kind: String,
    pub payload: String,
    pub attempts: u32,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats { pub queued: u32, pub running: u32, pub done: u32, pub dead: u32, pub cancelled: u32 }

pub struct Queue<'a> {
    store: &'a Store,
    max_attempts: u32,
    backoff_base_secs: u64,
}

fn se(e: rusqlite::Error) -> Error { Error::storage(e) }

fn parse_job_id(id: String) -> rusqlite::Result<JobId> {
    JobId::from_str(&id).map_err(|error| rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::other(error.to_string())),
    ))
}

impl<'a> Queue<'a> {
    /// Creates the queue's tables if needed, then attaches to them.
    pub fn new(store: &'a mut Store, max_attempts: u32, backoff_base_secs: u64) -> Result<Self> {
        Self::migrate(store)?;
        Ok(Queue { store, max_attempts, backoff_base_secs })
    }

    pub fn migrate(store: &mut Store) -> Result<()> {
        run_migrations(store.conn_mut(), NS, MIGRATIONS)
    }

    /// Attaches to a store whose queue tables already exist (see [`Queue::migrate`]).
    pub fn attach(store: &'a Store, max_attempts: u32, backoff_base_secs: u64) -> Self {
        Queue { store, max_attempts, backoff_base_secs }
    }

    /// Enqueues a job. With a `dedupe_key`, enqueueing a key whose job is still queued or running
    /// is a no-op returning that job's id; a key whose job finished (done, dead, or cancelled) is
    /// re-armed with the new payload and a fresh attempt budget.
    pub fn enqueue(&self, kind: &str, payload: &str, dedupe_key: Option<&str>, run_at: i64) -> Result<JobId> {
        let c = self.store.conn();
        if let Some(k) = dedupe_key
            && let Some((id, state)) = c.query_row("SELECT id, state FROM jobs WHERE dedupe_key=?1", [k], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional().map_err(se)? {
                if matches!(JobState::parse(&state), JobState::Done | JobState::Dead | JobState::Cancelled) {
                    c.execute(
                        "UPDATE jobs SET kind=?2, payload=?3, state='queued', attempts=0, max_attempts=?4, run_at=?5,
                                         lease_until=NULL, last_error=NULL, updated_at=?6 WHERE id=?1",
                        params![id, kind, payload, self.max_attempts, run_at, ll_core::now_unix()],
                    ).map_err(se)?;
                }
                return JobId::from_str(&id).map_err(|error| Error::Invalid(error.to_string()));
            }
        let now = ll_core::now_unix();
        let id = JobId::new();
        c.execute(
            "INSERT INTO jobs (id,kind,payload,dedupe_key,max_attempts,run_at,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
            params![id.to_string(), kind, payload, dedupe_key, self.max_attempts, run_at, now],
        ).map_err(se)?;
        Ok(id)
    }

    /// Atomically claims the oldest ready job (queued and due, or running with an expired lease).
    pub fn claim(&self, now: i64, lease_secs: i64) -> Result<Option<Job>> {
        self.store.conn().query_row(
            "UPDATE jobs SET state='running', attempts=attempts+1, lease_until=?1+?2, updated_at=?1
             WHERE id = (SELECT id FROM jobs
                         WHERE (state='queued' AND run_at<=?1) OR (state='running' AND lease_until<=?1)
                         ORDER BY run_at, rowid LIMIT 1)
             RETURNING id,kind,payload,attempts,max_attempts",
            params![now, lease_secs],
            |r| Ok(Job { id: parse_job_id(r.get(0)?)?, kind: r.get(1)?, payload: r.get(2)?, attempts: r.get(3)?, max_attempts: r.get(4)? }),
        ).optional().map_err(se)
    }

    /// Marks a running job done. Returns false when the job was no longer running (it was
    /// cancelled, or its lease expired and another worker reclaimed it).
    pub fn complete(&self, id: JobId) -> Result<bool> {
        let changed = self.store.conn().execute(
            "UPDATE jobs SET state='done', lease_until=NULL, updated_at=?2 WHERE id=?1 AND state='running'",
            params![id.to_string(), ll_core::now_unix()],
        ).map_err(se)?;
        Ok(changed == 1)
    }

    /// Whether `job` is still this claim's to finish: running and not reclaimed since.
    pub fn is_current(&self, job: &Job) -> Result<bool> {
        self.store.conn().query_row(
            "SELECT 1 FROM jobs WHERE id=?1 AND state='running' AND attempts=?2",
            params![job.id.to_string(), job.attempts], |_| Ok(()),
        ).optional().map(|row| row.is_some()).map_err(se)
    }

    /// Cancels a queued or running job. Returns false when it had already finished.
    pub fn cancel(&self, id: JobId) -> Result<bool> {
        let changed = self.store.conn().execute(
            "UPDATE jobs SET state='cancelled', lease_until=NULL, updated_at=?2
             WHERE id=?1 AND state IN ('queued','running')",
            params![id.to_string(), ll_core::now_unix()],
        ).map_err(se)?;
        Ok(changed == 1)
    }

    /// The job holding `dedupe_key`, if any.
    pub fn find(&self, dedupe_key: &str) -> Result<Option<(JobId, JobState)>> {
        self.store.conn().query_row(
            "SELECT id, state FROM jobs WHERE dedupe_key=?1", [dedupe_key],
            |r| Ok((parse_job_id(r.get(0)?)?, JobState::parse(&r.get::<_, String>(1)?))),
        ).optional().map_err(se)
    }

    /// Crash recovery for a single-process owner: jobs left `running` by a process that exited
    /// are made claimable now instead of waiting for their lease. Attempts are kept, so a job
    /// that crashes the process every time still ends up `dead`. Returns how many were reset.
    pub fn recover(&self, now: i64) -> Result<u32> {
        let changed = self.store.conn().execute(
            "UPDATE jobs SET state='queued', run_at=?1, lease_until=NULL, updated_at=?1 WHERE state='running'",
            [now],
        ).map_err(se)?;
        Ok(changed as u32)
    }

    /// When the next queued job becomes due, if any.
    pub fn next_run_at(&self) -> Result<Option<i64>> {
        self.store.conn().query_row(
            "SELECT MIN(CASE WHEN state='queued' THEN run_at ELSE lease_until END) FROM jobs WHERE state IN ('queued','running')",
            [], |r| r.get(0),
        ).map_err(se)
    }

    /// Records a failure of a running job. Returns the new state: `Queued` (will retry), `Dead`,
    /// or the job's current state when it was no longer running (cancelled meanwhile).
    pub fn fail(&self, job: &Job, err: &str, now: i64) -> Result<JobState> {
        let (state, run_at) = if job.attempts >= job.max_attempts {
            ("dead", now)
        } else {
            let exp = job.attempts.saturating_sub(1).min(20);
            ("queued", now + (self.backoff_base_secs as i64) * (1i64 << exp))
        };
        let changed = self.store.conn().execute(
            "UPDATE jobs SET state=?2, run_at=?3, lease_until=NULL, last_error=?4, updated_at=?5 WHERE id=?1 AND state='running'",
            params![job.id.to_string(), state, run_at, err, now],
        ).map_err(se)?;
        if changed == 0 {
            return self.state(job.id);
        }
        Ok(JobState::parse(state))
    }

    /// Records a failure that retrying cannot fix: the job is marked `dead` at once.
    pub fn bury(&self, job: &Job, err: &str, now: i64) -> Result<JobState> {
        let changed = self.store.conn().execute(
            "UPDATE jobs SET state='dead', lease_until=NULL, last_error=?2, updated_at=?3 WHERE id=?1 AND state='running'",
            params![job.id.to_string(), err, now],
        ).map_err(se)?;
        if changed == 0 { self.state(job.id) } else { Ok(JobState::Dead) }
    }

    pub fn state(&self, id: JobId) -> Result<JobState> {
        let s: String = self.store.conn().query_row("SELECT state FROM jobs WHERE id=?1", [id.to_string()], |r| r.get(0)).map_err(se)?;
        Ok(JobState::parse(&s))
    }

    pub fn last_error(&self, id: JobId) -> Result<Option<String>> {
        self.store.conn().query_row("SELECT last_error FROM jobs WHERE id=?1", [id.to_string()], |r| r.get(0)).map_err(se)
    }

    pub fn stats(&self) -> Result<Stats> {
        let mut st = self.store.conn().prepare("SELECT state, COUNT(*) FROM jobs GROUP BY state").map_err(se)?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))).map_err(se)?;
        let mut s = Stats::default();
        for r in rows {
            let (state, n) = r.map_err(se)?;
            match JobState::parse(&state) {
                JobState::Queued => s.queued = n, JobState::Running => s.running = n,
                JobState::Done => s.done = n, JobState::Dead => s.dead = n,
                JobState::Cancelled => s.cancelled = n,
            }
        }
        Ok(s)
    }

    /// Runs jobs until none are ready at `now`. Returns how many were processed (successes + failures).
    pub fn drain<F>(&self, now: i64, lease_secs: i64, mut handler: F) -> Result<u32>
    where
        F: FnMut(&Job) -> std::result::Result<(), String>,
    {
        let mut n = 0;
        while let Some(job) = self.claim(now, lease_secs)? {
            match handler(&job) {
                Ok(()) => { self.complete(job.id)?; }
                Err(e) => { self.fail(&job, &e, now)?; }
            }
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store { Store::open_in_memory().unwrap() }

    #[test]
    fn enqueue_claim_complete() {
        let mut s = store();
        let q = Queue::new(&mut s, 3, 10).unwrap();
        let id = q.enqueue("ocr", "{\"doc\":1}", None, 100).unwrap();
        assert!(q.claim(50, 30).unwrap().is_none(), "not due yet");
        let job = q.claim(100, 30).unwrap().unwrap();
        assert_eq!((job.id, job.attempts), (id, 1));
        assert!(q.claim(100, 30).unwrap().is_none(), "leased");
        q.complete(id).unwrap();
        assert_eq!(q.state(id).unwrap(), JobState::Done);
    }

    #[test]
    fn dedupe_key() {
        let mut s = store();
        let q = Queue::new(&mut s, 3, 10).unwrap();
        let a = q.enqueue("ocr", "x", Some("doc-1"), 0).unwrap();
        let b = q.enqueue("ocr", "y", Some("doc-1"), 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(q.stats().unwrap().queued, 1);
    }

    #[test]
    fn finished_dedupe_key_is_rearmed() {
        let mut s = store();
        let q = Queue::new(&mut s, 2, 10).unwrap();
        let a = q.enqueue("read", "v1", Some("doc-1"), 0).unwrap();
        let job = q.claim(0, 30).unwrap().unwrap();
        q.fail(&job, "boom", 0).unwrap();
        let job = q.claim(10, 30).unwrap().unwrap();
        assert_eq!(q.fail(&job, "boom", 10).unwrap(), JobState::Dead);

        let b = q.enqueue("read", "v2", Some("doc-1"), 50).unwrap();
        assert_eq!(a, b);
        let job = q.claim(50, 30).unwrap().unwrap();
        assert_eq!((job.payload.as_str(), job.attempts), ("v2", 1));
        assert_eq!(q.last_error(a).unwrap(), None);
    }

    #[test]
    fn cancelled_jobs_are_not_claimed_and_results_are_discarded() {
        let mut s = store();
        let q = Queue::new(&mut s, 3, 10).unwrap();
        let queued = q.enqueue("x", "", None, 0).unwrap();
        assert!(q.cancel(queued).unwrap());
        assert!(q.claim(0, 30).unwrap().is_none());

        let running = q.enqueue("x", "", None, 0).unwrap();
        let job = q.claim(0, 30).unwrap().unwrap();
        assert!(q.is_current(&job).unwrap());
        assert!(q.cancel(running).unwrap());
        assert!(!q.is_current(&job).unwrap());
        assert!(!q.complete(running).unwrap(), "a cancelled job stays cancelled");
        assert_eq!(q.fail(&job, "late", 0).unwrap(), JobState::Cancelled);
        assert_eq!(q.state(running).unwrap(), JobState::Cancelled);
        assert!(!q.cancel(running).unwrap());
        assert_eq!(q.stats().unwrap().cancelled, 2);
    }

    #[test]
    fn reclaimed_job_is_no_longer_current_for_the_first_worker() {
        let mut s = store();
        let q = Queue::new(&mut s, 5, 10).unwrap();
        q.enqueue("x", "", None, 0).unwrap();
        let first = q.claim(0, 30).unwrap().unwrap();
        let second = q.claim(30, 30).unwrap().unwrap();
        assert!(!q.is_current(&first).unwrap());
        assert!(q.is_current(&second).unwrap());
    }

    #[test]
    fn recover_requeues_jobs_left_running_by_a_crash() {
        let mut s = store();
        Queue::migrate(&mut s).unwrap();
        let q = Queue::attach(&s, 3, 10);
        let id = q.enqueue("x", "", None, 0).unwrap();
        q.claim(0, 600).unwrap().unwrap();
        assert_eq!(q.next_run_at().unwrap(), Some(600));
        assert_eq!(q.recover(5).unwrap(), 1);
        let job = q.claim(5, 600).unwrap().expect("claimable right after recovery");
        assert_eq!((job.id, job.attempts), (id, 2));
        q.complete(id).unwrap();
        assert_eq!(q.next_run_at().unwrap(), None);
    }

    #[test]
    fn retries_with_backoff_then_dies() {
        let mut s = store();
        let q = Queue::new(&mut s, 3, 10).unwrap();
        let id = q.enqueue("x", "", None, 0).unwrap();

        let j = q.claim(0, 5).unwrap().unwrap();
        assert_eq!(q.fail(&j, "boom", 0).unwrap(), JobState::Queued);
        assert!(q.claim(9, 5).unwrap().is_none(), "backoff 10s");
        let j = q.claim(10, 5).unwrap().unwrap();
        assert_eq!(j.attempts, 2);
        q.fail(&j, "boom", 10).unwrap();
        assert!(q.claim(29, 5).unwrap().is_none(), "backoff 20s");
        let j = q.claim(30, 5).unwrap().unwrap();
        assert_eq!(q.fail(&j, "final", 30).unwrap(), JobState::Dead);
        assert_eq!(q.state(id).unwrap(), JobState::Dead);
        assert_eq!(q.last_error(id).unwrap().as_deref(), Some("final"));
        assert!(q.claim(10_000, 5).unwrap().is_none());

        let id = q.enqueue("x", "", None, 0).unwrap();
        let j = q.claim(0, 5).unwrap().unwrap();
        assert_eq!(q.bury(&j, "permanent", 0).unwrap(), JobState::Dead);
        assert_eq!(q.state(id).unwrap(), JobState::Dead);
    }

    #[test]
    fn expired_lease_is_reclaimed() {
        let mut s = store();
        let q = Queue::new(&mut s, 5, 10).unwrap();
        q.enqueue("x", "", None, 0).unwrap();
        q.claim(0, 30).unwrap().unwrap();
        assert!(q.claim(29, 30).unwrap().is_none());
        let j = q.claim(30, 30).unwrap().expect("lease expired");
        assert_eq!(j.attempts, 2);
    }

    #[test]
    fn drain_processes_all() {
        let mut s = store();
        let q = Queue::new(&mut s, 2, 1).unwrap();
        for i in 0..3 { q.enqueue("x", &i.to_string(), None, 0).unwrap(); }
        let mut seen = vec![];
        let n = q.drain(0, 10, |j| { seen.push(j.payload.clone()); if j.payload == "1" { Err("bad".into()) } else { Ok(()) } }).unwrap();
        assert_eq!(n, 3);
        assert_eq!(seen, ["0", "1", "2"]);
        let st = q.stats().unwrap();
        assert_eq!((st.done, st.queued), (2, 1));
    }
}
