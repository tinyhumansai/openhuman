//! Wire authentication styles shared by crate-native provider builders.

#[derive(Debug, Clone)]
pub enum AuthStyle {
    None,
    Bearer,
    XApiKey,
    Anthropic,
    Custom(String),
}
