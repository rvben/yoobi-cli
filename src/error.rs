//! Stable error kinds and process status codes.
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("{message}")]
    Usage { message: String },
    #[error("{message}")]
    TtyRequired { message: String },
    #[error("{message}")]
    InvalidInput { message: String },
    #[error("{message}")]
    Auth { message: String },
    #[error("{message}")]
    Network { message: String },
    #[error("{message}")]
    Protocol { message: String },
    #[error("{message}")]
    Conflict { message: String },
    #[error("{message}")]
    Io { message: String },
}

impl Error {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "usage",
            Self::TtyRequired { .. } => "tty_required",
            Self::InvalidInput { .. } => "invalid_input",
            Self::Auth { .. } => "auth",
            Self::Network { .. } => "network",
            Self::Protocol { .. } => "protocol",
            Self::Conflict { .. } => "conflict",
            Self::Io { .. } => "io",
        }
    }
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::Usage { .. } | Self::InvalidInput { .. } => Some("see `yoobi-cli --help`"),
            Self::Auth { .. } => Some(
                "set YOOBI_BASE_URL, YOOBI_USERNAME, and YOOBI_PASSWORD or YOOBI_PASSWORD_FILE",
            ),
            Self::Conflict { .. } => {
                Some("read the timesheet and generate a new plan; do not blindly retry a write")
            }
            Self::Network { .. } | Self::Protocol { .. } => {
                Some("after an interrupted apply, read the timesheet before retrying")
            }
            Self::TtyRequired { .. } => {
                Some("run in a terminal, or use `tui --demo --snapshot` for an offline preview")
            }
            Self::Io { .. } => None,
        }
    }
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage { .. } => 3,
            Self::TtyRequired { .. } => 6,
            Self::Auth { .. } => 4,
            Self::Conflict { .. } => 5,
            _ => 1,
        }
    }
}
