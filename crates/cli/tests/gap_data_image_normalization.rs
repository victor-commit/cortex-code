//! VAL-GAP-DATA-009: image normalization resizes and bounds media before model upload.
//!
//! The offline test generates an oversized PNG, references it from a `cortex run` prompt, and asserts
//! the run reports the downscaled dimensions before any provider upload (the turn itself then fails
//! closed under `CORTEX_NO_NET`, which is fine — normalization happens first). A small in-bounds
//! image is reported as passing through unchanged. The live test (gated on `OPENROUTER_API_KEY`)
//! sends the normalized oversized image to a real model and asserts a real answer comes back.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cortex-img-norm-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).expect("home");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn db_path(&self) -> PathBuf {
        self.root.join("cortex.db")
    }

    fn command(&self, args: &[&str], net: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.current_dir(&self.root);
        command.env("HOME", self.home());
        command.env("CORTEX_DB_PATH", self.db_path());
        command.env("PATH", std::env::var("PATH").unwrap_or_default());
        if !net {
            command.env("CORTEX_NO_NET", "1");
        }
        if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
            command.env("OPENROUTER_API_KEY", key);
        }
        command.args(args);
        command
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Encode a solid-color PNG of the requested size to `path`.
fn write_png(path: &std::path::Path, width: u32, height: u32) {
    let buffer = image::RgbImage::from_pixel(width, height, image::Rgb([24, 160, 220]));
    image::DynamicImage::ImageRgb8(buffer)
        .save_with_format(path, image::ImageFormat::Png)
        .expect("write png fixture");
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex run");
    let started = Instant::now();
    loop {
        if let Some(_status) = child.try_wait().expect("try wait") {
            break;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not exit within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut stdout = Vec::new();
    if let Some(mut handle) = child.stdout.take() {
        let _ = handle.read_to_end(&mut stdout);
    }
    let mut stderr = Vec::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_end(&mut stderr);
    }
    Output {
        status: child.wait().expect("wait"),
        stdout,
        stderr,
    }
}

#[test]
fn oversized_image_is_normalized_to_bounded_dimensions_before_upload() {
    let scratch = Scratch::new("oversized");
    let image = scratch.root.join("oversized.png");
    write_png(&image, 4000, 3000);

    let prompt = format!("Describe this image ![oversized]({})", image.display());
    let output = run_with_timeout(
        scratch.command(&["run", &prompt], false),
        Duration::from_secs(30),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("oversized.png: normalized 4000x3000 ->"),
        "stderr should report the oversized image being downscaled, got: {stderr}"
    );
    // The longest reported side must be bounded at 2000.
    assert!(
        stderr.contains("-> 2000x1500") || stderr.contains("2000x"),
        "normalized image should be bounded at the 2000px longest side, got: {stderr}"
    );
}

#[test]
fn in_bounds_image_passes_through_without_resize() {
    let scratch = Scratch::new("inbounds");
    let image = scratch.root.join("small.png");
    write_png(&image, 640, 480);

    let prompt = format!("Describe this image ![small]({})", image.display());
    let output = run_with_timeout(
        scratch.command(&["run", &prompt], false),
        Duration::from_secs(30),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("small.png: 640x480 within upload bounds"),
        "an in-bounds image should be reported as passing through unchanged, got: {stderr}"
    );
    assert!(
        !stderr.contains("small.png: normalized"),
        "an in-bounds image must not be resized: {stderr}"
    );
}

#[test]
fn live_multimodal_turn_succeeds_with_a_normalized_oversized_image() {
    let Ok(key) = std::env::var("OPENROUTER_API_KEY") else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    assert!(
        !key.is_empty(),
        "OPENROUTER_API_KEY must be non-empty when set"
    );

    let scratch = Scratch::new("live");
    let image = scratch.root.join("big.png");
    // An oversized image that must be normalized before the provider will accept it.
    write_png(&image, 3200, 2400);

    let prompt = format!(
        "Reply with the single word OK after looking at this image ![big]({})",
        image.display()
    );
    let output = run_with_timeout(
        scratch.command(
            &[
                "run",
                "--model",
                "google/gemini-2.5-flash",
                "--format",
                "json",
                &prompt,
            ],
            true,
        ),
        Duration::from_secs(90),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("big.png: normalized 3200x2400 ->"),
        "live run should report normalization before upload, got: {stderr}"
    );
    assert!(
        output.status.success(),
        "live multimodal turn should succeed for a normalized image; stderr: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|err| panic!("run JSON ({err}): {stdout}"));
    let content = value["content"].as_str().unwrap_or_default();
    assert!(
        !content.trim().is_empty(),
        "the model should return a real answer for the normalized image: {value}"
    );
}
