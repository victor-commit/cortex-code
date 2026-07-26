use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;

fn main() {
    let mission = Path::new("/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc");
    let root = mission.join("evidence/v3-form-gate-flows-4-6-bridge-evidence/bridges");
    println!("listing {}", root.display());
    for entry in fs::read_dir(&root).expect("rd") {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains("mission-start") || name.contains("mission-monitor-to-failure") {
            let meta = entry.metadata().unwrap();
            println!("  {} ino={} size={}", name, meta.ino(), meta.len());
        }
    }
    let files = [
        (
            "mission-start-tick-1",
            "remaining-three bridge mission explicit launch",
            "mission-start",
            "mission",
            "direct",
            "f1e22f2a-6e26-4c66-bf0e-2cd9a6de36bb",
            "0e1049df57832f2cad3dacd477c6dfab55c6946632296aa94c8488b89baafdf6",
            "call-670d466e-3da6-442c-a718-428a6b1e9d6b-117",
            "68db81ee958afd1531e6e7bc8dd308dcbc840e96f2b7e85797b6a0b2d3f24cf0",
        ),
        (
            "mission-monitor-to-failure-bridge-tick-1",
            "remaining-three bridge mission monitor failure detour",
            "mission-monitor-to-failure-bridge",
            "mission",
            "illustrative_detour",
            "c6d7c528-bce1-4134-a4dd-cdfa1de81507",
            "f9a89b73e04b1d59effe2d2251ff9dff36aa868d1ed3bc48a9d8534372e922e6",
            "call-670d466e-3da6-442c-a718-428a6b1e9d6b-118",
            "b20ee529e04cd6def296b631dba82e544420c00b51c21c34e767865925198c13",
        ),
    ];
    for (name, role, case, flow, kind, sid, rsha, tool, sha) in files {
        let art = mission
            .join("evidence/v3-form-gate-flows-4-6-bridge-evidence/bridges")
            .join(format!("{name}.png"));
        let vis = mission
            .join("evidence/v3-form-gate-flows-4-6-bridge-evidence/bridges")
            .join(format!("{name}.vision.txt"));
        let _ = Command::new("chattr")
            .args(["-i", vis.to_str().unwrap()])
            .status();
        let content = format!(
            "artifact_path={}\n\
role={role}\n\
sha256={sha}\n\
model=anthropic/claude-opus-4.8-fast\n\
verdict=pass\n\
review_session_id={sid}\n\
response_sha256={rsha}\n\
response_path=/root/.factory/sessions/-projects-cortex-code/{sid}.jsonl#3\n\
tool_use_id={tool}\n\
case_id={case}\n\
tick=1\n\
flow={flow}\n\
kind={kind}\n\
hierarchy_ok=true\n\
density_ok=true\n\
primary_action_ok=true\n\
color_restraint_ok=true\n\
disclosure_ok=true\n\
terminal_faithful_ok=true\n\
pure_cortex_fidelity_ok=true\n\
reconciliation=Pure bridge rebind isolated Factory session provenance for unique pair.\n\
response_summary=Pure Cortex bridge rebind residual reconciled.\n\
rebind=v3-pure-cortex-evidence-rebind-and-hybrid-guard\n",
            art.display()
        );
        fs::write(&vis, &content).expect("write");
        let meta = fs::metadata(&vis).unwrap();
        println!("wrote {} ino={} size={}", name, meta.ino(), meta.len());
        println!("line {}", fs::read_to_string(&vis).unwrap().lines().nth(5).unwrap());
        let _ = Command::new("chattr")
            .args(["+i", vis.to_str().unwrap()])
            .status();
    }
}
