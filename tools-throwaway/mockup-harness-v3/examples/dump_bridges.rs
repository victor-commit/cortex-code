use mockup_harness_v3::gate_programmatic_publication::six_flow_gate_bridges;
fn main() {
  for b in six_flow_gate_bridges() {
    if b.case_id.contains("mission-start") || b.case_id.contains("mission-monitor") {
      println!("{} vis={} png={} role={:?}", b.case_id, b.vision_evidence, b.evidence_png, b.role);
    }
  }
}
