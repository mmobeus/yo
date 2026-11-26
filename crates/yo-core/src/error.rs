//! yo-core's errors. The CLI renders them; the core never prints.

use std::fmt;
use std::io;
use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    /// The config file is wrong: bad JSON, an unknown key, a bad value.
    Config { path: PathBuf, msg: String },
    /// A setting (from the config or the environment) has a bad value, or
    /// the registry it describes doesn't hold together.
    Setting(String),
    /// A file couldn't be read or written.
    Io { path: PathBuf, err: io::Error },
    /// A command can't do what was asked (an unknown repo, a path that's
    /// taken, a bad name), found before anything changed.
    Refused(String),
    /// Something yo ran failed partway (git, gh, a launch).
    Failed(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, err: io::Error) -> Self {
        Error::Io {
            path: path.into(),
            err,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Config { path, msg } => write!(f, "{}: {msg}", path.display()),
            Error::Setting(msg) => f.write_str(msg),
            Error::Io { path, err } => write!(f, "{}: {err}", path.display()),
            Error::Refused(msg) | Error::Failed(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { err, .. } => Some(err),
            _ => None,
        }
    }
}
