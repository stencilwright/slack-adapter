//! Dev/analysis tool: dump the **raw** (unmasked) page HTML of the live daemon.
//!
//! Attaches to the running `acme` daemon (or starts one), optionally
//! navigates to a URL, and prints the full raw DOM to stdout.
//!
//! ```text
//! cargo run --example dump_raw                # dump the current page
//! cargo run --example dump_raw -- <url>       # navigate first, then dump
//! ```
//!
//! Raw output — acme has no PII. Do not point this at a PII site.

use apiwright::{AdapterSession, RuntimeConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if apiwright::run_if_daemon().await? {
        return Ok(());
    }
    let session = AdapterSession::open(RuntimeConfig::new("acme")).await?;
    if let Some(url) = std::env::args().nth(1) {
        session.goto(&url).await?;
    }
    print!("{}", session.dump_raw().await?);
    Ok(())
}
