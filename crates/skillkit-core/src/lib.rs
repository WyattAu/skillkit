//! skillkit-core — strict parsing and linting for SKILL.md agent skills.
//!
//! Philosophy mirrors the estate's config tools: **unknown-field
//! denial**. The frontmatter grammar is deliberately tiny (`name`,
//! `description`, `license`, `allowed-tools`); anything else is a
//! finding, because a skill format that accepts everything validates
//! nothing.

use std::fmt;

/// Severity of a lint finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth fixing; does not block.
    Warning,
    /// Blocks `pack`/`publish` (exit code reflects error count).
    Error,
}

/// One lint finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Severity.
    pub severity: Severity,
    /// Stable machine code, e.g. `unknown-field`.
    pub code: String,
    /// Human explanation.
    pub message: String,
}

impl Finding {
    fn error(code: &str, message: impl Into<String>) -> Finding {
        Finding {
            severity: Severity::Error,
            code: code.to_owned(),
            message: message.into(),
        }
    }

    fn warning(code: &str, message: impl Into<String>) -> Finding {
        Finding {
            severity: Severity::Warning,
            code: code.to_owned(),
            message: message.into(),
        }
    }

    /// Whether this finding blocks.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "[{level}] {}: {}", self.code, self.message)
    }
}

/// The typed skill manifest parsed from SKILL.md frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Skill name: lowercase letters/digits/hyphens, <= 64 chars.
    pub name: String,
    /// What the skill does, <= 1024 chars (agent-facing).
    pub description: String,
    /// Optional SPDX license id.
    pub license: Option<String>,
    /// Optional tool allowlist (comma-separated in frontmatter).
    pub allowed_tools: Vec<String>,
}

/// Errors from reading a skill directory.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    /// Filesystem failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A directory was given where a file was required, or similar.
    #[error("{0}")]
    Other(String),
}

/// Parse SKILL.md text: returns the manifest (when the frontmatter is
/// well-formed enough to extract one) plus every finding.
#[must_use]
pub fn parse_skill_md(text: &str) -> (Option<Manifest>, Vec<Finding>) {
    let mut findings = Vec::new();
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        findings.push(Finding::error(
            "missing-frontmatter",
            "SKILL.md must start with `---` frontmatter",
        ));
        return (None, findings);
    }

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut license: Option<String> = None;
    let mut allowed_tools: Option<Vec<String>> = None;
    let mut seen_keys: Vec<String> = Vec::new();
    let mut body_start: Option<usize> = None;
    let mut line_no = 1usize;

    for line in lines {
        line_no += 1;
        if line.trim() == "---" {
            body_start = Some(line_no);
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            findings.push(Finding::error(
                "bad-frontmatter-line",
                format!("line {line_no}: expected `key: value`, got `{line}`"),
            ));
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "name" => {
                if name.is_some() {
                    findings.push(Finding::error("duplicate-key", "`name` set twice"));
                }
                name = Some(value.to_owned());
            }
            "description" => {
                if description.is_some() {
                    findings.push(Finding::error("duplicate-key", "`description` set twice"));
                }
                description = Some(value.to_owned());
            }
            "license" => license = Some(value.to_owned()),
            "allowed-tools" => {
                allowed_tools = Some(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(str::to_owned)
                        .collect(),
                );
            }
            other => findings.push(Finding::error(
                "unknown-field",
                format!("line {line_no}: unknown frontmatter field `{other}` — the skill format accepts only name/description/license/allowed-tools"),
            )),
        }
        seen_keys.push(key.to_owned());
    }

    if body_start.is_none() {
        findings.push(Finding::error(
            "unclosed-frontmatter",
            "frontmatter is never closed with a second `---`",
        ));
    }

    let mut manifest = None;
    match (name, description) {
        (Some(n), Some(d)) => {
            manifest = Some(Manifest {
                name: n,
                description: d,
                license,
                allowed_tools: allowed_tools.unwrap_or_default(),
            });
        }
        (None, _) => findings.push(Finding::error("missing-name", "frontmatter has no `name`")),
        (_, None) => findings.push(Finding::error(
            "missing-description",
            "frontmatter has no `description`",
        )),
    }
    let _ = seen_keys;
    (manifest, findings)
}

/// Validate a manifest's field contents (name shape, description length).
#[must_use]
pub fn validate_manifest(manifest: &Manifest) -> Vec<Finding> {
    let mut findings = Vec::new();
    let valid_name = !manifest.name.is_empty()
        && manifest.name.len() <= 64
        && manifest
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !manifest.name.starts_with('-')
        && !manifest.name.ends_with('-');
    if !valid_name {
        findings.push(Finding::error(
            "bad-name",
            format!(
                "`name` must be lowercase letters/digits/hyphens, 1-64 chars, no leading/trailing hyphen (got `{}`)",
                manifest.name
            ),
        ));
    }
    if manifest.description.is_empty() {
        findings.push(Finding::error(
            "empty-description",
            "`description` is empty",
        ));
    } else if manifest.description.len() > 1024 {
        findings.push(Finding::error(
            "long-description",
            format!(
                "`description` is {} chars; the agent-facing limit is 1024",
                manifest.description.len()
            ),
        ));
    }
    if manifest.description.contains("TODO") {
        findings.push(Finding::warning(
            "placeholder-description",
            "`description` contains TODO — agents read this to decide when to load the skill",
        ));
    }
    findings
}

/// Lint a skill directory: SKILL.md frontmatter, body, and referenced
/// relative files.
///
/// # Errors
/// Fails when the directory cannot be read.
pub fn lint_dir(dir: &std::path::Path) -> Result<Vec<Finding>, SkillError> {
    let skill_path = dir.join("SKILL.md");
    let text = std::fs::read_to_string(&skill_path)
        .map_err(|e| SkillError::Other(format!("cannot read {}: {e}", skill_path.display())))?;
    let (manifest, mut findings) = parse_skill_md(&text);

    if let Some(m) = &manifest {
        findings.extend(validate_manifest(m));
    }

    // Body must exist and say something.
    let has_body = text
        .split_once("---\n")
        .and_then(|(_, rest)| rest.split_once("---"))
        .is_some_and(|(_, body)| body.trim().len() > 32);
    if !findings.iter().any(|f| f.code == "unclosed-frontmatter") && !has_body {
        findings.push(Finding::error(
            "empty-body",
            "skill body is empty or under 32 chars — instructions are the point of a skill",
        ));
    }

    // Relative markdown-link targets must exist.
    for (idx, line) in text.lines().enumerate() {
        for candidate in relative_link_targets(line) {
            if !dir.join(&candidate).exists() {
                findings.push(Finding::error(
                    "missing-referenced-file",
                    format!(
                        "line {}: links to `{candidate}` which does not exist",
                        idx + 1
                    ),
                ));
            }
        }
    }

    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.code.cmp(&b.code))
    });
    Ok(findings)
}

/// Extract relative link targets `(path)` from one markdown line;
/// skips URLs and anchors.
fn relative_link_targets(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b']' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            if let Some(close) = line[i + 2..].find(')') {
                let target = &line[i + 2..i + 2 + close];
                if !target.is_empty()
                    && !target.contains("://")
                    && !target.starts_with('#')
                    && !target.starts_with('/')
                {
                    out.push(target.to_owned());
                }
                i += 2 + close;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const GOOD: &str = "---\nname: my-skill\ndescription: Does one useful thing.\n---\n\n## Use\n\nStep one: read the referenced file (see [details](details.md)).\n";

    #[test]
    fn good_skill_has_no_errors() {
        let (manifest, findings) = parse_skill_md(GOOD);
        assert!(manifest.is_some());
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn unknown_fields_are_denied() {
        let text = "---\nname: x\ndescription: y\nmodel: gpt-9\n---\nbody";
        let (manifest, findings) = parse_skill_md(text);
        assert!(manifest.is_some());
        assert!(findings.iter().any(|f| f.code == "unknown-field"));
    }

    #[test]
    fn missing_frontmatter_is_fatal() {
        let (manifest, findings) = parse_skill_md("just text");
        assert!(manifest.is_none());
        assert!(findings.iter().any(|f| f.code == "missing-frontmatter"));
    }

    #[test]
    fn bad_names_are_flagged() {
        let text = "---\nname: My Skill\ndescription: ok\n---\nbody content here";
        let (manifest, findings) = parse_skill_md(text);
        let m = manifest.unwrap();
        let mut all = findings;
        all.extend(validate_manifest(&m));
        assert!(all.iter().any(|f| f.code == "bad-name"));
    }

    #[test]
    fn allowed_tools_split_on_commas() {
        let text = "---\nname: x\ndescription: y\nallowed-tools: read, write, bash\n---\nbody";
        let (manifest, _) = parse_skill_md(text);
        let m = manifest.unwrap();
        assert_eq!(m.allowed_tools, vec!["read", "write", "bash"]);
    }

    #[test]
    fn lint_dir_reports_missing_links() {
        let dir = std::env::temp_dir().join(format!("skillkit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), GOOD).unwrap();
        let findings = lint_dir(&dir).unwrap();
        assert!(findings.iter().any(|f| f.code == "missing-referenced-file"));
        std::fs::write(dir.join("details.md"), "details").unwrap();
        let findings = lint_dir(&dir).unwrap();
        assert!(!findings.iter().any(|f| f.code == "missing-referenced-file"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
