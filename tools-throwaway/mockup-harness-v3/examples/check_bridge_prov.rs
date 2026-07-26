use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn main() {
    let paths = [
        "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc/evidence/v3-form-gate-flows-4-6-bridge-evidence/bridges/mission-start-tick-1.vision.txt",
        "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc/evidence/v3-form-gate-flows-4-6-bridge-evidence/bridges/mission-monitor-to-failure-bridge-tick-1.vision.txt",
    ];
    for path in paths {
        let text = fs::read_to_string(path).expect("read");
        let mut map = BTreeMap::new();
        for line in text.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
        match mockup_harness_v3::gate_integration::validate_review_provenance(&map) {
            Ok(()) => println!("OK {path}"),
            Err(e) => println!("ERR {path}: {e}"),
        }
        let _ = Path::new(path);
    }
}
