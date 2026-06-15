//! `slack-adapter-test` — CLI over [`slack_adapter`].
//!
//! ```text
//! slack-adapter-test --workspace acme-team --team-id T0XXXXXXXX \
//!     --from 2026-05-25 --to 2026-05-31 --mine --mentions
//! ```
//!
//! Prints the matching messages as JSON (default) or CSV. See
//! `specs/01-slack-adapter.md` for the contract.

use anyhow::Context;
use chrono::NaiveDate;
use clap::Parser;
use slack_adapter::{SearchQuery, Slack, SlackConfig};

#[derive(Parser, Debug)]
#[command(
    name = "slack-adapter-test",
    about = "Search your Slack via the web app and print billable-candidate rows"
)]
struct Args {
    /// Local profile name under ~/.stencilwright/<instance>/ — keeps multiple
    /// workspaces' sessions (and logins) separate.
    #[arg(long, default_value = "slack")]
    instance: String,

    /// Your workspace login subdomain — the `your-team` in your-team.slack.com.
    #[arg(long)]
    workspace: String,

    /// Your team id — the T0XXXXXXXX in app.slack.com/client/T0XXXXXXXX
    /// (visible in the address bar once you're signed in).
    #[arg(long)]
    team_id: String,

    /// Extra runtime value as NAME=REF (repeatable), e.g.
    /// slack_email=secret://1password/Private/Slack/username
    #[arg(long = "value", value_name = "NAME=REF")]
    values: Vec<String>,

    /// Inclusive start date (YYYY-MM-DD).
    #[arg(long)]
    from: NaiveDate,

    /// Inclusive end date (YYYY-MM-DD).
    #[arg(long)]
    to: NaiveDate,

    /// Include messages you sent (from:@me).
    #[arg(long)]
    mine: bool,

    /// Include messages mentioning / DM'd to you (to:@me).
    #[arg(long)]
    mentions: bool,

    /// Restrict to a channel.
    #[arg(long)]
    channel: Option<String>,

    /// Free-text terms.
    #[arg(long)]
    text: Option<String>,

    /// Run off-screen; surface only for login / captcha.
    #[arg(long)]
    offscreen: bool,

    /// Output format: json or csv.
    #[arg(long, default_value = "json")]
    format: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // If this process was re-exec'd as the browser daemon (`slack-adapter-test daemon
    // <dir>`), run it and exit — apiwright spawns it this way.
    if apiwright::run_if_daemon().await? {
        return Ok(());
    }

    let args = Args::parse();

    let mut q = SearchQuery::new().between(args.from, args.to);
    if args.mine {
        q = q.mine();
    }
    if args.mentions {
        q = q.mentions();
    }
    if let Some(c) = args.channel {
        q = q.in_channel(c);
    }
    if let Some(t) = args.text {
        q = q.text(t);
    }

    let mut config = SlackConfig::new(args.workspace.as_str(), args.team_id.as_str());
    for v in &args.values {
        let (name, reference) = v
            .split_once('=')
            .context("--value must be NAME=REF (e.g. slack_email=secret://…)")?;
        config = config.value(name, reference);
    }

    let slack = if args.offscreen {
        Slack::open_offscreen(&args.instance, &config).await?
    } else {
        Slack::open(&args.instance, &config).await?
    };

    let results = slack.search(&q).await?;

    match args.format.as_str() {
        "csv" => {
            println!("ts,channel,author,permalink,text");
            for r in &results {
                println!(
                    "{},{},{},{},{:?}",
                    r.ts, r.channel, r.author, r.permalink, r.text
                );
            }
        }
        _ => println!("{}", serde_json::to_string_pretty(&results)?),
    }
    Ok(())
}
