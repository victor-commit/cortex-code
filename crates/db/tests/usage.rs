//! Behavior tests for the usage repository: per-turn recording, session-level cumulative totals
//! (monotonic, never reset to a single turn), durability across re-open, and the not-found guard.
//! These back the session-level usage/cost reporting the CLI surfaces (VAL-LLM-031) at the library
//! level; the CLI live tests prove the same behavior black-box end-to-end.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use db::{Database, DbError};

/// A self-cleaning scratch directory under the system temp dir.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-db-usage-test-{}-{}",
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

// A session with no recorded usage reports all-zero token totals and no cost.
#[test]
fn empty_session_totals_are_zero() {
    let db = Database::open_in_memory().expect("memory db");
    let session = db.sessions().create(None).expect("create session");

    let totals = db.usage().session_totals(&session.id).expect("totals");
    assert_eq!(totals.input_tokens, 0);
    assert_eq!(totals.output_tokens, 0);
    assert_eq!(totals.total_tokens, 0);
    assert_eq!(totals.cost, None);
}

// Recording a single turn round-trips its token counts and cost, and the session total mirrors it.
#[test]
fn recording_one_turn_round_trips_and_totals_match() {
    let db = Database::open_in_memory().expect("memory db");
    let session = db.sessions().create(None).expect("create session");

    let record = db
        .usage()
        .record(&session.id, 124, 8, 132, Some(0.000_039_7))
        .expect("record usage");
    assert_eq!(record.session_id, session.id);
    assert_eq!(record.input_tokens, 124);
    assert_eq!(record.output_tokens, 8);
    assert_eq!(record.total_tokens, 132);
    assert_eq!(record.cost, Some(0.000_039_7));
    assert!(
        record.id.starts_with("usg_"),
        "id is prefixed: {}",
        record.id
    );

    let totals = db.usage().session_totals(&session.id).expect("totals");
    assert_eq!(totals.input_tokens, 124);
    assert_eq!(totals.output_tokens, 8);
    assert_eq!(totals.total_tokens, 132);
    assert_eq!(totals.cost, Some(0.000_039_7));
}

// The core of VAL-LLM-031 at the library level: session totals accumulate across turns, so the
// cumulative figure after the second turn is strictly greater than after the first and equals the
// sum of the per-turn counts (never reset to a single turn's value).
#[test]
fn session_totals_accumulate_monotonically_across_turns() {
    let db = Database::open_in_memory().expect("memory db");
    let session = db.sessions().create(None).expect("create session");

    db.usage()
        .record(&session.id, 124, 8, 132, Some(0.000_04))
        .expect("turn 1");
    let after_one = db.usage().session_totals(&session.id).expect("totals 1");
    assert_eq!(after_one.total_tokens, 132);
    assert_eq!(after_one.cost, Some(0.000_04));

    db.usage()
        .record(&session.id, 300, 50, 350, Some(0.000_11))
        .expect("turn 2");
    let after_two = db.usage().session_totals(&session.id).expect("totals 2");

    assert_eq!(after_two.input_tokens, 424, "input sums across turns");
    assert_eq!(after_two.output_tokens, 58, "output sums across turns");
    assert_eq!(
        after_two.total_tokens, 482,
        "total is the accumulated per-turn sum (132 + 350)"
    );
    assert!(
        after_two.total_tokens > after_one.total_tokens,
        "cumulative total must grow across turns"
    );
    assert!(
        after_two.cost.unwrap() > after_one.cost.unwrap(),
        "cumulative cost must grow across turns"
    );
    // Within reporting tolerance, the cumulative cost is the sum of the per-turn costs.
    assert!((after_two.cost.unwrap() - 0.000_15).abs() < 1e-12);
}

// A turn that reports no cost (cost = None) still contributes its token counts; the session cost is
// the sum of only the turns that reported a cost.
#[test]
fn missing_costs_are_skipped_in_the_session_cost_sum() {
    let db = Database::open_in_memory().expect("memory db");
    let session = db.sessions().create(None).expect("create session");

    db.usage()
        .record(&session.id, 10, 2, 12, None)
        .expect("turn with no cost");
    let only_none = db.usage().session_totals(&session.id).expect("totals");
    assert_eq!(only_none.total_tokens, 12);
    assert_eq!(
        only_none.cost, None,
        "with no priced turn the session cost is absent, not zero"
    );

    db.usage()
        .record(&session.id, 20, 4, 24, Some(0.000_02))
        .expect("priced turn");
    let mixed = db.usage().session_totals(&session.id).expect("totals");
    assert_eq!(mixed.total_tokens, 36, "token counts still accumulate");
    assert_eq!(
        mixed.cost,
        Some(0.000_02),
        "the session cost sums only the priced turns"
    );
}

// Recorded usage is durable: totals computed after re-opening the database equal what was recorded
// in a prior "process".
#[test]
fn usage_totals_persist_across_reopen() {
    let scratch = Scratch::new();
    let db_path = scratch.path("cortex.db");

    let session_id = {
        let db = Database::open(&db_path).expect("open db (A)");
        let session = db.sessions().create(None).expect("create session");
        db.usage()
            .record(&session.id, 100, 20, 120, Some(0.0001))
            .expect("turn 1");
        db.usage()
            .record(&session.id, 200, 40, 240, Some(0.0002))
            .expect("turn 2");
        session.id
    };

    let db = Database::open(&db_path).expect("open db (B)");
    let totals = db.usage().session_totals(&session_id).expect("totals");
    assert_eq!(totals.input_tokens, 300);
    assert_eq!(totals.output_tokens, 60);
    assert_eq!(totals.total_tokens, 360);
    assert!(
        (totals.cost.unwrap() - 0.000_3).abs() < 1e-12,
        "cumulative cost survives re-open, got {:?}",
        totals.cost
    );
}

// Usage is partitioned by session: one session's totals never include another's turns.
#[test]
fn usage_is_isolated_per_session() {
    let db = Database::open_in_memory().expect("memory db");
    let a = db.sessions().create(None).expect("session a");
    let b = db.sessions().create(None).expect("session b");

    db.usage()
        .record(&a.id, 100, 10, 110, Some(0.001))
        .expect("a turn");
    db.usage()
        .record(&b.id, 5, 1, 6, Some(0.000_01))
        .expect("b turn");

    let a_totals = db.usage().session_totals(&a.id).expect("a totals");
    let b_totals = db.usage().session_totals(&b.id).expect("b totals");
    assert_eq!(a_totals.total_tokens, 110);
    assert_eq!(b_totals.total_tokens, 6);
    assert_eq!(a_totals.cost, Some(0.001));
    assert_eq!(b_totals.cost, Some(0.000_01));
}

// Recording usage under an unknown session is a clear NotFound, not an opaque constraint failure.
#[test]
fn record_under_unknown_session_is_a_clear_not_found() {
    let db = Database::open_in_memory().expect("memory db");
    let err = db
        .usage()
        .record("ses_does_not_exist", 1, 1, 2, Some(0.0))
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
