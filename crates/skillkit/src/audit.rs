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

/// Extract GitHub `owner/repo` candidates from a marketplace.json body.
///
/// Handles both string sources (`"owner/repo"`) and object sources with a
/// `repo` field (`{"source": "git-subdir", "repo": "owner/repo"}`).
/// Real shapes (anthropics/claude-plugins-official): string "owner/repo",
/// `{source: "git-subdir", url: "...github.com/o/r.git", path: ...}`,
/// `{source: "url", url: ...}` — relative-dir sources are skipped.
/// Returns `(repos, total plugin entries)`.
#[must_use]
pub fn parse_marketplace_repos(json: &str) -> (Vec<String>, usize) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return (Vec::new(), 0);
    };
    let Some(plugins) = value.get("plugins").and_then(|p| p.as_array()) else {
        return (Vec::new(), 0);
    };
    let total = plugins.len();
    let mut out: Vec<String> = Vec::new();
    for plugin in plugins {
        let candidate = match plugin.get("source") {
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            Some(source) => source
                .get("repo")
                .and_then(|r| r.as_str())
                .map(str::to_owned)
                .or_else(|| {
                    source
                        .get("url")
                        .and_then(|u| u.as_str())
                        .and_then(github_repo_from_url)
                }),
            None => None,
        };
        if let Some(repo) = candidate {
            let parts: Vec<&str> = repo.split('/').collect();
            if parts.len() == 2
                && parts.iter().all(|p| !p.is_empty())
                && !repo.starts_with('.')
                && !repo.starts_with('/')
                && repo
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._/".contains(c))
                && !out.contains(&repo)
            {
                out.push(repo);
            }
        }
    }
    (out, total)
}

/// `https://github.com/{owner}/{repo}(.git)?` -> `owner/repo`.
fn github_repo_from_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Fetch a marketplace index from a GitHub repo, trying the standard
/// locations (contents API returns base64 for JSON files).
///
/// # Errors
/// Fails when both known locations return errors.
pub async fn fetch_marketplace_json(repo: &str, token: Option<&str>) -> anyhow::Result<String> {
    let client = fetch_kit::Client::builder()
        .user_agent("skillkit/0.1")
        .default_header("accept", "application/vnd.github+json")
        .build();
    for path in [".claude-plugin/marketplace.json", "marketplace.json"] {
        let url = format!("https://api.github.com/repos/{repo}/contents/{path}");
        let mut request = client.get(&url);
        if let Some(t) = token {
            request = request.bearer_auth(t);
        }
        let response: serde_json::Value = match request.json_response().await {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(content) = response.get("content").and_then(|c| c.as_str()) {
            let cleaned: String = content.chars().filter(|c| !c.is_whitespace()).collect();
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(cleaned)
                .map_err(|e| anyhow::anyhow!("marketplace base64 decode failed: {e}"))?;
            return Ok(String::from_utf8_lossy(&decoded).into_owned());
        }
    }
    anyhow::bail!("no marketplace.json found in {repo}")
}

/// The finished batch audit.
pub struct BatchReport {
    /// Total plugin entries in the index (some are relative-dir, not repos).
    pub total_entries: usize,
    /// Audited repos, strongest verdicts first.
    pub audited: Vec<AuditReport>,
    /// Repos that failed to fetch: (repo, error).
    pub failed: Vec<(String, String)>,
}

impl BatchReport {
    /// Render the batch report.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let elevated = self
            .audited
            .iter()
            .filter(|r| r.verdict == signalkit_integrity::ForkVerdict::Elevated)
            .count();
        let extreme = self
            .audited
            .iter()
            .filter(|r| r.verdict == signalkit_integrity::ForkVerdict::Extreme)
            .count();
        out.push_str(&format!(
            "batch audit: {} of {} entries audited, {} elevated, {} extreme, {} fetch-failed\n",
            self.audited.len(),
            self.total_entries,
            elevated,
            extreme,
            self.failed.len()
        ));
        out.push_str(
            "Verdicts are heuristic fork-ratio signals (min-stars floored); evidence below.\n",
        );
        let mut rows: Vec<&AuditReport> = self
            .audited
            .iter()
            // Display floor: sub-min_stars repos have meaningless ratios
            // and would bury the real signals under 400%-of-1-star noise.
            .filter(|r| r.stats.stargazers_count >= 200)
            .collect();
        let verdict_rank = |v: signalkit_integrity::ForkVerdict| match v {
            signalkit_integrity::ForkVerdict::Extreme => 0,
            signalkit_integrity::ForkVerdict::Elevated => 1,
            signalkit_integrity::ForkVerdict::Normal => 2,
        };
        rows.sort_by_key(|r| {
            (
                verdict_rank(r.verdict),
                std::cmp::Reverse((r.fork_ratio_pct * 100.0) as i64),
            )
        });
        for r in rows {
            let tag = match r.verdict {
                signalkit_integrity::ForkVerdict::Extreme => "EXTREME",
                signalkit_integrity::ForkVerdict::Elevated => "elevated",
                signalkit_integrity::ForkVerdict::Normal => "normal",
            };
            out.push_str(&format!(
                "  {:<8} {:>7}\u{2605} {:>6} forks ({:>5.1}%)  {}\n",
                tag, r.stats.stargazers_count, r.stats.forks_count, r.fork_ratio_pct, r.repo
            ));
        }
        for (repo, err) in &self.failed {
            out.push_str(&format!("  failed    {repo}: {err}\n"));
        }
        out
    }
}

/// Audit every GitHub-repo plugin in a marketplace body.
///
/// # Errors
/// Fails when `body` has no parseable plugins array.
pub async fn audit_batch(
    body: &str,
    min_stars: i64,
    token: Option<&str>,
) -> anyhow::Result<BatchReport> {
    let (repos, total_entries) = parse_marketplace_repos(body);
    if repos.is_empty() {
        anyhow::bail!("no auditable GitHub repos found in marketplace index");
    }
    let mut audited = Vec::new();
    let mut failed = Vec::new();
    for repo in &repos {
        match fetch_repo_stats(repo, token).await {
            Ok(stats) => {
                let fork_ratio_pct =
                    stats.forks_count as f64 * 100.0 / stats.stargazers_count.max(1) as f64;
                let verdict = signalkit_integrity::fork_verdict(
                    stats.stargazers_count,
                    stats.forks_count,
                    min_stars,
                );
                audited.push(AuditReport {
                    repo: stats.full_name.clone(),
                    stats,
                    verdict,
                    fork_ratio_pct,
                });
            }
            Err(err) => failed.push((repo.clone(), err.to_string())),
        }
    }
    Ok(BatchReport {
        total_entries,
        audited,
        failed,
    })
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
    #[test]
    fn marketplace_parse_handles_url_and_string_shapes() {
        let json = r#"{"plugins": [
            {"name": "a", "source": "owner/one"},
            {"name": "b", "source": {"source": "git-subdir", "url": "https://github.com/owner/two.git", "path": "plugins/x", "ref": "main"}},
            {"name": "c", "source": "./relative"},
            {"name": "d", "source": {"source": "git-subdir", "repo": "owner/one"}},
            {"name": "e", "source": {"source": "url", "url": "https://gitlab.com/nope/none.git"}}
        ]}"#;
        let (repos, total) = parse_marketplace_repos(json);
        assert_eq!(total, 5);
        assert_eq!(repos, vec!["owner/one".to_owned(), "owner/two".to_owned()]);
    }

    #[test]
    fn marketplace_parse_rejects_garbage() {
        assert_eq!(parse_marketplace_repos("nope"), (Vec::<String>::new(), 0));
    }
}
