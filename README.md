# slack-adapter

**Use your Slack programmatically — on your terms.** Drives the Slack web app in
a real browser *as you*, so what you can already do by hand becomes something you
can script. First capability: structured, date-ranged message search
(`from:@me`, `to:@me`, channel, free text) → rows of `ts, channel, author, text,
permalink`. **No bot token, app install, or admin approval.**

Built on [apiwright](https://github.com/stencilwright/stencilwright/tree/main/crates/apiwright)
(the runtime) and mapped with [stencilwright](https://github.com/stencilwright/stencilwright)
(the masked, LLM-collaborative site-mapper). Search is the first interaction
mapped — the same approach extends to anything the web app can do.

The crate ships a generic Slack map; you point it at your own workspace with
`SlackConfig` (subdomain + team id) — nothing to fork, no per-workspace build.

```rust
use chrono::NaiveDate;
use slack_adapter::{Slack, SlackConfig, SearchQuery};

// Point the published crate at *your* workspace — subdomain + team id. No fork.
let slack = Slack::open("acme", &SlackConfig::new("acme-team", "T0XXXXXXXX")).await?;
let q = SearchQuery::new().mine().mentions()
    .between(NaiveDate::from_ymd_opt(2026,5,25).unwrap(),
             NaiveDate::from_ymd_opt(2026,5,31).unwrap());
let rows = slack.search(&q).await?;   // ts, channel, author, text, permalink
```

Dev/test CLI (a harness for exercising the adapter during development — not meant to be installed):

```sh
slack-adapter-test --workspace acme-team --team-id T0XXXXXXXX \
    --from 2026-05-25 --to 2026-05-31 --mine --mentions
```

## Consent first

The browser is **headed by default**, or **off-screen but surfaceable** for
batch runs — never truly headless. Login, 2FA, captcha, or any unrecognized page
brings the window forward. You can always watch what's being done in your name.

## Status

**Working end-to-end** — login, date-ranged search, and structured results
(deduped across scopes, sorted) are implemented and validated against a live
workspace. Full library API and CLI reference:
[specs/01-slack-adapter.md](specs/01-slack-adapter.md).

## License

MIT
