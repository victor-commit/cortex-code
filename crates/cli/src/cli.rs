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
    /// Increase logging verbosity (repeatable: `-v` for debug, `-vv` for trace).
    ///
    /// Routes debug/event logs (including event-bus delivery) to stderr; `RUST_LOG` is also
    /// honored. Never affects stdout, so machine output stays clean.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a single headless turn (scriptable; reads the prompt from an argument or stdin).
    ///
    /// Input-source precedence is deterministic: a positional prompt argument always wins,
    /// and stdin is read only when no argument is given. The two are never blended.
    Run(RunArgs),

    /// Launch the interactive terminal UI (same as running with no arguments).
    Tui(TuiArgs),

    /// Start the local loopback HTTP/SSE server.
    Serve(ServeArgs),

    /// Report the effective configuration after merging every layer.
    ///
    /// Layers are merged lowest-precedence first:
    /// defaults < global < project < nested < env < CLI. Environment overrides use the
    /// `CORTEX_<KEY>` mapping (e.g. `CORTEX_MODEL`); `--model` is the highest-precedence
    /// CLI override. Use `--format json` for a machine-parseable dump of the merged config.
    Config(ConfigArgs),

    /// Create, list, and inspect persisted sessions and their messages.
    ///
    /// Sessions and messages live in a SQLite database created on first use under the data
    /// directory (`${XDG_DATA_HOME:-$HOME/.local/share}/cortex/cortex.db` by default). Set
    /// `CORTEX_DB_PATH` to point at an explicit database file, or `CORTEX_DATA_DIR` to relocate
    /// the data directory; the default location is left untouched when an override is used.
    Session(SessionArgs),
}

/// Output format shared by the `session` subcommands.
#[derive(Debug, Args)]
pub struct FormatArgs {
    /// Output format: text (default) or json (a single strict JSON document).
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, value_name = "FORMAT")]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommand,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Create a new session and print its id.
    New(SessionNewArgs),

    /// List existing sessions, newest first.
    List(SessionListArgs),

    /// Show a session and the messages stored under it.
    Show(SessionShowArgs),

    /// Append a message to a session (content from an argument or stdin).
    Message(SessionMessageArgs),
}

#[derive(Debug, Args)]
pub struct SessionNewArgs {
    /// Optional human-facing title for the session.
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SessionListArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SessionShowArgs {
    /// The id of the session to show.
    #[arg(value_name = "ID")]
    pub id: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SessionMessageArgs {
    /// The id of the session to append to.
    #[arg(value_name = "ID")]
    pub id: String,

    /// The message content. Takes precedence over stdin; when omitted, stdin is read.
    #[arg(value_name = "CONTENT")]
    pub content: Option<String>,

    /// The role to record for the message.
    #[arg(long, default_value = "user", value_name = "ROLE")]
    pub role: String,

    #[command(flatten)]
    pub format: FormatArgs,
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
    /// The prompt to send. Takes precedence over stdin: when given, stdin is not read.
    /// When omitted, the prompt is read from stdin instead.
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
pub struct ConfigArgs {
    /// Output format: text (default, a TOML dump) or json (a single strict JSON document).
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, value_name = "FORMAT")]
    pub format: OutputFormat,

    /// Override the effective model (highest precedence; beats env and every config file).
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,
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
        for sub in ["run", "tui", "serve", "session"] {
            assert!(help.contains(sub), "help should list `{sub}`:\n{help}");
        }
    }

    #[test]
    fn session_new_parses_title_and_format() {
        let cli = Cli::try_parse_from([
            "cortex", "session", "new", "--title", "demo", "--format", "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Session(SessionArgs {
                command: SessionCommand::New(args),
            })) => {
                assert_eq!(args.title.as_deref(), Some("demo"));
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected session new, got {other:?}"),
        }
    }

    #[test]
    fn session_message_parses_id_content_and_role() {
        let cli = Cli::try_parse_from([
            "cortex",
            "session",
            "message",
            "ses_1",
            "hello there",
            "--role",
            "assistant",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Session(SessionArgs {
                command: SessionCommand::Message(args),
            })) => {
                assert_eq!(args.id, "ses_1");
                assert_eq!(args.content.as_deref(), Some("hello there"));
                assert_eq!(args.role, "assistant");
            }
            other => panic!("expected session message, got {other:?}"),
        }
    }

    #[test]
    fn session_requires_a_subcommand() {
        let err = Cli::try_parse_from(["cortex", "session"]).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
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
