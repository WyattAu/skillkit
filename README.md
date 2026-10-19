# skillkit

[![crates.io](https://img.shields.io/crates/v/skillkit.svg)](https://crates.io/crates/skillkit) [![crates.io](https://img.shields.io/crates/v/skillkit-core.svg)](https://crates.io/crates/skillkit-core) [![crates.io](https://img.shields.io/crates/v/skillkit-store.svg)](https://crates.io/crates/skillkit-store)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.95+-orange.svg)](https://www.rust-lang.org/)

Tooling for the agent-skill economy: **lint, package, verify, and audit**
[SKILL.md](https://agentskills.io) skills.

```sh
skillkit lint ./my-skill        # strict frontmatter + structure lint
skillkit pack ./my-skill        # deterministic archive -> content-addressed store
skillkit verify ./my-skill --hash <blake3> --store ~/.local/share/skillkit/store
skillkit audit owner/repo       # engagement-integrity signals (fork-ratio heuristics)
skillkit audit anthropics/claude-plugins-official --batch
                                # audit every repo-plugin in a marketplace index
```

## Batch mode / trust reports

`audit --batch` parses a marketplace's `.claude-plugin/marketplace.json`,
audits every repo-sourced plugin, and prints flagged-first evidence. The
first run against Anthropic's official directory is checked in at
[`reports/claude-plugins-official-2026-10-19.txt`](reports/claude-plugins-official-2026-10-19.txt):
**227 of 315 entries audited — 1 EXTREME (`adobe/skills`, 41.8% fork
ratio), 9 elevated** (Salesforce, Oracle, GCP, Databricks, Microsoft —
the fork-to-customize vendor pattern).

## Why

Agent skills are becoming the Dockerfiles of AI agents — and the ecosystem
has formats and marketplaces but no tooling layer:

- **Lint** with unknown-field denial: the frontmatter grammar is tiny and
  anything else is a finding, because a format that accepts everything
  validates nothing.
- **Package** deterministically: same directory ⇒ same BLAKE3 hash ⇒ free
  deduplication, hash-verified updates.
- **Audit** engagement integrity: fork-ratio verdicts and other heuristics
  from [`signalkit-integrity`](https://crates.io/crates/signalkit-integrity),
  because trending metrics get farmed (forks are cheap; 46% fork ratios
  exist in the wild).

## Layered on the estate

| Crate | Role | Reuses |
|---|---|---|
| `skillkit-core` | parse + lint SKILL.md | — (zero-dep) |
| `skillkit-store` | canonical `SKILLPACK1` archives | [`cas-kit`](https://crates.io/crates/cas-kit) (BLAKE3 + zstd store) |
| `skillkit` (CLI) | commands + `audit` | [`signalkit-integrity`](https://crates.io/crates/signalkit-integrity), [`fetch-kit`](https://crates.io/crates/fetch-kit) |

Apache-2.0. Part of the [WyattAu](https://github.com/WyattAu) toolkit estate.
