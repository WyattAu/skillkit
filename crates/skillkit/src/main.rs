//! skillkit CLI — lint, package, verify, audit agent skills.

mod audit;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Tooling for the agent-skill economy: lint, package, verify, audit.
#[derive(Debug, Parser)]
#[command(name = "skillkit", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Lint a skill directory (SKILL.md frontmatter + structure).
    Lint {
        /// Skill directory containing SKILL.md.
        path: std::path::PathBuf,
    },
    /// Package a skill directory into the content-addressed store.
    Pack {
        /// Skill directory containing SKILL.md.
        path: std::path::PathBuf,
        /// Store root (default: ~/.local/share/skillkit/store).
        #[arg(long)]
        store: Option<std::path::PathBuf>,
    },
    /// Verify a skill directory against a previously packaged hash.
    Verify {
        /// Skill directory containing SKILL.md.
        path: std::path::PathBuf,
        /// Blake3 hash printed by `pack`.
        #[arg(long)]
        hash: String,
        /// Store root used at pack time.
        #[arg(long)]
        store: std::path::PathBuf,
    },
    /// Audit a GitHub repo for engagement-integrity signals.
    Audit {
        /// `owner/repo` to audit.
        repo: String,
        /// Minimum stars before fork-ratio is judged.
        #[arg(long, default_value_t = 200)]
        min_stars: i64,
    },
}

/// Resolve the GitHub token for audit calls: `GH_TOKEN` env when set.
fn github_token() -> Option<String> {
    std::env::var("GH_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command).await {
        Ok(0) => ExitCode::SUCCESS,
        Ok(errors) => ExitCode::from(u8::try_from(errors).unwrap_or(u8::MAX)),
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> anyhow::Result<usize> {
    match command {
        Command::Lint { path } => {
            let findings = skillkit_core::lint_dir(&path)?;
            for f in &findings {
                println!("{f}");
            }
            let errors = findings.iter().filter(|f| f.is_error()).count();
            println!(
                "{} finding(s): {} error(s), {} warning(s)",
                findings.len(),
                errors,
                findings.len() - errors
            );
            Ok(errors)
        }
        Command::Pack { path, store } => {
            let root = store.unwrap_or_else(default_store_root);
            let archive = skillkit_store::archive_dir(&path)?;
            let store = cas_kit::store::BlobStore::new(&root)
                .map_err(|e| anyhow::anyhow!("open store at {}: {e}", root.display()))?;
            let hash = store
                .put_blob(&archive.bytes)
                .map_err(|e| anyhow::anyhow!("store put failed: {e}"))?;
            println!(
                "archived {} file(s), {} bytes",
                archive.file_count,
                archive.bytes.len()
            );
            println!("hash: {}", hash.to_hex());
            Ok(0)
        }
        Command::Verify { path, hash, store } => {
            let archive = skillkit_store::archive_dir(&path)?;
            let expected =
                cas_kit::Hash::from_hex(&hash).map_err(|e| anyhow::anyhow!("bad hash: {e}"))?;
            let actual = cas_kit::hash_bytes(&archive.bytes);
            if actual.to_hex() != expected.to_hex() {
                anyhow::bail!(
                    "MISMATCH: dir hashes to {}, expected {}",
                    actual.to_hex(),
                    expected.to_hex()
                );
            }
            let root = cas_kit::store::BlobStore::new(&store)
                .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
            if !root.has_blob(&expected) {
                anyhow::bail!(
                    "hash matches the directory but blob {} is not in the store",
                    hash
                );
            }
            println!(
                "verified: {} ({} bytes)",
                expected.to_hex(),
                archive.bytes.len()
            );
            Ok(0)
        }
        Command::Audit { repo, min_stars } => {
            let report = audit::audit_repo(&repo, min_stars, github_token().as_deref()).await?;
            println!("{}", report.render());
            Ok(0)
        }
    }
}

fn default_store_root() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    std::path::PathBuf::from(home).join(".local/share/skillkit/store")
}
