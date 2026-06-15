//! Dev/analysis tool: evaluate JavaScript in the live daemon's page and print
//! the JSON result. For network/app-state inspection on the no-PII acme.
//!
//! ```text
//! cargo run --example probe -- '(() => ({ x: 1 + 1 }))()'
//! ```

use apiwright::{AdapterSession, RuntimeConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if apiwright::run_if_daemon().await? {
        return Ok(());
    }
    let js = std::env::args().nth(1).unwrap_or_else(|| "1 + 1".into());
    let session = AdapterSession::open(RuntimeConfig::new("acme")).await?;
    let v = session.evaluate(&js).await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}
