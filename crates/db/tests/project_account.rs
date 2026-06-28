//! Behavior tests for the project (multi-directory) and account repositories, plus the
//! session-to-project link. These back VAL-GAP-DATA-002/003 at the persistence-layer level; the
//! CLI integration tests prove the same behaviors black-box.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use db::Database;

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-db-projacct-{}-{}",
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

// VAL-GAP-DATA-003: a project supports multiple directories, the primary is listed first, and the
// model survives a reopen (durable project identity).
#[test]
fn project_multi_directory_persists_across_reopen() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let project_id = {
        let db = Database::open(&db_path).expect("open db (A)");
        let project = db
            .projects()
            .ensure("prj_key_a", Some("alpha"), "/work/alpha")
            .expect("ensure project");
        assert_eq!(project.directories, vec!["/work/alpha".to_owned()]);

        let updated = db
            .projects()
            .add_directory(&project.id, "/work/beta")
            .expect("add directory");
        assert_eq!(
            updated.directories,
            vec!["/work/alpha".to_owned(), "/work/beta".to_owned()],
            "primary stays first; the added directory follows"
        );
        project.id
    };

    let db = Database::open(&db_path).expect("open db (B)");
    let reloaded = db
        .projects()
        .get(&project_id)
        .expect("query")
        .expect("project survives restart");
    assert_eq!(
        reloaded.directories,
        vec!["/work/alpha".to_owned(), "/work/beta".to_owned()],
        "both directories survive a restart"
    );

    // Both directories resolve back to the SAME project (no cwd-hash collision).
    for dir in ["/work/alpha", "/work/beta"] {
        let found = db
            .projects()
            .find_by_directory(dir)
            .expect("find")
            .expect("directory maps to a project");
        assert_eq!(
            found.id, project_id,
            "directory `{dir}` maps to one project"
        );
    }
}

// A directory belongs to at most one project: re-homing it to another project is refused.
#[test]
fn a_directory_belongs_to_one_project() {
    let db = Database::open_in_memory().expect("memory db");
    db.projects()
        .ensure("prj_a", Some("a"), "/work/shared")
        .expect("project a");
    db.projects()
        .ensure("prj_b", Some("b"), "/work/b")
        .expect("project b");

    let err = db
        .projects()
        .add_directory("prj_b", "/work/shared")
        .expect_err("a directory cannot belong to two projects");
    assert!(
        err.to_string().contains("already belongs to project"),
        "the conflict must be explicit: {err}"
    );
}

// The primary directory cannot be removed; a secondary one can.
#[test]
fn primary_directory_is_sticky_and_secondaries_are_removable() {
    let db = Database::open_in_memory().expect("memory db");
    let project = db
        .projects()
        .ensure("prj_rm", Some("rm"), "/work/primary")
        .expect("ensure");
    db.projects()
        .add_directory(&project.id, "/work/secondary")
        .expect("add secondary");

    assert!(
        !db.projects()
            .remove_directory(&project.id, "/work/primary")
            .expect("remove primary"),
        "the primary directory cannot be removed"
    );
    assert!(
        db.projects()
            .remove_directory(&project.id, "/work/secondary")
            .expect("remove secondary"),
        "a secondary directory is removable"
    );
    assert_eq!(
        db.projects().directories(&project.id).expect("directories"),
        vec!["/work/primary".to_owned()],
        "only the primary remains"
    );
}

// VAL-GAP-DATA-003: a session retains its project identity across a restart.
#[test]
fn session_retains_project_identity_across_reopen() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let (session_id, project_id) = {
        let db = Database::open(&db_path).expect("open db (A)");
        let project = db
            .projects()
            .ensure("prj_link", Some("link"), "/work/link")
            .expect("ensure project");
        let session = db
            .sessions()
            .create(Some("linked"))
            .expect("create session");
        db.sessions()
            .set_project(&session.id, Some(&project.id))
            .expect("link session to project");
        (session.id, project.id)
    };

    let db = Database::open(&db_path).expect("open db (B)");
    assert_eq!(
        db.sessions().project_id(&session_id).expect("project id"),
        Some(project_id.clone()),
        "the session remembers its project across a restart"
    );
    assert_eq!(
        db.sessions().ids_for_project(&project_id).expect("ids"),
        vec![session_id],
        "the project lists its linked session"
    );
}

// VAL-GAP-DATA-002: an account record persists durably across a restart, keyed by provider, and
// upsert refreshes metadata while preserving the id.
#[test]
fn account_records_persist_and_upsert_by_provider() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let original_id = {
        let db = Database::open(&db_path).expect("open db (A)");
        let account = db
            .accounts()
            .upsert("openrouter", Some("OpenRouter"), Some("api"), Some("flag"))
            .expect("record account");
        assert_eq!(account.provider, "openrouter");
        assert_eq!(account.source.as_deref(), Some("flag"));
        account.id
    };

    let db = Database::open(&db_path).expect("open db (B)");
    let reloaded = db
        .accounts()
        .get_by_provider("openrouter")
        .expect("query")
        .expect("account survives restart");
    assert_eq!(reloaded.id, original_id, "the account id is stable");

    // Re-authenticating refreshes metadata but keeps the id (upsert by provider).
    let refreshed = db
        .accounts()
        .upsert(
            "openrouter",
            Some("OpenRouter"),
            Some("oauth"),
            Some("env:OPENROUTER_API_KEY"),
        )
        .expect("re-record account");
    assert_eq!(refreshed.id, original_id, "upsert preserves the id");
    assert_eq!(refreshed.auth_method.as_deref(), Some("oauth"));
    assert_eq!(refreshed.source.as_deref(), Some("env:OPENROUTER_API_KEY"));

    assert_eq!(db.accounts().list().expect("list").len(), 1, "one provider");
    assert!(
        db.accounts().remove("openrouter").expect("remove"),
        "the account is removable"
    );
    assert!(
        db.accounts()
            .get_by_provider("openrouter")
            .expect("query")
            .is_none(),
        "the removed account is gone"
    );
}

// An account never stores a raw secret: a key-shaped value in the metadata is redacted before it
// reaches the database file.
#[test]
fn account_metadata_redacts_secrets() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");
    // Assemble a key-shaped fixture from fragments so no contiguous key literal lives in source.
    let secret = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(5));

    {
        let db = Database::open(&db_path).expect("open db");
        db.accounts()
            .upsert("leaky", Some(&secret), Some("api"), Some(&secret))
            .expect("record account with secret-shaped metadata");
    }

    let on_disk = std::fs::read(&db_path).expect("read db file");
    let haystack = String::from_utf8_lossy(&on_disk);
    assert!(
        !haystack.contains(&secret),
        "a raw secret must never reach the database file"
    );
}
