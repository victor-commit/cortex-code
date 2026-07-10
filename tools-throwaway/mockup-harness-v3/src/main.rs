//! Public inventory surface for the isolated v3 harness.

use std::process::ExitCode;

use mockup_harness_v3::{canonical_inventory, validate_canonical_map};

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.as_slice() != ["--list"] {
        eprintln!("usage: mockup-harness-v3 --list");
        return ExitCode::from(2);
    }

    if let Err(error) = validate_canonical_map() {
        eprintln!("inventory validation failed: {error}");
        return ExitCode::from(3);
    }

    for screen in canonical_inventory() {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            screen.category.as_str(),
            screen.id,
            screen.status.as_str(),
            screen.flow.as_str(),
            screen.surface.as_str(),
        );
    }

    ExitCode::SUCCESS
}
