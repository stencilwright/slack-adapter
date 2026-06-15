# slack-adapter

**Turn Slack's web app into a local search API.** Logs in as *you* in a real
browser, runs date-ranged searches (`from:@me`, `to:@me`, channel, free text),
and extracts the results as structured rows — **no Slack API token, app install,
or admin approval required.**

Built on [apiwright](https://github.com/stencilwright/stencilwright/tree/main/crates/apiwright) (the runtime)
and mapped with [stencilwright](https://github.com/stencilwright/stencilwright)
(the masked, LLM-collaborative site-mapper).

```rust
use chrono::NaiveDate;
use slack_adapter::{Slack, SearchQuery};

let slack = Slack::open("acme").await?;
let q = SearchQuery::new().mine().mentions()
    .between(NaiveDate::from_ymd_opt(2026,5,25).unwrap(),
             NaiveDate::from_ymd_opt(2026,5,31).unwrap());
let rows = slack.search(&q).await?;   // ts, channel, author, text, permalink
```

CLI:

```sh
slack-search --site acme --from 2026-05-25 --to 2026-05-31 --mine --mentions
```

## Why browser-driven, not the official Slack API?

Slack's official `search.messages` method is deprecated; its replacement is
gated behind directory-published / internal apps, admin install, and (for
semantic search) a paid AI plan. On a *client's* workspace you're typically a
member, not an admin. Driving the web client as yourself needs none of that —
and a user's search only ever sees what that user can already see.

The search itself isn't scraped from the page: the adapter calls Slack's **own
browser-automation-backed API** (`search.modules.messages`) from the authenticated session —
the exact request the web client makes — which is what yields full message text,
every page, and correct `from:@me` / `to:@me`. Rationale and mechanics:
[specs/01-slack-adapter.md](specs/01-slack-adapter.md) (§1, §6.4).

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
