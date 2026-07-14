//! One-shot immutable awaiting_user request writer for pre_bulk continuity.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let repo = args
        .get(0)
        .map(String::as_str)
        .unwrap_or("/projects/cortex-code");
    let mission = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc");
    match mockup_harness_v3::gate_approval::write_human_gate_request(repo, mission, None) {
        Ok(req) => {
            println!("review_bundle_sha256={}", req.review_bundle_sha256);
            println!("request_digest={}", req.request_digest);
            println!("pre_bulk_tree_sha256={}", req.pre_bulk_tree.tree_sha256);
            println!("status={}", req.status);
            println!("entry_count={}", req.pre_bulk_tree.entry_count);
            println!("relative_path={}", req.relative_path);
        }
        Err(err) => {
            eprintln!("write_human_gate_request failed: {err}");
            std::process::exit(1);
        }
    }
}
