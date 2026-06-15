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
//! Under the hood the adapter logs in via the embedded site map and runs the
//! search through Slack's own browser-automation-backed API (`search.modules.messages`),
//! called from the authenticated page — see [`search`] for the mechanics. The
//! full contract is specified in `specs/01-slack-adapter.md`.
//!
//! [`apiwright`]: https://github.com/stencilwright/apiwright

use chrono::NaiveDate;

use apiwright::{AdapterSession, RuntimeConfig};

mod map;
mod search;

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
        // Inclusive [from, to] → Slack's *exclusive* after:/before: bounds.
        // `after:` excludes the named day, so to include `from` we name the day
        // before; likewise `before:` excludes its day, so we name the day after
        // `to`. This off-by-one is the single most error-prone bit of the whole
        // adapter, which is why it lives here and is unit-tested.
        let mut dates = String::new();
        if let Some(from) = self.from {
            let after = from.pred_opt().unwrap_or(from);
            dates.push_str(&format!(" after:{}", after.format("%Y-%m-%d")));
        }
        if let Some(to) = self.to {
            let before = to.succ_opt().unwrap_or(to);
            dates.push_str(&format!(" before:{}", before.format("%Y-%m-%d")));
        }
        let channel = self
            .channel
            .as_ref()
            .map(|c| format!(" in:#{}", c.trim_start_matches('#')));
        let text = self.text.as_ref().map(|t| format!(" {t}"));

        // One query string per scope (unioned by the caller); modifier order:
        // scope, in:, after:, before:, free text.
        let render = |scope: Option<Scope>| -> String {
            let mut q = String::new();
            if let Some(s) = scope {
                q.push_str(s.modifier());
            }
            if let Some(c) = &channel {
                q.push_str(c);
            }
            q.push_str(&dates);
            if let Some(t) = &text {
                q.push_str(t);
            }
            q.trim().to_string()
        };

        if self.scopes.is_empty() {
            vec![render(None)]
        } else {
            self.scopes.iter().map(|s| render(Some(*s))).collect()
        }
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
    session: AdapterSession,
}

impl Slack {
    /// Open the adapter for the embedded workspace map (`site` = the map name,
    /// e.g. `"acme"`). Reuses the persistent Chrome profile, so most runs
    /// are already authenticated; otherwise the mapped login places drive what
    /// they can and surface the window for the magic-link code / SSO / captcha.
    pub async fn open(site: &str) -> anyhow::Result<Self> {
        Self::open_with(site, RuntimeConfig::new(site)).await
    }

    /// Open off-screen — surfaces only for login / captcha / on request.
    pub async fn open_offscreen(site: &str) -> anyhow::Result<Self> {
        Self::open_with(site, RuntimeConfig::new(site).offscreen()).await
    }

    async fn open_with(site: &str, cfg: RuntimeConfig) -> anyhow::Result<Self> {
        let graph = map::load(site)?;
        let session = AdapterSession::open_with_map(cfg, graph).await?;
        // Login is lazy: the first `search` navigates to `search_results`, which
        // (when the persistent profile isn't authenticated) Slack redirects to
        // the mapped login places — surfacing the window for the magic-link code.
        // Going straight to the search view also avoids the fragile `workspace`
        // recognition (Slack restores the last search view on navigation).
        Ok(Self { session })
    }

    /// Run a search and return matching messages — full text included, paged
    /// through Slack's search API, deduped by permalink and merged across
    /// scopes (sorted by timestamp ascending). Very broad queries are capped
    /// per scope (with a stderr notice); narrow by date/channel/text for more.
    pub async fn search(&self, query: &SearchQuery) -> anyhow::Result<Vec<SearchResult>> {
        search::run(&self.session, query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn spec_example_inclusive_to_exclusive() {
        // specs/01-slack-adapter.md §6.2: 2026-05-25..=2026-05-31 renders
        // after:2026-05-24 before:2026-06-01 (the days *outside* the window).
        let q = SearchQuery::new()
            .mine()
            .in_channel("proj-acme")
            .text("invoice")
            .between(d(2026, 5, 25), d(2026, 5, 31));
        assert_eq!(
            q.to_slack_queries(),
            vec!["from:@me in:#proj-acme after:2026-05-24 before:2026-06-01 invoice"]
        );
    }

    #[test]
    fn one_query_per_scope() {
        let q = SearchQuery::new()
            .mine()
            .mentions()
            .between(d(2026, 5, 25), d(2026, 5, 31));
        assert_eq!(
            q.to_slack_queries(),
            vec![
                "from:@me after:2026-05-24 before:2026-06-01",
                "to:@me after:2026-05-24 before:2026-06-01",
            ]
        );
    }

    #[test]
    fn no_scope_is_one_query_without_modifier() {
        let q = SearchQuery::new()
            .text("invoice")
            .between(d(2026, 5, 25), d(2026, 5, 31));
        assert_eq!(
            q.to_slack_queries(),
            vec!["after:2026-05-24 before:2026-06-01 invoice"]
        );
    }

    #[test]
    fn channel_hash_prefix_is_normalized() {
        let q = SearchQuery::new().mine().in_channel("#general");
        assert_eq!(q.to_slack_queries(), vec!["from:@me in:#general"]);
    }

    #[test]
    fn month_and_year_boundaries_off_by_one() {
        let q = SearchQuery::new().between(d(2026, 3, 1), d(2026, 12, 31));
        assert_eq!(
            q.to_slack_queries(),
            vec!["after:2026-02-28 before:2027-01-01"]
        );
    }

    #[test]
    fn embedded_acme_map_loads() {
        let g = crate::map::load("acme").expect("embedded map loads");
        for p in [
            "login_email",
            "login_captcha",
            "login_code",
            "workspace",
            "search_results",
        ] {
            assert!(g.place(p).is_some(), "missing place {p}");
        }
        let names: Vec<&str> = g
            .elements_at("search_results")
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        for e in [
            "result_row",
            "result_author",
            "result_channel",
            "result_permalink",
            "result_text",
        ] {
            assert!(names.contains(&e), "search_results missing element {e}");
        }
    }

    #[test]
    fn unknown_site_errors() {
        assert!(crate::map::load("nope").is_err());
    }
}
