//! Small Rust/GPUI integration starting point. Uses only synthetic demo mail.
use lightmail_core::*;
use std::sync::Arc;

struct DemoPlatform;
impl PlatformServices for DemoPlatform {
    fn read_secret(&self, _key: String) -> Result<Option<String>> {
        Ok(None)
    }
    fn write_secret(&self, _key: String, _value: String) -> Result<()> {
        Err(fail("Demo does not store credentials"))
    }
    fn remove_secret(&self, _key: String) -> Result<()> {
        Ok(())
    }
    fn proxy_for(&self, _host: String) -> Result<ProxyRoute> {
        Ok(ProxyRoute {
            kind: "direct".into(),
            host: String::new(),
            port: 0,
        })
    }
}
struct Observer;
impl ApplicationObserver for Observer {
    fn changed(&self, event: ApplicationEvent) {
        println!("{:?} account={}", event.kind, event.account_id);
    }
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args()
        .nth(1)
        .ok_or("Pass an isolated demo data directory")?;
    let engine = MailEngine::new(directory)?;
    engine.seed_demo()?;
    let app = MailApplication::new(engine.clone(), Arc::new(DemoPlatform), Arc::new(Observer));
    let message = engine
        .list_messages(MessageQuery {
            account_id: String::new(),
            folder_id: String::new(),
            scope: "inbox".into(),
            search: String::new(),
            unread_only: false,
            limit: 1,
            offset: 0,
        })?
        .remove(0);
    // GPUI can await the same future in its executor; no Swift bridge is needed.
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let body = executor.block_on(app.clone().body(message.id.clone()))?;
    let output = export_markdown(message, body, None, ExportMode::Original, "Demo".into())?;
    assert!(output.starts_with('#'));
    println!("Read and exported {} bytes without a UI", output.len());
    app.stop();
    Ok(())
}
