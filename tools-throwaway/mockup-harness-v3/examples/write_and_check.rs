use mockup_harness_v3::gate_integration::validate_review_provenance;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn main() {
    let mission = Path::new("/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc");
    let vis = mission.join("evidence/v3-form-gate-flows-4-6-bridge-evidence/bridges/mission-start-tick-1.vision.txt");
    let text = fs::read_to_string(&vis).unwrap();
    let meta = fs::metadata(&vis).unwrap();
    println!("single-process ino={} size={} line5={}", meta.ino(), meta.len(), text.lines().nth(5).unwrap());
    let mut map = BTreeMap::new();
    for line in text.lines() {
        if let Some((k,v)) = line.split_once('=') {
            map.insert(k.trim().to_owned(), v.trim().to_owned());
        }
    }
    match validate_review_provenance(&map) {
        Ok(()) => println!("provenance OK"),
        Err(e) => println!("provenance ERR {e}"),
    }
}
