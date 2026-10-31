//! How a server in a snippet relates to what a target already contains.

/// How a server in the snippet relates to what the target file already contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerChange {
    /// No server with this name exists yet.
    New,
    /// A server with this name exists and its definition would be replaced.
    Updated,
    /// A server with this name exists and already has exactly this definition.
    Unchanged,
}

impl ServerChange {
    /// Display label (Japanese) shared by the GUI badges and the headless output.
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "新規追加",
            Self::Updated => "上書き",
            Self::Unchanged => "変更なし",
        }
    }
}
