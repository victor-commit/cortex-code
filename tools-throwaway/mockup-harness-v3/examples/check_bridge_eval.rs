use mockup_harness_v3::gate_programmatic_publication::six_flow_gate_bridges;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn main() {
    let mission = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";
    for bridge in six_flow_gate_bridges() {
        if bridge.case_id != "mission-start" {
            continue;
        }
        let vision = Path::new(mission).join(bridge.vision_evidence);
        let meta = fs::metadata(&vision).expect("meta");
        let text = fs::read_to_string(&vision).expect("vision");
        println!("path={}", vision.display());
        println!("ino={} size={}", meta.ino(), meta.len());
        println!("first_session_line={}", text.lines().nth(5).unwrap_or(""));
        let mut map = std::collections::BTreeMap::new();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
        match mockup_harness_v3::gate_integration::validate_review_provenance(&map) {
            Ok(()) => println!("provenance OK"),
            Err(e) => println!("provenance ERR {e}"),
        }
    }
}
