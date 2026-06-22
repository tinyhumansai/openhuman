pub mod ingest;
mod post_process;
mod provider;
mod source;
mod sync;
#[cfg(test)]
mod tests;
pub mod tools;

pub use provider::GmailProvider;
pub use tools::GMAIL_CURATED;
