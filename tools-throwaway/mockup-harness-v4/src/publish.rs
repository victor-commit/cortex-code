//! Atomic-ish publish of pure Cortex pilot artifacts into existing mockups-v3 roots.
//!
//! Staging writes beside the destination, validates glyphs + dimensions, then renames.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::chrome::canvas::{self, Canvas};
use crate::chrome::tokens::{validate_glyphs, CANVAS_COLUMNS, CANVAS_ROWS};
use crate::screens::{list_pilots, render_pilot, PilotSpec};

pub const CANONICAL_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3";
pub const GATE_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3-gates";
const MANIFEST_NAME: &str = ".renderer-manifest-v1-pure-cortex";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishResult {
    pub id: String,
    pub category: String,
    pub txt_path: PathBuf,
    pub png_path: Option<PathBuf>,
    pub txt_sha256: String,
    pub png_sha256: Option<String>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    // Minimal SHA-256 (FIPS-style) without extra crates — pin for determinism audits.
    // Prefer system `sha256sum` parity via our own permanent pure-Rust digest.
    sha256_pure(bytes)
}

fn sha256_pure(message: &[u8]) -> String {
    // Compact public-domain style SHA-256 for harness digests only.
    fn rotr(x: u32, n: u32) -> u32 {
        x.rotate_right(n)
    }
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (message.len() as u64) * 8;
    let mut data = message.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
            let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

fn assert_gallery_text(text: &str) -> Result<(), String> {
    validate_glyphs(text)?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != CANVAS_ROWS as usize {
        return Err(format!(
            "expected {} text rows, got {}",
            CANVAS_ROWS,
            lines.len()
        ));
    }
    for (i, line) in lines.iter().enumerate() {
        if line.chars().count() != CANVAS_COLUMNS as usize {
            return Err(format!(
                "row {i} width {} != {}",
                line.chars().count(),
                CANVAS_COLUMNS
            ));
        }
    }
    // Pure dump: no copper brand residual.
    crate::chrome::components::reject_copper_residual(text)?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    }
    let staging = path.with_extension(format!(
        "{}.pure-cortex-staging",
        path.extension().and_then(|s| s.to_str()).unwrap_or("tmp")
    ));
    {
        let mut file = File::create(&staging).map_err(|e| format!("create {staging:?}: {e}"))?;
        file.write_all(bytes)
            .map_err(|e| format!("write {staging:?}: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("sync {staging:?}: {e}"))?;
    }
    fs::rename(&staging, path).map_err(|e| format!("rename {staging:?} -> {path:?}: {e}"))?;
    Ok(())
}

pub fn publish_pilot(spec: &PilotSpec) -> Result<PublishResult, String> {
    let canvas = render_pilot(spec.id)?;
    publish_canvas(spec, &canvas)
}

fn publish_canvas(spec: &PilotSpec, canvas: &Canvas) -> Result<PublishResult, String> {
    let text = canvas.to_text();
    assert_gallery_text(&text)?;
    let category_dir = Path::new(CANONICAL_ROOT).join(spec.category);
    let txt_path = category_dir.join(format!("{}.txt", spec.id));
    write_atomic(&txt_path, text.as_bytes())?;

    #[cfg(feature = "png")]
    let (png_path, png_sha) = {
        let png_bytes = canvas::render_png(canvas)?;
        if png_bytes.len() < 32 {
            return Err("PNG encoder returned empty/tiny buffer".to_owned());
        }
        // PNG magic
        if &png_bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
            return Err("PNG magic missing".to_owned());
        }
        let dest = category_dir.join(format!("{}.png", spec.id));
        write_atomic(&dest, &png_bytes)?;
        (Some(dest), Some(sha256_hex(&png_bytes)))
    };
    #[cfg(not(feature = "png"))]
    let (png_path, png_sha): (Option<PathBuf>, Option<String>) = (None, None);

    Ok(PublishResult {
        id: spec.id.to_owned(),
        category: spec.category.to_owned(),
        txt_path,
        png_path,
        txt_sha256: sha256_hex(text.as_bytes()),
        png_sha256: png_sha,
    })
}

pub fn publish_pilots() -> Result<Vec<PublishResult>, String> {
    let mut results = Vec::new();
    for spec in list_pilots() {
        results.push(publish_pilot(spec)?);
    }
    write_manifest(&results)?;
    Ok(results)
}

fn write_manifest(results: &[PublishResult]) -> Result<(), String> {
    let mut body = String::from("mockup-harness-v4-pure-cortex-manifest-v1\n");
    body.push_str("generator=mockup-harness-v4\n");
    body.push_str("identity=pure-cortex-no-copper\n");
    body.push_str("theme=CortexNight\n");
    for result in results {
        body.push_str(&format!(
            "artifact\t{}/{}\ttxt={}\tpng={}\n",
            result.category,
            result.id,
            result.txt_sha256,
            result.png_sha256.as_deref().unwrap_or("-")
        ));
    }
    let path = Path::new(CANONICAL_ROOT).join(MANIFEST_NAME);
    write_atomic(&path, body.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        // SHA256("") =
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // SHA256("abc")
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
