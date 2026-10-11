//! A Loopwerk session: an open project, where it came from, and its undo history. Every change
//! goes through a command (see `docs/decisions/0002-commands.md`), so the CLI, the MCP server and
//! later the UI all drive the project the same way.

mod commands;

use std::fmt;
use std::path::{Path, PathBuf};

use loopwerk_model::Project;
use serde_json::Value;

pub use commands::{CommandKind, CommandSpec, Param, ParamType, commands};

/// Undo steps kept per session; the oldest are dropped first.
const MAX_UNDO: usize = 256;

/// Why a command did not run. The session is unchanged when this is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError(pub String);

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CommandError {}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self(message.to_string())
    }
}

pub struct Session {
    project: Project,
    source: Option<PathBuf>,
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl Session {
    pub fn new(project: Project) -> Self {
        Self { project, source: None, undo: Vec::new(), redo: Vec::new() }
    }

    /// Opens a Live Set. Returns the session and the importer's warnings.
    pub fn open(path: &Path) -> Result<(Self, Vec<String>), CommandError> {
        let import = loopwerk_als::read_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut session = Self::new(import.project);
        session.source = Some(path.to_path_buf());
        Ok((session, import.warnings))
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Runs the command `id` with JSON `params` and returns its JSON result. An editing command
    /// records one undo step when it changes the project; if it fails, or leaves a project that
    /// does not validate, the project is restored and an error is returned.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        let spec = commands().iter().find(|c| c.id == id).ok_or_else(|| unknown_command(id))?;
        if !(params.is_object() || params.is_null()) {
            return Err(format!("{id}: parameters must be a JSON object").into());
        }
        if let Some(object) = params.as_object()
            && let Some(unknown) = object.keys().find(|key| !spec.params.iter().any(|p| p.name == key.as_str()))
        {
            let known: Vec<&str> = spec.params.iter().map(|p| p.name).collect();
            return Err(format!("{id}: unknown parameter {unknown:?}; expected {known:?}").into());
        }
        match spec.kind {
            CommandKind::Read | CommandKind::History => (spec.run)(self, params),
            CommandKind::Edit => {
                let before = self.project.clone();
                let result = (spec.run)(self, params).and_then(|value| match self.project.validate() {
                    Ok(()) => Ok(value),
                    Err(errors) => Err(CommandError(format!("{id} would leave an invalid project: {errors:?}"))),
                });
                match result {
                    Ok(value) => {
                        if self.project != before {
                            self.push_undo(before);
                        }
                        Ok(value)
                    }
                    Err(e) => {
                        self.project = before;
                        Err(e)
                    }
                }
            }
        }
    }

    fn push_undo(&mut self, before: Project) {
        if self.undo.len() >= MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
    }

    fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.project, previous));
        true
    }

    fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.project, next));
        true
    }
}

fn unknown_command(id: &str) -> CommandError {
    let area = id.split('.').next().unwrap_or_default();
    let similar: Vec<&str> = commands().iter().map(|c| c.id).filter(|c| c.starts_with(area)).collect();
    if similar.is_empty() {
        CommandError(format!("unknown command {id}; commands.list shows them all"))
    } else {
        CommandError(format!("unknown command {id}; did you mean one of: {}", similar.join(", ")))
    }
}
