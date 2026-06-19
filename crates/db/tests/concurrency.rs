//! Durability-under-concurrency tests for the persistence layer (the library-level analogue of
//! VAL-RUN-009 / VAL-RUN-011 / VAL-RUN-012, which the CLI live tests prove end-to-end).
//!
//! These exercise the real on-disk SQLite file (not the in-memory db) with multiple independent
//! connections, each with its own busy timeout, mirroring separate `cortex run` processes sharing
//! one database file:
//! - many connections writing to DISTINCT sessions concurrently all succeed without corruption;
//! - many connections writing to the SAME session concurrently serialize (the busy timeout absorbs
//!   the contention) so every write lands without a lost/duplicated row;
//! - an uncommitted write left by an abruptly abandoned transaction is rolled back on the next
//!   open, leaving the database uncorrupted (the in-process analogue of a `SIGKILL` mid-turn).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;

use db::Database;

/// A self-cleaning scratch directory under the system temp dir.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-db-conc-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create scratch dir");
        Self { root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Open a fresh connection to the same file and assert `PRAGMA integrity_check` is `ok`.
fn assert_integrity_ok(db_path: &PathBuf) {
    let db = Database::open(db_path).expect("reopen for integrity check");
    assert_eq!(
        db.integrity_check().expect("integrity check"),
        "ok",
        "the database must remain uncorrupted"
    );
}

// `integrity_check` reports `ok` for a freshly migrated database and for one carrying real rows.
#[test]
fn integrity_check_reports_ok_for_fresh_and_populated_databases() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let db = Database::open(&db_path).expect("open fresh db");
    assert_eq!(db.integrity_check().expect("integrity"), "ok");

    let session = db.sessions().create(Some("populated")).expect("session");
    db.messages()
        .create(&session.id, "user", "hello")
        .expect("message");
    db.usage()
        .record(&session.id, 5, 3, 8, Some(0.000_01))
        .expect("usage");
    assert_eq!(db.integrity_check().expect("integrity"), "ok");
}

// Many independent connections (separate `cortex run` processes) writing to DISTINCT sessions
// concurrently each succeed with no shared-state corruption: every session and its message land,
// the row counts are exact, and the database passes its integrity check (VAL-RUN-009, library
// level).
#[test]
fn concurrent_writes_to_distinct_sessions_all_succeed() {
    let scratch = Scratch::new();
    let db_path = Arc::new(scratch.path("cortex.db"));
    // Materialize the file + schema once up front so every worker only contends on writes.
    Database::open(db_path.as_ref()).expect("create db");

    const WORKERS: usize = 5;
    let (ready_tx, ready_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let go_rx = Arc::new(std::sync::Mutex::new(go_rx));

    let mut handles = Vec::new();
    for worker in 0..WORKERS {
        let path = Arc::clone(&db_path);
        let ready = ready_tx.clone();
        let go = Arc::clone(&go_rx);
        handles.push(std::thread::spawn(move || {
            let db = Database::open(path.as_ref()).expect("open per-worker connection");
            let session_id = format!("ses-distinct-{worker}");
            db.sessions()
                .ensure(&session_id, None)
                .expect("ensure session");
            // Synchronize the actual writes so the workers genuinely overlap.
            ready.send(()).expect("signal ready");
            go.lock().expect("lock").recv().expect("await go");
            let nonce = format!("nonce-{worker}");
            db.messages()
                .create(&session_id, "user", &nonce)
                .expect("append message under contention");
            (session_id, nonce)
        }));
    }

    for _ in 0..WORKERS {
        ready_rx.recv().expect("worker ready");
    }
    // Release all workers at once to force overlapping writes.
    for _ in 0..WORKERS {
        go_tx.send(()).expect("release worker");
    }

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.join().expect("worker thread"));
    }

    let db = Database::open(db_path.as_ref()).expect("reopen");
    let sessions = db.sessions().list().expect("list sessions");
    assert_eq!(
        sessions.len(),
        WORKERS,
        "every distinct session must be recorded exactly once"
    );
    for (session_id, nonce) in &results {
        let messages = db.messages().list_for_session(session_id).expect("list");
        assert_eq!(
            messages.len(),
            1,
            "each session holds exactly its own single message"
        );
        assert_eq!(
            messages[0].content, *nonce,
            "no cross-talk: each session carries its own nonce"
        );
    }
    assert_integrity_ok(db_path.as_ref());
}

// Many independent connections writing to the SAME session concurrently are serialized by the busy
// timeout: every append succeeds (none lost to a "database is locked" error), the message count is
// exactly the number of writers with no duplicated or truncated rows, and the database stays
// uncorrupted (VAL-RUN-012, library level).
#[test]
fn concurrent_writes_to_one_session_serialize_without_loss() {
    let scratch = Scratch::new();
    let db_path = Arc::new(scratch.path("cortex.db"));
    let session_id = "ses-shared".to_owned();
    {
        let db = Database::open(db_path.as_ref()).expect("create db");
        db.sessions()
            .ensure(&session_id, None)
            .expect("create shared session");
    }

    const WRITERS: usize = 8;
    let (ready_tx, ready_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let go_rx = Arc::new(std::sync::Mutex::new(go_rx));

    let mut handles = Vec::new();
    for writer in 0..WRITERS {
        let path = Arc::clone(&db_path);
        let session_id = session_id.clone();
        let ready = ready_tx.clone();
        let go = Arc::clone(&go_rx);
        handles.push(std::thread::spawn(move || {
            let db = Database::open(path.as_ref()).expect("open per-writer connection");
            ready.send(()).expect("signal ready");
            go.lock().expect("lock").recv().expect("await go");
            db.messages()
                .create(&session_id, "user", &format!("msg-{writer}"))
                .expect("append under same-session contention");
        }));
    }

    for _ in 0..WRITERS {
        ready_rx.recv().expect("writer ready");
    }
    for _ in 0..WRITERS {
        go_tx.send(()).expect("release writer");
    }
    for handle in handles {
        handle.join().expect("writer thread");
    }

    let db = Database::open(db_path.as_ref()).expect("reopen");
    let messages = db.messages().list_for_session(&session_id).expect("list");
    assert_eq!(
        messages.len(),
        WRITERS,
        "every concurrent append must land exactly once (no loss, no duplication)"
    );
    let mut contents: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
    contents.sort_unstable();
    contents.dedup();
    assert_eq!(
        contents.len(),
        WRITERS,
        "each writer's distinct message is present with no duplicated rows"
    );
    // Ids are unique even though the rows were written concurrently.
    let mut ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), WRITERS, "every message id is unique");
    assert_integrity_ok(db_path.as_ref());
}

// An uncommitted write left behind by an abruptly abandoned transaction is rolled back when the
// database is reopened, leaving it uncorrupted with no partial row — the in-process analogue of a
// `SIGKILL` mid-turn before the assistant reply is committed (VAL-RUN-011, library level).
#[test]
fn abandoned_uncommitted_write_leaves_db_clean_on_reopen() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let session_id = {
        let db = Database::open(&db_path).expect("open db");
        let session = db.sessions().create(None).expect("session");
        db.messages()
            .create(&session.id, "user", "committed prompt")
            .expect("committed message");
        session.id
    };

    // Open a raw connection, begin a write transaction, insert a row, then drop the connection
    // WITHOUT committing — simulating a process that died after starting a write but before the
    // commit. SQLite must discard the uncommitted work.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("raw connection");
        conn.busy_timeout(std::time::Duration::from_millis(15_000))
            .expect("busy timeout");
        conn.execute_batch("BEGIN IMMEDIATE").expect("begin");
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at) \
             VALUES (?1, ?2, 'assistant', 'never committed', datetime('now'))",
            rusqlite::params!["msg_uncommitted_phantom", session_id],
        )
        .expect("insert uncommitted row");
        // No commit: dropping the connection rolls the transaction back.
    }

    let db = Database::open(&db_path).expect("reopen after abandoned write");
    assert_eq!(
        db.integrity_check().expect("integrity"),
        "ok",
        "the database must be uncorrupted after an abandoned write"
    );
    let messages = db.messages().list_for_session(&session_id).expect("list");
    assert_eq!(
        messages.len(),
        1,
        "only the committed message survives; the uncommitted row is gone"
    );
    assert_eq!(messages[0].content, "committed prompt");
    assert!(
        messages.iter().all(|m| m.content != "never committed"),
        "no phantom row from the abandoned transaction"
    );
}
