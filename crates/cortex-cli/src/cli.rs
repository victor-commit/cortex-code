use clap::{Args, Parser, Subcommand, ValueEnum};

/// Cortex Code — a terminal coding agent with interactive, headless, and server surfaces.
#[derive(Debug, Parser)]
#[command(
    name = "cortex",
    version,
    about = "Cortex Code — terminal coding agent (TUI, headless run, local server)",
    long_about = "Cortex Code is a single-binary terminal coding agent.\n\n\
        Run with no arguments to launch the interactive TUI. Use `cortex run` for\n\
        scriptable headless turns, or `cortex serve` to expose the local HTTP/SSE server.",
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a single headless turn (scriptable; reads the prompt from an argument or stdin).
    Run(RunArgs),

    /// Launch the interactive terminal UI (same as running with no arguments).
    Tui(TuiArgs),

    /// Start the local loopback HTTP/SSE server.
    Serve(ServeArgs),
}

/// Machine/human output format for headless runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable text (default).
    #[default]
    Text,
    /// A single strict JSON document on stdout.
    Json,
    /// Newline-delimited JSON events, one object per line.
    Jsonl,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The prompt to send. When omitted, the prompt is read from stdin.
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,

    /// Output format: text (default), json, or jsonl.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, value_name = "FORMAT")]
    pub format: OutputFormat,

    /// Shorthand for `--format jsonl` (newline-delimited JSON events).
    #[arg(long, conflicts_with = "format")]
    pub jsonl: bool,

    /// Model to use for this turn (overrides config and defaults).
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Agent persona to use for this turn.
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,

    /// Resume an explicit session by id.
    #[arg(long, value_name = "ID")]
    pub session: Option<String>,

    /// Resume the most recent session.
    #[arg(long, short = 'c', conflicts_with = "session")]
    pub r#continue: bool,
}

#[derive(Debug, Args)]
pub struct TuiArgs {}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address to bind the loopback server to.
    #[arg(long, default_value = "127.0.0.1", value_name = "HOST")]
    pub hostname: String,

    /// Port to bind the loopback server to.
    #[arg(long, default_value_t = 3200, value_name = "PORT")]
    pub port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::error::ErrorKind;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_args_yields_no_subcommand() {
        let cli = Cli::try_parse_from(["cortex"]).expect("parse");
        assert!(cli.command.is_none());
    }

    #[test]
    fn version_flag_is_recognized() {
        let err = Cli::try_parse_from(["cortex", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn short_version_flag_is_recognized() {
        let err = Cli::try_parse_from(["cortex", "-V"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn help_flag_is_recognized() {
        let err = Cli::try_parse_from(["cortex", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn short_help_flag_is_recognized() {
        let err = Cli::try_parse_from(["cortex", "-h"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_lists_core_subcommands() {
        let mut cmd = Cli::command();
        let help = cmd.render_help().to_string();
        for sub in ["run", "tui", "serve"] {
            assert!(help.contains(sub), "help should list `{sub}`:\n{help}");
        }
    }

    #[test]
    fn run_parses_prompt_and_flags() {
        let cli = Cli::try_parse_from([
            "cortex",
            "run",
            "hello world",
            "--model",
            "google/gemini-2.5-flash",
            "--session",
            "abc",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Run(args)) => {
                assert_eq!(args.prompt.as_deref(), Some("hello world"));
                assert_eq!(args.model.as_deref(), Some("google/gemini-2.5-flash"));
                assert_eq!(args.session.as_deref(), Some("abc"));
                assert_eq!(args.format, OutputFormat::Json);
                assert!(!args.jsonl);
                assert!(!args.r#continue);
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_jsonl_shorthand_is_parsed() {
        let cli = Cli::try_parse_from(["cortex", "run", "hi", "--jsonl"]).expect("parse");
        match cli.command {
            Some(Command::Run(args)) => {
                assert!(args.jsonl);
                assert_eq!(args.format, OutputFormat::Text);
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_continue_shorthand() {
        let cli = Cli::try_parse_from(["cortex", "run", "-c", "hi"]).expect("parse");
        match cli.command {
            Some(Command::Run(args)) => assert!(args.r#continue),
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_format_and_jsonl_conflict() {
        let err = Cli::try_parse_from(["cortex", "run", "hi", "--format", "json", "--jsonl"])
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn serve_defaults() {
        let cli = Cli::try_parse_from(["cortex", "serve"]).expect("parse");
        match cli.command {
            Some(Command::Serve(args)) => {
                assert_eq!(args.hostname, "127.0.0.1");
                assert_eq!(args.port, 3200);
            }
            other => panic!("expected serve, got {other:?}"),
        }
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        let err = Cli::try_parse_from(["cortex", "bogus"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn unknown_flag_is_rejected() {
        let err = Cli::try_parse_from(["cortex", "run", "--not-a-flag"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }
}
