mod application;
mod auth;
mod cache;
mod composition;
mod html;
mod markdown;
mod mime;
mod models;
mod platform;
mod presentation;
mod proxy;
mod store;
mod translation;
mod transport;
pub use application::*;
pub use auth::GoogleLogin;
pub use composition::*;
pub use models::*;
pub use platform::*;
pub use presentation::*;
pub use store::MailEngine;
pub use translation::*;
uniffi::setup_scaffolding!();
#[cfg(test)]
mod application_tests;
#[cfg(test)]
mod tests;
