use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{reason}")]
    InvalidProfile { reason: String },
    #[error("Share link is not valid or is too large")]
    InvalidLink,
    #[error("{reason}")]
    InvalidRules { reason: String },
    #[error("{reason}")]
    Engine { reason: String },
}

impl AppError {
    pub fn engine(error: impl std::fmt::Display) -> Self {
        Self::Engine {
            reason: error.to_string(),
        }
    }

    pub(crate) fn rules(error: impl std::fmt::Display) -> Self {
        Self::InvalidRules {
            reason: error.to_string(),
        }
    }
}
