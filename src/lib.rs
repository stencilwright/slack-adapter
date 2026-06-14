//! # slack-adapter — a local search API over Slack's web app
//!
//! Drives the Slack web client (as *you*, in a real browser via
//! [`apiwright`]) to run date-ranged searches and extract the results as
//! structured rows — no Slack API token, app install, or admin approval needed.
//!
//! The motivating use case: a contractor reconciling billable work. "What did I
//! say, and what was said to me, in this client's Slack last week?" becomes:
//!
//! ```no_run
//! # use slack_adapter::*;
//! # async fn demo() -> anyhow::Result<()> {
//! use chrono::NaiveDate;
//! let slack = Slack::open("acme").await?;
//! let q = SearchQuery::new()
//!     .mine()        // messages you sent
//!     .mentions()    // + messages mentioning / DM'd to you
//!     .between(NaiveDate::from_ymd_opt(2026, 5, 25).unwrap(),
//!              NaiveDate::from_ymd_opt(2026, 5, 31).unwrap());
//! for r in slack.search(&q).await? {
//!     println!("{}  #{}  {}  {}", r.ts, r.channel, r.author, r.permalink);
//! }
//! # Ok(()) }
//! ```
//!
//! **Skeleton.** Method bodies are `todo!()`; the contract is specified in
//! `specs/01-slack-adapter.md`.
//!
//! [`apiwright`]: https://github.com/stencilwright/apiwright

use chrono::NaiveDate;

/// Who a result involves, relative to the authenticated user. Maps to Slack
/// search modifiers (`from:@me`, `to:@me`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Messages you sent (`from:@me`).
    FromMe,
    /// Messages mentioning you or DM'd to you (`to:@me`).
    ToMe,
}

impl Scope {
    /// The Slack search modifier for this scope.
    pub fn modifier(self) -> &'static str {
        match self {
            Scope::FromMe => "from:@me",
            Scope::ToMe => "to:@me",
        }
    }
}

/// A date-ranged Slack search.
///
/// Dates are **inclusive**; the adapter converts them to Slack's *exclusive*
/// `after:`/`before:` bounds internally (so `25..=31` becomes
/// `after:<24> before:<+1>`).
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: Option<String>,
    pub scopes: Vec<Scope>,
    pub channel: Option<String>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

impl SearchQuery {
    pub fn new() -> Self {
        Self::default()
    }

    /// Include messages you sent (`from:@me`).
    pub fn mine(mut self) -> Self {
        if !self.scopes.contains(&Scope::FromMe) {
            self.scopes.push(Scope::FromMe);
        }
        self
    }

    /// Include messages mentioning / DM'd to you (`to:@me`).
    pub fn mentions(mut self) -> Self {
        if !self.scopes.contains(&Scope::ToMe) {
            self.scopes.push(Scope::ToMe);
        }
        self
    }

    /// Inclusive date window.
    pub fn between(mut self, from: NaiveDate, to: NaiveDate) -> Self {
        self.from = Some(from);
        self.to = Some(to);
        self
    }

    /// Restrict to a channel (`in:#name`).
    pub fn in_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    /// Free-text terms.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Render to one Slack search string per scope (or a single string when no
    /// scope is set). Inclusive `from`/`to` become exclusive `after:`/`before:`.
    pub fn to_slack_queries(&self) -> Vec<String> {
        todo!("compose modifiers: scope, in:, after:/before: (exclusive), text")
    }
}

/// One extracted search result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    /// Message timestamp (ISO 8601, derived from the Slack `ts`).
    pub ts: String,
    pub channel: String,
    pub author: String,
    /// The message text (raw — this is billable content you need to read).
    pub text: String,
    /// Permalink to the message/thread, for click-through during review.
    pub permalink: String,
}

/// The Slack adapter handle.
pub struct Slack {
    _session: apiwright::AdapterSession,
}

impl Slack {
    /// Open the adapter for a mapped Slack workspace (`site` = the
    /// `stencilwright` map name, e.g. `"acme"`). Logs in via the mapped
    /// interactive places if the session isn't already authenticated; surfaces
    /// the window for SSO / 2FA / captcha.
    pub async fn open(_site: &str) -> anyhow::Result<Self> {
        todo!("RuntimeConfig::new(site) -> AdapterSession::open; ensure logged in")
    }

    /// Open off-screen (surfaces only for login / captcha / on request).
    pub async fn open_offscreen(_site: &str) -> anyhow::Result<Self> {
        todo!()
    }

    /// Run a search and extract all matching results — scroll-collected across
    /// the virtualized results pane, deduped, merged across scopes.
    pub async fn search(&self, _query: &SearchQuery) -> anyhow::Result<Vec<SearchResult>> {
        todo!("drive search box per to_slack_queries(); scroll-collect; parse; dedup")
    }
}
