# Contributing to Cortex

Thanks for your interest in Cortex. This project is Apache-2.0 and welcomes
external contributions that improve the product, docs, or developer experience.

## Development setup

1. Install **Rust 1.85+** (edition 2024) via [rustup](https://rustup.rs/).
2. Clone the repository and build the CLI:

   ```bash
   cargo build -p cli --bins
   ./target/debug/cortex --version
   ```

3. Optional: set `OPENROUTER_API_KEY` (or use `cortex auth login`) for live
   model turns. Never commit keys; `.env` / `.env.local` are gitignored.

## Checks before opening a PR

Run the subset that matches your change, then a broader gate if you touch
shared crates:

```bash
# Always: the package you edited
cargo check -p <crate>
cargo test -p <crate>

# CLI binary still builds
cargo check -p cli --bins

# Broader (when touching workspace-wide code)
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=8
```

Prefer targeted `cargo test -p …` during iteration; full workspace runs are
slower but required for cross-cutting changes.

## Pull request expectations

- **One concern per PR** when practical (feature, fix, or docs — not all three).
- **Clear description**: what changed, why, and how you verified it.
- **Tests**: add or update tests for behavior changes. Live/network tests must
  skip cleanly without credentials.
- **No secrets**: never commit API keys, tokens, or private credentials.
- **Style**: follow existing Rust idioms in the crate you touch; run
  `cargo fmt` if rustfmt is available in your environment.
- **Docs**: update `README.md` or `docs/` when user-facing behavior or commands
  change.

## Commit messages

Use conventional subjects:

```text
type(scope): summary
```

Examples: `fix(tui): keep footer visible during stream`,  
`feat(providers): add catalogue fallback for offline models`.

## Code of conduct

Be respectful and constructive. Harassment or bad-faith contributions will not
be accepted.

## License

By contributing, you agree that your contributions are licensed under the
project’s Apache-2.0 license (and any file-level licenses already present, such
as MIT headers on ported TUI framework files — see `NOTICE`).
