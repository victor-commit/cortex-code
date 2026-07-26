fn main() {
  match mockup_harness_v3::gate_vision_review::validate_gate_vision_review(
    "/projects/cortex-code",
    "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc",
  ) {
    Ok(r) => println!("OK bridges current {}", r.bridge_records.iter().filter(|b| b.record_is_current).count()),
    Err(e) => println!("ERR {e}"),
  }
}
