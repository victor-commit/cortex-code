-- Per-session model selection: the model chosen for a session (e.g. via the TUI `/models`
-- picker) so a later headless `cortex run --session <id>` resume honors it without `--model`.
-- NULL means no per-session choice was made, so the configured/default model governs.
ALTER TABLE sessions ADD COLUMN model TEXT;
