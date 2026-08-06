mod post_process;
mod provider;
#[cfg(test)]
mod tests;
pub mod tools;

pub use provider::GmailProvider;
pub use tools::GMAIL_CURATED;
