use mockup_harness_v3::audit::sha256_hex;
use mockup_harness_v3::gate_integration::validate_review_provenance;
use mockup_harness_v3::gate_programmatic_publication::six_flow_gate_bridges;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn main() {
    let mission = Path::new("/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc");
    for bridge in six_flow_gate_bridges() {
        if bridge.case_id != "mission-start" && bridge.case_id != "mission-monitor-to-failure-bridge"
        {
            continue;
        }
        let png = mission.join(bridge.evidence_png);
        let sha = sha256_hex(&fs::read(&png).unwrap());
        let vision = mission.join(bridge.vision_evidence);
        let text = fs::read_to_string(&vision).unwrap();
        println!("RAW first 8 lines for {}:\n{}", bridge.case_id, text.lines().take(8).collect::<Vec<_>>().join("\n"));
        let mut map = BTreeMap::new();
        for line in text.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line.split_once('=').unwrap();
            map.insert(k.trim().to_owned(), v.trim().to_owned());
        }
        println!(" map review_session_id={}", map.get("review_session_id").unwrap());
        match validate_review_provenance(&map) {
            Ok(()) => println!(" provenance OK"),
            Err(e) => println!(" provenance ERR {e}"),
        }
        let _ = sha;
    }
}
