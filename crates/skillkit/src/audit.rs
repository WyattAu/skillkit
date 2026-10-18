//! Engagement-integrity audit for a GitHub repository.
//!
//! Pulls the repo's public stats and runs `signalkit-integrity`'s
//! fork-ratio verdict. This is evidence-first: the report always shows
//! the numbers, and the verdict is a heuristic, not an accusation.

use serde::Deserialize;

/// Stats pulled from the GitHub API.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoStats {
    /// Full name.
    pub full_name: String,
    /// Stars.
    pub stargazers_count: i64,
    /// Forks.
    pub forks_count: i64,
    /// Open issues+PRs (payload fidelity; not part of v0 verdicts).
    #[allow(dead_code)]
    pub open_issues_count: i64,
    /// Created at (RFC 3339).
    pub created_at: String,
    /// Days since creation, derived at render time from `created_at`
    /// (parsed leniently; 0 when unparseable).
    #[serde(default)]
    pub age_days: i64,
}

/// The finished audit.
#[derive(Debug, Clone)]
pub struct AuditReport {
    /// Repo audited.
    pub repo: String,
    /// Underlying stats.
    pub stats: RepoStats,
    /// Fork-ratio verdict from `signalkit-integrity`.
    pub verdict: signalkit_integrity::ForkVerdict,
    /// The measured ratio, in percent.
    pub fork_ratio_pct: f64,
}

/// Fetch current stats for `owner/repo`; `token` upgrades the 60 req/h
/// anonymous limit to 5,000 req/h.
///
/// # Errors
/// Fails on transport errors and non-2xx responses.
pub async fn fetch_repo_stats(repo: &str, token: Option<&str>) -> anyhow::Result<RepoStats> {
    let client = fetch_kit::Client::builder()
        .user_agent("skillkit/0.1")
        .default_header("accept", "application/vnd.github+json")
        .build();
    let url = format!("https://api.github.com/repos/{repo}");
    let mut request = client.get(&url);
    if let Some(t) = token {
        request = request.bearer_auth(t);
    }
    let mut stats: RepoStats = request.json_response().await?;
    stats.age_days = age_in_days(&stats.created_at, current_epoch_days());
    Ok(stats)
}

/// Days since `created_at` (RFC 3339 date prefix), using the current
/// UTC date. Coarse civil-date math; ±1 day slop is irrelevant for
/// verdicts.
fn age_in_days(created_at: &str, now_epoch_days: i64) -> i64 {
    let Some(day) = created_at.get(0..10) else {
        return 0;
    };
    // YYYY-MM-DD → days via a coarse civil-date computation; a leap-year
    // slop of ±1 day is irrelevant for verdicts.
    let mut parts = [1970i64, 1, 1];
    for (i, chunk) in day.split('-').take(3).enumerate() {
        if let Ok(v) = chunk.parse::<i64>() {
            parts[i] = v;
        }
    }
    now_epoch_days - days_from_epoch(parts[0], parts[1], parts[2])
}

/// Days from 1970-01-01 to `y-m-d` (proleptic Gregorian).
fn days_from_epoch(y: i64, m: i64, d: i64) -> i64 {
    let days_before_month = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let y_days = (y - 1970) * 365 + (y - 1969) / 4 - (y - 1901) / 100 + (y - 1921) / 400;
    y_days + days_before_month[(m - 1).clamp(0, 11) as usize] + d - 1
}

fn current_epoch_days() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    secs / 86_400
}

/// Audit `repo`: fetch stats + fork-ratio verdict.
///
/// # Errors
/// Fails on fetch errors; verdicts never fail.
pub async fn audit_repo(
    repo: &str,
    min_stars: i64,
    token: Option<&str>,
) -> anyhow::Result<AuditReport> {
    let stats = fetch_repo_stats(repo, token).await?;
    let fork_ratio_pct = if stats.stargazers_count > 0 {
        stats.forks_count as f64 * 100.0 / stats.stargazers_count as f64
    } else {
        0.0
    };
    let verdict =
        signalkit_integrity::fork_verdict(stats.stargazers_count, stats.forks_count, min_stars);
    Ok(AuditReport {
        repo: stats.full_name.clone(),
        stats,
        verdict,
        fork_ratio_pct,
    })
}

impl AuditReport {
    /// Render the evidence-first report.
    #[must_use]
    pub fn render(&self) -> String {
        let verdict_line = match self.verdict {
            signalkit_integrity::ForkVerdict::Normal => "normal — nothing unusual",
            signalkit_integrity::ForkVerdict::Elevated => {
                "ELEVATED — unusual fork ratio, worth a manual look"
            }
            signalkit_integrity::ForkVerdict::Extreme => {
                "EXTREME — fork ratio matches farmed-engagement shapes"
            }
        };
        format!(
            "audit: {}\n  stars: {}  forks: {}  (ratio {:.1}%)\n  age: {} days\n  fork-ratio verdict (min {} stars): {}\n  evidence is heuristic; check commit history and contributor spread before drawing conclusions",
            self.repo,
            self.stats.stargazers_count,
            self.stats.forks_count,
            self.fork_ratio_pct,
            self.stats.age_days,
            200,
            verdict_line
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn age_parses_rfc3339_prefix() {
        let now = days_from_epoch(2026, 10, 18);
        assert_eq!(age_in_days("2026-10-07T08:34:24Z", now), 11);
        assert_eq!(age_in_days("garbage", now), 0);
        assert_eq!(days_from_epoch(1970, 1, 1), 0);
        assert_eq!(days_from_epoch(2026, 10, 18), now);
    }

    #[test]
    fn render_shows_evidence() {
        let report = AuditReport {
            repo: "farm/thing".to_owned(),
            stats: RepoStats {
                full_name: "farm/thing".to_owned(),
                stargazers_count: 5_000,
                forks_count: 2_500,
                open_issues_count: 3,
                created_at: "2026-10-07T00:00:00Z".to_owned(),
                age_days: 11,
            },
            verdict: signalkit_integrity::ForkVerdict::Extreme,
            fork_ratio_pct: 50.0,
        };
        let text = report.render();
        assert!(text.contains("5000"));
        assert!(text.contains("50.0%"));
        assert!(text.contains("EXTREME"));
        assert!(text.contains("heuristic"));
    }
}
