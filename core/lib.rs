mod cache;
mod html;
mod markdown;
mod mime;
mod models;
mod proxy;
mod store;
mod transport;
pub use models::*;
pub use store::MailEngine;
uniffi::setup_scaffolding!();
#[cfg(test)]
mod tests;
