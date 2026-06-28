//! Monotonic, lexically time-sortable identifier generation.
//!
//! Identifiers are `{{prefix}}_{{32 hex chars}}`: a 16-hex monotonic clock component followed by a
//! 16-hex random tail. The clock component is a single strictly-increasing 64-bit value (a
//! millisecond timestamp shifted to leave room for a within-millisecond counter), so a plain
//! lexicographic sort of two ids generated in the same process reproduces their creation order even
//! when many are minted inside one millisecond. The random tail (64 bits drawn from SQLite's
//! CSPRNG) preserves cross-process collision resistance without ordering guarantees across
//! processes.
//!
//! The documented prefix taxonomy lives in [`prefix`]; every persisted entity that mints an id uses
//! one of those constants so the prefix identifies the entity kind on sight.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::error::DbResult;

/// Documented identifier prefixes, one per persisted entity kind.
///
/// The prefix is the leading token before the first `_` of every generated id. Surfaces and SDK
/// clients can recognize an id's entity kind from this prefix alone.
pub mod prefix {
    /// A conversation session (`ses_…`).
    pub const SESSION: &str = "ses";
    /// A message stored under a session (`msg_…`).
    pub const MESSAGE: &str = "msg";
    /// The short code for a message part. Persisted message parts do not mint a standalone id;
    /// they derive a hierarchical id from their owning message (`<message_id>_pNNNN`) so a part
    /// sorts with — and is attributable to — its message, while engine streaming parts use
    /// `part_<turn>_<kind>` ids. This constant names the part entity kind for surfaces/SDKs.
    pub const PART: &str = "prt";
    /// A per-turn token-usage record (`usg_…`).
    pub const USAGE: &str = "usg";
    /// A todo item (`tod_…`).
    pub const TODO: &str = "tod";
    /// A provider/plugin auth account (`acc_…`).
    pub const ACCOUNT: &str = "acc";
    /// A project grouping one or more directories (`prj_…`).
    ///
    /// A project id is directory-derived (a stable workspace key) rather than time-minted, so it is
    /// stable across runs rather than time-sortable; the prefix still identifies the entity kind.
    pub const PROJECT: &str = "prj";
}

/// Bits reserved at the bottom of the monotonic clock value for a within-millisecond counter.
/// 2^20 ≈ 1.05M ids can share a single millisecond before the clock borrows from the next
/// millisecond's space (ordering is always preserved; only the embedded time drifts forward under
/// that — practically unreachable — load).
const COUNTER_BITS: u32 = 20;

/// The process-global monotonic clock. Holds the last `(timestamp_ms << COUNTER_BITS) | counter`
/// value handed out, so each new id strictly exceeds every prior id minted in this process.
static MONOTONIC: AtomicU64 = AtomicU64::new(0);

/// Current Unix time in milliseconds, saturating at 0 if the clock is before the epoch.
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Return the next strictly-increasing monotonic clock value.
///
/// Combines the current millisecond timestamp with a within-millisecond counter into one 64-bit
/// value that never decreases or repeats for the life of the process, using a lock-free
/// compare-and-swap so concurrent callers stay correctly ordered without a mutex.
fn next_monotonic() -> u64 {
    let now = now_millis() << COUNTER_BITS;
    loop {
        let prev = MONOTONIC.load(Ordering::Acquire);
        let next = if now > prev { now } else { prev + 1 };
        if MONOTONIC
            .compare_exchange_weak(prev, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return next;
        }
    }
}

/// Generate a prefixed, lexically time-sortable identifier (e.g. `ses_<32 hex chars>`).
///
/// The first 16 hex chars encode the monotonic clock (sortable); the last 16 encode 64 bits of
/// SQLite-CSPRNG randomness (collision resistance). Two ids minted in this process always sort by
/// creation order.
pub(crate) fn random_id(conn: &Connection, prefix: &str) -> DbResult<String> {
    let clock = next_monotonic();
    let random: String =
        conn.query_row("SELECT lower(hex(randomblob(8)))", [], |row| row.get(0))?;
    Ok(format!("{prefix}_{clock:016x}{random}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_conn() -> Connection {
        Connection::open_in_memory().expect("open in-memory sqlite")
    }

    #[test]
    fn ids_carry_their_prefix_and_fixed_width_hex_body() {
        let conn = memory_conn();
        let id = random_id(&conn, prefix::SESSION).expect("id");
        let (head, body) = id.split_once('_').expect("prefix separator");
        assert_eq!(head, "ses");
        assert_eq!(body.len(), 32, "16 hex clock + 16 hex random: {id}");
        assert!(
            body.chars().all(|c| c.is_ascii_hexdigit()),
            "body is hex: {id}"
        );
    }

    #[test]
    fn sequential_ids_sort_lexicographically_by_creation_order() {
        let conn = memory_conn();
        let mut ids = Vec::new();
        for _ in 0..2_000 {
            ids.push(random_id(&conn, prefix::MESSAGE).expect("id"));
        }
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(
            ids, sorted,
            "lexicographic order must equal creation order even within one process"
        );
    }

    #[test]
    fn generated_ids_are_unique() {
        let conn = memory_conn();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..5_000 {
            assert!(
                seen.insert(random_id(&conn, prefix::USAGE).expect("id")),
                "ids must not collide"
            );
        }
    }

    #[test]
    fn monotonic_clock_strictly_increases_across_threads() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;

        let collisions = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(thread::spawn(|| {
                let mut local = Vec::with_capacity(1_000);
                for _ in 0..1_000 {
                    local.push(next_monotonic());
                }
                local
            }));
        }
        let mut all = Vec::new();
        for handle in handles {
            all.extend(handle.join().expect("join"));
        }
        let mut deduped = all.clone();
        deduped.sort_unstable();
        deduped.dedup();
        if deduped.len() != all.len() {
            collisions.store(true, Ordering::SeqCst);
        }
        assert!(
            !collisions.load(Ordering::SeqCst),
            "the monotonic clock must hand out a unique value to every concurrent caller"
        );
    }
}
