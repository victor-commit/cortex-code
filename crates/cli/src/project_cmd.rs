//! The `cortex project` subcommands: an observable view over the multi-directory project model.
//!
//! A project groups one or more directories under one stable id (derived from the primary
//! directory's canonical [`checkpoints::workspace_key`], the same per-workspace key every other
//! per-project store uses). Because each directory belongs to exactly one project, a session opened
//! from any registered directory resolves to that one project rather than minting a fresh cwd-hash
//! identity. The database is opened (created and migrated on first use) at the path resolved by
//! [`db::DbPathInputs`]; diagnostics go to stderr and machine output (`--format json`) is a single
//! strict JSON document on stdout. Paths are redacted at the output boundary.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use db::{Database, DbError, DbPathInputs, Project};
use errors::Exit;
use serde::Serialize;

use crate::cli::{
    OutputFormat, ProjectActionArgs, ProjectArgs, ProjectCommand, ProjectDirectoryArgs,
};

/// Dispatch a `cortex project` invocation. With no subcommand it behaves like `current`.
pub fn run(args: ProjectArgs) -> Exit {
    let db = match open_database() {
        Ok(db) => db,
        Err(exit) => return exit,
    };

    match args.command {
        None => current(&db, OutputFormat::Text),
        Some(ProjectCommand::Current(args)) => current(&db, args.format.format),
        Some(ProjectCommand::List(args)) => list(&db, args),
        Some(ProjectCommand::Add(args)) => add(&db, args),
        Some(ProjectCommand::Remove(args)) => remove(&db, args),
        Some(ProjectCommand::Directories(args)) => directories(&db, args),
        Some(ProjectCommand::Sessions(args)) => sessions(&db, args),
    }
}

/// Resolve the database path and open it, mapping any persistence failure to a clear stderr
/// diagnostic and a non-zero exit (never a panic).
fn open_database() -> Result<Database, Exit> {
    let inputs = DbPathInputs::from_process();
    let Some(path) = inputs.resolve() else {
        eprintln!(
            "cortex project: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Err(Exit::Failure);
    };
    Database::open(&path).map_err(|err| db_error(&err))
}

fn current(db: &Database, format: OutputFormat) -> Exit {
    let cwd = match current_dir() {
        Ok(cwd) => cwd,
        Err(exit) => return exit,
    };
    match resolve_project(db, &cwd) {
        Ok(project) => emit_project(&project, format),
        Err(err) => db_error(&err),
    }
}

fn list(db: &Database, args: ProjectActionArgs) -> Exit {
    match db.projects().list() {
        Ok(projects) => match args.format.format {
            OutputFormat::Json | OutputFormat::Jsonl => {
                emit_json(&ProjectList::from_projects(projects))
            }
            OutputFormat::Text => emit_text(&render_project_list(&projects)),
        },
        Err(err) => db_error(&err),
    }
}

fn add(db: &Database, args: ProjectDirectoryArgs) -> Exit {
    let cwd = match current_dir() {
        Ok(cwd) => cwd,
        Err(exit) => return exit,
    };
    let project = match resolve_project(db, &cwd) {
        Ok(project) => project,
        Err(err) => return db_error(&err),
    };
    let directory = match canonicalize_existing(&cwd, &args.directory) {
        Ok(directory) => directory,
        Err(exit) => return exit,
    };
    match db.projects().add_directory(&project.id, &directory) {
        Ok(project) => emit_project(&project, args.format.format),
        Err(err) => db_error(&err),
    }
}

fn remove(db: &Database, args: ProjectDirectoryArgs) -> Exit {
    let cwd = match current_dir() {
        Ok(cwd) => cwd,
        Err(exit) => return exit,
    };
    let project = match resolve_project(db, &cwd) {
        Ok(project) => project,
        Err(err) => return db_error(&err),
    };
    // Removal targets a recorded directory; canonicalize against cwd but do not require existence
    // on disk, so a directory that was deleted can still be unregistered.
    let directory = resolve_path(&cwd, &args.directory);
    let directory = directory.to_string_lossy().into_owned();
    match db.projects().remove_directory(&project.id, &directory) {
        Ok(true) => {
            let project = match db.projects().require(&project.id) {
                Ok(project) => project,
                Err(err) => return db_error(&err),
            };
            emit_project(&project, args.format.format)
        }
        Ok(false) => {
            eprintln!(
                "cortex project: `{}` is not a removable directory of this project \
                 (the primary directory cannot be removed)",
                redaction::redact(&directory)
            );
            Exit::Failure
        }
        Err(err) => db_error(&err),
    }
}

fn directories(db: &Database, args: ProjectActionArgs) -> Exit {
    let cwd = match current_dir() {
        Ok(cwd) => cwd,
        Err(exit) => return exit,
    };
    let project = match resolve_project(db, &cwd) {
        Ok(project) => project,
        Err(err) => return db_error(&err),
    };
    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            emit_json(&DirectoryList::from_project(&project))
        }
        OutputFormat::Text => emit_text(&render_directories(&project)),
    }
}

fn sessions(db: &Database, args: ProjectActionArgs) -> Exit {
    let cwd = match current_dir() {
        Ok(cwd) => cwd,
        Err(exit) => return exit,
    };
    let project = match resolve_project(db, &cwd) {
        Ok(project) => project,
        Err(err) => return db_error(&err),
    };
    let ids = match db.sessions().ids_for_project(&project.id) {
        Ok(ids) => ids,
        Err(err) => return db_error(&err),
    };
    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&SessionList {
            id: project.id,
            sessions: ids,
        }),
        OutputFormat::Text => {
            let mut out = String::new();
            for id in &ids {
                out.push_str(id);
                out.push('\n');
            }
            emit_text(&out)
        }
    }
}

/// Resolve (creating on first use) the project for a working directory.
///
/// If the directory is already registered to a project it returns that project (so a directory
/// added as a second directory of project A still resolves to A); otherwise it creates a new
/// project anchored at this directory, keyed by its canonical workspace key.
pub fn resolve_project(db: &Database, cwd: &Path) -> Result<Project, DbError> {
    let canonical = cwd
        .canonicalize()
        .unwrap_or_else(|_| cwd.to_path_buf())
        .to_string_lossy()
        .into_owned();
    if let Some(project) = db.projects().find_by_directory(&canonical)? {
        return Ok(project);
    }
    let id = format!("prj-{}", checkpoints::workspace_key(cwd));
    let name = Path::new(&canonical)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    db.projects().ensure(&id, name.as_deref(), &canonical)
}

fn current_dir() -> Result<PathBuf, Exit> {
    std::env::current_dir().map_err(|err| {
        eprintln!("cortex project: cannot determine the working directory: {err}");
        Exit::Failure
    })
}

fn resolve_path(cwd: &Path, input: &str) -> PathBuf {
    let candidate = Path::new(input);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };
    joined.canonicalize().unwrap_or(joined)
}

fn canonicalize_existing(cwd: &Path, input: &str) -> Result<String, Exit> {
    let candidate = Path::new(input);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };
    match joined.canonicalize() {
        Ok(path) if path.is_dir() => Ok(path.to_string_lossy().into_owned()),
        Ok(path) => {
            eprintln!(
                "cortex project: `{}` is not a directory",
                redaction::redact(&path.to_string_lossy())
            );
            Err(Exit::Failure)
        }
        Err(err) => {
            eprintln!(
                "cortex project: cannot resolve directory `{}`: {err}",
                redaction::redact(input)
            );
            Err(Exit::Failure)
        }
    }
}

/// A project for JSON output, with paths redacted at the boundary.
#[derive(Serialize)]
struct ProjectView {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    directories: Vec<String>,
    created_at: String,
    updated_at: String,
}

impl ProjectView {
    fn from_project(project: &Project) -> Self {
        Self {
            id: project.id.clone(),
            name: project.name.as_deref().map(redaction::redact),
            directories: project
                .directories
                .iter()
                .map(|directory| redaction::redact(directory))
                .collect(),
            created_at: project.created_at.clone(),
            updated_at: project.updated_at.clone(),
        }
    }
}

#[derive(Serialize)]
struct ProjectList {
    projects: Vec<ProjectView>,
}

impl ProjectList {
    fn from_projects(projects: Vec<Project>) -> Self {
        Self {
            projects: projects.iter().map(ProjectView::from_project).collect(),
        }
    }
}

#[derive(Serialize)]
struct DirectoryList {
    id: String,
    directories: Vec<String>,
}

#[derive(Serialize)]
struct SessionList {
    id: String,
    sessions: Vec<String>,
}

impl DirectoryList {
    fn from_project(project: &Project) -> Self {
        Self {
            id: project.id.clone(),
            directories: project
                .directories
                .iter()
                .map(|directory| redaction::redact(directory))
                .collect(),
        }
    }
}

fn emit_project(project: &Project, format: OutputFormat) -> Exit {
    match format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&ProjectView::from_project(project)),
        OutputFormat::Text => emit_text(&render_project(project)),
    }
}

fn render_project(project: &Project) -> String {
    let mut out = format!("project {}\n", project.id);
    if let Some(name) = project.name.as_deref() {
        out.push_str(&format!("name: {}\n", redaction::redact(name)));
    }
    out.push_str(&format!("directories: {}\n", project.directories.len()));
    for directory in &project.directories {
        out.push_str(&format!("{}\n", redaction::redact(directory)));
    }
    out
}

fn render_directories(project: &Project) -> String {
    let mut out = String::new();
    for directory in &project.directories {
        out.push_str(&format!("{}\n", redaction::redact(directory)));
    }
    out
}

fn render_project_list(projects: &[Project]) -> String {
    let mut out = String::new();
    for project in projects {
        let name = project
            .name
            .as_deref()
            .map(redaction::redact)
            .unwrap_or_default();
        out.push_str(&format!("{}\t{}", project.id, name));
        for directory in &project.directories {
            out.push('\t');
            out.push_str(&redaction::redact(directory));
        }
        out.push('\n');
    }
    out
}

/// Serialize a value to a single-line strict JSON document terminated with a newline.
fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex project: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

/// Write a body to stdout, mapping a broken pipe to a generic failure exit.
fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

/// Render a [`DbError`] to stderr and map it to a non-zero exit.
fn db_error(err: &DbError) -> Exit {
    eprintln!("cortex project: {err}");
    Exit::Failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, name: Option<&str>, directories: &[&str]) -> Project {
        Project {
            id: id.to_owned(),
            name: name.map(str::to_owned),
            directories: directories.iter().map(|d| (*d).to_owned()).collect(),
            created_at: "2026-01-01 00:00:00".to_owned(),
            updated_at: "2026-01-01 00:00:00".to_owned(),
        }
    }

    #[test]
    fn project_list_text_lists_every_directory_on_the_project_line() {
        let rendered =
            render_project_list(&[project("prj-a", Some("alpha"), &["/work/a", "/work/b"])]);
        let line = rendered.lines().next().expect("one line");
        assert!(line.starts_with("prj-a\talpha\t"));
        assert!(line.contains("/work/a"));
        assert!(line.contains("/work/b"));
    }

    #[test]
    fn directories_text_is_one_path_per_line() {
        let rendered = render_directories(&project("prj-a", None, &["/work/a", "/work/b"]));
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines, vec!["/work/a", "/work/b"]);
    }

    #[test]
    fn project_text_reports_directory_count_and_paths() {
        let rendered = render_project(&project("prj-a", Some("alpha"), &["/work/a", "/work/b"]));
        assert!(rendered.contains("project prj-a"));
        assert!(rendered.contains("name: alpha"));
        assert!(rendered.contains("directories: 2"));
        assert!(rendered.contains("/work/a"));
        assert!(rendered.contains("/work/b"));
    }
}
