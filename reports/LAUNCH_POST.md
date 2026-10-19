# Show HN draft — trust report launch

**Title options:**
- We fork-ratio-audited Anthropic's official Claude plugin directory (227 plugins)
- Trending metrics are getting farmed, so we built a fork-ratio auditor and ran it on the official plugin marketplace

**Body:**

We've been snapshotting GitHub/HN/crates.io/HuggingFace velocity daily
(trendkit), and kept noticing repos where forks grew as fast as stars —
sometimes faster (one repo: 153.9% fork ratio; one org: 7 repos bursting
1,269 stars/day combined). Forks are the cheapest engagement signal to
farm, and every ranking surface consumes them raw.

So we built skillkit: a CLI that lints/packages SKILL.md agent skills and
audits engagement integrity. Today we ran `skillkit audit --batch`
against Anthropic's official Claude plugin directory:

- 227 of 315 entries are git-repo sourced and auditable
- 1 EXTREME (41.8% fork ratio), 9 elevated (22-34%) — mostly
  vendor "fork-to-customize" skills (Salesforce, Oracle, GCP,
  Databricks, Microsoft), which is the honest caveat: fork ratio is a
  smoke detector, not a conviction. Users legitimately fork skills to
  customize them. The distinguishing farm shape needs org-burst +
  age + cross-repo correlation, which the underlying engine
  (signalkit-integrity) also computes.

Full report: reports/claude-plugins-official-2026-10-19.txt
Tooling: github.com/WyattAu/skillkit (core+store on crates.io, zero-dep
linter, BLAKE3 content-addressed packaging)
Engine: github.com/WyattAu/signalkit

Everything is Apache-2.0. Evidence numbers are printed next to every
verdict — we'd rather show the data than pretend a heuristic is a judge.
