//! Behavior tests for the persistence layer: migrations, durability, repositories, and the
//! error classification for junk/unwritable paths. These back VAL-SKEL-037..042 and VAL-SKEL-068
//! at the library level; the CLI integration tests prove the same behaviors black-box.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use db::{Database, DbError, latest_version};

/// A self-cleaning scratch directory under the system temp dir.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-db-test-{}-{}",
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

/// Read the set of user table names directly from `sqlite_master`.
fn table_names(db: &Database) -> Vec<String> {
    // A read-only probe via a fresh connection so the test asserts on-disk schema, not cache.
    let conn =
        rusqlite::Connection::open(db.path()).expect("open probe connection to the same file");
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("prepare table query");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query tables")
        .map(|r| r.expect("row"))
        .collect()
}

// VAL-SKEL-037: a fresh DB is created with migrations applied and the expected tables present.
#[test]
fn fresh_database_has_expected_tables_and_version() {
    let scratch = Scratch::new();
    let db_path = scratch.path("nested/dir/cortex.db");
    assert!(!db_path.exists(), "precondition: DB path is fresh");

    let db = Database::open(&db_path).expect("open fresh db");

    assert!(db_path.exists(), "the DB file is created on first open");
    assert_eq!(db.schema_version().unwrap(), latest_version());

    let tables = table_names(&db);
    for expected in ["sessions", "messages", "schema_migrations"] {
        assert!(
            tables.iter().any(|t| t == expected),
            "expected table `{expected}` in {tables:?}"
        );
    }
}

// VAL-SKEL-040: re-opening an already-migrated DB is idempotent (stable version, no error).
#[test]
fn reopening_is_idempotent_and_preserves_version() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    for _ in 0..3 {
        let db = Database::open(&db_path).expect("reopen migrated db");
        assert_eq!(db.schema_version().unwrap(), latest_version());
        // Exactly one row per applied migration; never re-applied.
        let conn = rusqlite::Connection::open(db.path()).unwrap();
        let migration_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(migration_rows, latest_version());
    }
}

// VAL-SKEL-038: a session created in one "process" is retrievable after re-open.
#[test]
fn session_persists_across_reopen() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let session_id = {
        let db = Database::open(&db_path).expect("open db (A)");
        let session = db.sessions().create(Some("first run")).expect("create");
        session.id
    };

    let db = Database::open(&db_path).expect("open db (B)");
    let reloaded = db
        .sessions()
        .get(&session_id)
        .expect("query")
        .expect("session survives restart");
    assert_eq!(reloaded.id, session_id);
    assert_eq!(reloaded.title.as_deref(), Some("first run"));

    let listed = db.sessions().list().expect("list");
    assert!(listed.iter().any(|s| s.id == session_id));
}

// VAL-SKEL-039: a stored message (with content) is retrievable after re-open.
#[test]
fn message_content_persists_across_reopen() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let (session_id, content) = {
        let db = Database::open(&db_path).expect("open db (A)");
        let session = db.sessions().create(None).expect("create session");
        let content = "remember the word: marmalade";
        db.messages()
            .create(&session.id, "user", content)
            .expect("store message");
        (session.id, content.to_owned())
    };

    let db = Database::open(&db_path).expect("open db (B)");
    let messages = db.messages().list_for_session(&session_id).expect("list");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, content);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].session_id, session_id);
}

#[test]
fn message_under_unknown_session_is_a_clear_not_found() {
    let db = Database::open_in_memory().expect("memory db");
    let err = db
        .messages()
        .create("ses_does_not_exist", "user", "hi")
        .unwrap_err();
    assert!(matches!(
        err,
        DbError::NotFound {
            kind: "session",
            ..
        }
    ));
    assert!(err.to_string().contains("ses_does_not_exist"));
}

// VAL-SKEL-042 (junk file): a non-SQLite file yields NotADatabase, not a panic.
#[test]
fn junk_file_is_classified_as_not_a_database() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");
    std::fs::write(
        &db_path,
        b"this is plainly not a sqlite database header\n".repeat(8),
    )
    .expect("write junk");

    let err = Database::open(&db_path).unwrap_err();
    assert!(
        matches!(err, DbError::NotADatabase { .. }),
        "expected NotADatabase, got: {err:?}"
    );
    assert!(err.to_string().contains("not a valid SQLite database"));
}

// `ensure` creates a session under a caller-supplied id and is idempotent: a second ensure with
// the same id returns the same row (no duplicate, no primary-key error), so an external id like a
// `--session` value durably resumes across invocations.
#[test]
fn ensure_creates_then_resumes_an_explicit_id() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let explicit = "val-1234-explicit-id";
    {
        let db = Database::open(&db_path).expect("open db (A)");
        let created = db.sessions().ensure(explicit, None).expect("ensure create");
        assert_eq!(created.id, explicit, "the supplied id is used verbatim");
        db.messages()
            .create(explicit, "user", "remember marmalade")
            .expect("store message under the ensured session");
    }

    let db = Database::open(&db_path).expect("open db (B)");
    let resumed = db.sessions().ensure(explicit, None).expect("ensure resume");
    assert_eq!(resumed.id, explicit);
    assert_eq!(
        resumed.created_at, // unchanged: the existing row is reused, not recreated
        db.sessions().require(explicit).expect("require").created_at
    );
    let messages = db.messages().list_for_session(explicit).expect("list");
    assert_eq!(
        messages.len(),
        1,
        "ensuring an existing id must not wipe its messages"
    );
    assert_eq!(messages[0].content, "remember marmalade");

    let sessions = db.sessions().list().expect("list sessions");
    assert_eq!(
        sessions.iter().filter(|s| s.id == explicit).count(),
        1,
        "ensure is idempotent: exactly one row for the id"
    );
}

// `most_recent` returns the session with the latest activity (advanced by appending a message),
// which is what a `--continue` run attaches to; an empty database yields `None`.
#[test]
fn most_recent_tracks_the_latest_active_session() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");
    let db = Database::open(&db_path).expect("open db");

    assert!(
        db.sessions().most_recent().expect("most_recent").is_none(),
        "an empty database has no most-recent session"
    );

    let first = db.sessions().create(Some("first")).expect("create first");
    let second = db.sessions().create(Some("second")).expect("create second");

    // Appending to `first` makes it the most recently active session even though `second` was
    // created later, proving activity (not creation) drives the ordering.
    db.messages()
        .create(&first.id, "user", "later activity on the first session")
        .expect("append to first");

    let recent = db
        .sessions()
        .most_recent()
        .expect("most_recent")
        .expect("a session exists");
    assert_eq!(
        recent.id, first.id,
        "the most recently written-to session wins, got {recent:?} (second was {second:?})"
    );
}

// Messages written within the same clock second are returned in true insertion order (user before
// assistant), not by the random hex id, so projected history stays coherent.
#[test]
fn messages_in_one_turn_keep_insertion_order() {
    let db = Database::open_in_memory().expect("memory db");
    let session = db.sessions().create(None).expect("create");

    // Both inserts land in the same second; only a rowid (insertion) tiebreak keeps them ordered.
    db.messages()
        .create(&session.id, "user", "Remember the word: marmalade.")
        .expect("user message");
    db.messages()
        .create(&session.id, "assistant", "Acknowledged.")
        .expect("assistant message");

    let messages = db.messages().list_for_session(&session.id).expect("list");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user", "the user turn must come first");
    assert_eq!(messages[1].role, "assistant", "the reply must come second");
}

// Renaming a session updates its title and persists across reopen; clearing the title is supported.
#[test]
fn set_title_renames_a_session_and_persists() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let id = {
        let db = Database::open(&db_path).expect("open db (A)");
        let session = db.sessions().create(Some("before")).expect("create");
        let renamed = db
            .sessions()
            .set_title(&session.id, Some("after"))
            .expect("rename");
        assert_eq!(renamed.title.as_deref(), Some("after"), "title updated");
        session.id
    };

    let db = Database::open(&db_path).expect("open db (B)");
    let reloaded = db.sessions().require(&id).expect("require");
    assert_eq!(
        reloaded.title.as_deref(),
        Some("after"),
        "the rename survives a reopen"
    );

    // Passing None clears the title.
    let cleared = db.sessions().set_title(&id, None).expect("clear title");
    assert_eq!(cleared.title, None, "None clears the title");
}

#[test]
fn set_title_on_unknown_session_is_not_found() {
    let db = Database::open_in_memory().expect("memory db");
    let err = db
        .sessions()
        .set_title("ses_does_not_exist", Some("x"))
        .unwrap_err();
    assert!(matches!(
        err,
        DbError::NotFound {
            kind: "session",
            ..
        }
    ));
}

// VAL-CROSS-006 (persistence half): a session's chosen model is stored and survives a reopen, so a
// later headless resume can read it back. A fresh session has no model; passing None clears it.
#[test]
fn set_model_persists_a_per_session_model_across_reopen() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let id = {
        let db = Database::open(&db_path).expect("open db (A)");
        let session = db.sessions().create(None).expect("create");
        assert_eq!(session.model, None, "a fresh session has no model");
        let chosen = db
            .sessions()
            .set_model(&session.id, Some("anthropic/claude-sonnet-4"))
            .expect("set model");
        assert_eq!(
            chosen.model.as_deref(),
            Some("anthropic/claude-sonnet-4"),
            "the model is recorded"
        );
        session.id
    };

    let db = Database::open(&db_path).expect("open db (B)");
    let reloaded = db.sessions().require(&id).expect("require");
    assert_eq!(
        reloaded.model.as_deref(),
        Some("anthropic/claude-sonnet-4"),
        "the chosen model survives a reopen"
    );
    // It is also visible through the listing path the TUI picker uses.
    let listed = db.sessions().list().expect("list");
    assert_eq!(
        listed
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.model.as_deref()),
        Some("anthropic/claude-sonnet-4"),
        "the listing exposes the per-session model"
    );

    let cleared = db.sessions().set_model(&id, None).expect("clear model");
    assert_eq!(cleared.model, None, "None clears the per-session model");
}

#[test]
fn set_model_on_unknown_session_is_not_found() {
    let db = Database::open_in_memory().expect("memory db");
    let err = db
        .sessions()
        .set_model("ses_does_not_exist", Some("google/gemini-2.5-flash"))
        .unwrap_err();
    assert!(matches!(
        err,
        DbError::NotFound {
            kind: "session",
            ..
        }
    ));
}

// Deleting a session removes it from the listing and cascades to its messages; deleting an unknown
// id is a graceful `false` rather than an error.
#[test]
fn delete_removes_a_session_and_cascades_to_messages() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let db = Database::open(&db_path).expect("open db");
    let keep = db.sessions().create(Some("keep")).expect("create keep");
    let drop = db.sessions().create(Some("drop")).expect("create drop");
    db.messages()
        .create(&drop.id, "user", "remember marmalade")
        .expect("message under the doomed session");

    let removed = db.sessions().delete(&drop.id).expect("delete");
    assert!(removed, "an existing session reports a removal");

    let listed = db.sessions().list().expect("list");
    assert!(
        listed.iter().any(|s| s.id == keep.id),
        "the surviving session remains"
    );
    assert!(
        !listed.iter().any(|s| s.id == drop.id),
        "the deleted session is gone from the listing"
    );
    assert!(
        db.sessions().get(&drop.id).expect("get").is_none(),
        "the deleted session is unfetchable"
    );
    assert!(
        db.messages()
            .list_for_session(&drop.id)
            .expect("list messages")
            .is_empty(),
        "the deleted session's messages cascade away"
    );

    let absent = db.sessions().delete("ses_never_existed").expect("delete");
    assert!(!absent, "deleting an unknown id is a graceful false");
}

// VAL-SKEL-042 (unwritable path): a path whose parent is a file cannot be created → Open error.
#[test]
fn unwritable_path_yields_open_error() {
    let scratch = Scratch::new();
    let blocker = scratch.path("a-file");
    std::fs::write(&blocker, b"i am a file, not a directory").expect("write blocker file");
    // The parent component `a-file` is a regular file, so the data dir cannot be created.
    let db_path = blocker.join("cortex.db");

    let err = Database::open(&db_path).unwrap_err();
    assert!(
        matches!(err, DbError::Open { .. }),
        "expected Open error, got: {err:?}"
    );
}

// VAL-SKEL-068 (best-effort, in-process): concurrent opens of one fresh path each migrate the
// DB without error; afterwards the version is the single expected value with one row per
// migration. True cross-process concurrency is proven by the CLI integration test.
#[test]
fn concurrent_opens_of_a_fresh_db_all_succeed() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let mut handles = Vec::new();
    for _ in 0..8 {
        let path = db_path.clone();
        handles.push(std::thread::spawn(move || {
            Database::open(&path).map(|db| db.schema_version().unwrap())
        }));
    }
    for handle in handles {
        let version = handle
            .join()
            .expect("thread")
            .expect("open under contention");
        assert_eq!(version, latest_version());
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let migration_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        migration_rows,
        latest_version(),
        "migrations applied exactly once despite concurrency"
    );
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}
