//! Proof that the `CORTEX_NO_NET` outbound guard short-circuits the provider before any socket
//! opens (fail-closed), exercised at the `llm` layer.
//!
//! The environment must be set at process start, and the workspace forbids `unsafe` (so in-process
//! `std::env::set_var` is not allowed). This test therefore re-invokes its own test binary as a
//! child with `CORTEX_NO_NET=1` and a credential present; the child runs the real provider and
//! reports the normalized error class on stdout. The parent asserts it is `net-disabled` — i.e. a
//! present credential cannot let the request slip past the kill-switch.

use std::process::Command;

const PROBE_ENV: &str = "CORTEX_LLM_NET_PROBE";
const MARKER: &str = "NET_GUARD_RESULT_KIND=";

#[test]
fn cortex_no_net_short_circuits_provider_before_socket() {
    if std::env::var_os(PROBE_ENV).is_some() {
        run_probe();
        return;
    }

    let exe = std::env::current_exe().expect("path to this test binary");
    let output = Command::new(exe)
        .args([
            "--exact",
            "cortex_no_net_short_circuits_provider_before_socket",
            "--nocapture",
        ])
        .env("CORTEX_NO_NET", "1")
        .env(PROBE_ENV, "1")
        // A credential IS present: fail-closed ordering means the kill-switch still wins.
        .env("OPENROUTER_API_KEY", "key-present-but-must-not-be-used")
        .output()
        .expect("spawn child probe");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("{MARKER}net-disabled")),
        "child must report a net-disabled refusal (fail-closed). stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The child-process body: run the real provider under `CORTEX_NO_NET=1` and print the error class.
fn run_probe() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    runtime.block_on(async {
        use llm::{LlmRequest, OpenAiCompatible, OpenAiConfig, Provider};

        let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
        let provider =
            OpenAiCompatible::new(OpenAiConfig::openrouter(api_key)).expect("build provider");
        let result = provider
            .stream(LlmRequest::new("google/gemini-2.5-flash").user("hi"))
            .await;
        match result {
            Ok(_) => println!("{MARKER}unexpected-ok"),
            Err(err) => println!("{MARKER}{}", err.kind()),
        }
    });
}
