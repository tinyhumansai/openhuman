use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryFile {
    /// Agent's notes about the environment, conventions, what worked.
    Memory,
    /// Notes about the user — preferences, role, recurring goals.
    User,
}

impl MemoryFile {
    pub fn filename(&self) -> &'static str {
        match self {
            Self::Memory => "MEMORY.md",
            Self::User => "USER.md",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub memory: String,
    pub user: String,
}
