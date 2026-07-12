//! Runtime integrity driver for the six-flow human-gate approval protocol.
//!
//! Usage:
//!   check_gate_approval [--require-approved] <evidence/v3-human-gate>

use mockup_harness_v3::gate_approval::{validate_approval_integrity, ApprovalIntegrity};

fn main() {
    let mut require_approved = false;
    let mut root = None;
    let args = std::env::args().skip(1);
    for arg in args {
        match arg.as_str() {
            "--require-approved" => require_approved = true,
            "-h" | "--help" => {
                eprintln!(
                    "usage: check_gate_approval [--require-approved] <evidence/v3-human-gate>"
                );
                std::process::exit(2);
            }
            value if root.is_none() => root = Some(value.to_owned()),
            _ => {
                eprintln!(
                    "usage: check_gate_approval [--require-approved] <evidence/v3-human-gate>"
                );
                std::process::exit(2);
            }
        }
    }
    let root = match root {
        Some(root) => root,
        None => {
            eprintln!(
                "usage: check_gate_approval [--require-approved] <evidence/v3-human-gate>"
            );
            std::process::exit(2);
        }
    };
    if !std::path::Path::new(&root).is_dir() {
        eprintln!("check_gate_approval: human-gate root does not exist: {root}");
        std::process::exit(1);
    }

    let repository = std::env::var("V3_REPO_ROOT").unwrap_or_else(|_| {
        // tools-throwaway/mockup-harness-v3 -> repo root
        "/projects/cortex-code".to_owned()
    });
    let mission = std::env::var("V3_MISSION_DIR").unwrap_or_else(|_| {
        "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc".to_owned()
    });

    match validate_approval_integrity(&repository, &mission, require_approved) {
        Ok(ApprovalIntegrity::AwaitingUser { request }) => {
            println!(
                "check_gate_approval: awaiting_user review_bundle_sha256={} request_digest={}",
                request.review_bundle_sha256, request.request_digest
            );
            if require_approved {
                eprintln!("check_gate_approval: bulk production blocked (awaiting_user)");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        Ok(ApprovalIntegrity::Approved { request, decision }) => {
            println!(
                "check_gate_approval: approved review_bundle_sha256={} actor={} message_digest={}",
                request.review_bundle_sha256, decision.user_actor, decision.user_message_digest
            );
            std::process::exit(0);
        }
        Ok(ApprovalIntegrity::Rejected {
            request,
            decision,
            affected_pilot_features,
        }) => {
            eprintln!(
                "check_gate_approval: rejected review_bundle_sha256={} actor={} affected={}",
                request.review_bundle_sha256,
                decision.user_actor,
                affected_pilot_features.join(",")
            );
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("check_gate_approval: {error}");
            std::process::exit(1);
        }
    }
}
