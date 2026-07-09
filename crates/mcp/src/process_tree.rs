//! Terminate a spawned stdio MCP server and every descendant it itself spawned, by exact PID.
//!
//! Cancelling an `rmcp` session kills the direct child the transport spawned, but a server that
//! forks its own helper processes would otherwise leave those grandchildren orphaned. On shutdown
//! the manager enumerates the full descendant set from `/proc` *before* tearing the session down
//! (so the parent/child links are still intact and nothing has been reparented to init yet), then
//! sends each collected PID a `SIGKILL` by exact PID — never a name-based kill — so no orphan
//! survives.

/// Collect `root` together with every transitive descendant currently visible in `/proc`.
///
/// The returned vector always contains `root` first, followed by its descendants in breadth-first
/// order. On a platform without `/proc` (or when the tables cannot be read) only `root` is returned.
#[must_use]
pub fn process_tree(root: u32) -> Vec<u32> {
    let mut tree = vec![root];
    let children = match read_children_map() {
        Some(map) => map,
        None => return tree,
    };

    let mut frontier = vec![root];
    while let Some(pid) = frontier.pop() {
        if let Some(kids) = children.get(&pid) {
            for &kid in kids {
                if !tree.contains(&kid) {
                    tree.push(kid);
                    frontier.push(kid);
                }
            }
        }
    }
    tree
}

/// Kill each of `pids` by exact PID with `SIGKILL`, in reverse order so a parent is killed after
/// its children. Pass a snapshot taken with [`process_tree`] *before* tearing the session down, so
/// reparented grandchildren are still reached.
pub async fn kill_all(pids: &[u32]) {
    for &pid in pids.iter().rev() {
        kill_pid(pid).await;
    }
}

/// Synchronously kill a previously captured process tree by exact PID.
///
/// This is used by the stdio connection guard's [`Drop`] implementation. A cancelled async
/// connection future cannot await cleanup, but it must still reap the child it already spawned
/// before unwinding on a lifecycle signal.
pub fn kill_all_now(pids: &[u32]) {
    for &pid in pids.iter().rev() {
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Whether `pid` is still present in the process table (Linux `/proc`).
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Send `SIGKILL` to a single PID via the `kill` utility (an explicit-PID, never name-based, kill).
async fn kill_pid(pid: u32) {
    let _ = tokio::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
}

/// Build a parent-PID → child-PIDs map from `/proc/<pid>/stat`, or `None` when `/proc` is absent.
fn read_children_map() -> Option<std::collections::HashMap<u32, Vec<u32>>> {
    let entries = std::fs::read_dir("/proc").ok()?;
    let mut map: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if let Some(ppid) = read_ppid(pid) {
            map.entry(ppid).or_default().push(pid);
        }
    }
    Some(map)
}

/// Parse the parent PID (field 4) out of `/proc/<pid>/stat`.
///
/// The `comm` field (field 2) is wrapped in parentheses and may itself contain spaces or `)`, so
/// the fields after it are read starting from the final `)`.
fn read_ppid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rfind(')').map(|idx| &stat[idx + 1..])?;
    // After `)` the remaining fields are: " <state> <ppid> ...".
    let mut fields = after_comm.split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse::<u32>().ok()
}
