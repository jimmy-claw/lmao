//! Skill marketplace — local publish, list, and inspect commands.
//!
//! Skills are stored as JSON bundles in `~/.config/lmao/skills/<id>.json`.
//! Phase 1 implements local-only storage; on-chain LEZ integration is
//! deferred to a later phase.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::cli::SkillAction;

// ── Data model ──────────────────────────────────────────────────────────────

/// A published skill bundle stored locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillBundle {
    /// Unique identifier (UUID v4).
    pub id: String,
    /// Human-readable skill name.
    pub name: String,
    /// One-sentence description.
    pub description: String,
    /// Searchable tags.
    pub tags: Vec<String>,
    /// Author name / handle.
    pub author: String,
    /// Semantic version string, e.g. "1.0.0".
    pub version: String,
    /// SHA-256 hex digest of the raw skill file contents.
    pub content_hash: String,
    /// ISO 8601 timestamp when this bundle was created.
    pub published_at: String,
    /// Absolute path to the original skill file on disk.
    pub path: String,
}

// ── Directory helpers ────────────────────────────────────────────────────────

/// Returns `~/.config/lmao/skills/`, creating it if necessary.
fn skills_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| anyhow!("cannot locate config directory"))?;
    let dir = base.join("lmao").join("skills");
    fs::create_dir_all(&dir).with_context(|| format!("creating skills dir {}", dir.display()))?;
    Ok(dir)
}

/// Path for a specific skill bundle file.
fn bundle_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{}.json", id))
}

// ── Parsing helpers ──────────────────────────────────────────────────────────

/// Extract the first Markdown heading (`# Heading`) from skill file content.
/// Falls back to the directory / file name when no heading is found.
fn extract_name(content: &str, fallback: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let name = rest.trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    fallback.to_string()
}

/// Extract a description from the skill file.
///
/// Looks for a line matching `Description: ...` (case-insensitive).
/// Falls back to the first non-heading, non-empty paragraph.
fn extract_description(content: &str) -> String {
    // Explicit "Description: ..." line
    for line in content.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("description:") {
            if let Some(colon_pos) = line.find(':') {
                let desc = line[colon_pos + 1..].trim().to_string();
                if !desc.is_empty() {
                    return desc;
                }
            }
        }
    }

    // First non-heading, non-empty paragraph
    let mut past_heading = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            past_heading = true;
            continue;
        }
        if past_heading && !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    String::new()
}

/// Extract tags from the skill file.
///
/// Looks for a line matching `Tags: foo, bar, baz` (case-insensitive).
fn extract_tags(content: &str) -> Vec<String> {
    for line in content.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("tags:") {
            if let Some(colon_pos) = line.find(':') {
                let tag_str = line[colon_pos + 1..].trim();
                return tag_str
                    .split(',')
                    .map(|t| t.trim().to_lowercase())
                    .filter(|t| !t.is_empty())
                    .collect();
            }
        }
    }
    vec![]
}

/// Resolve the skill file path (accepts a directory containing SKILL.md or a
/// direct path to any `.md` file).
fn resolve_skill_file(path: &Path) -> Result<PathBuf> {
    let p = path
        .canonicalize()
        .with_context(|| format!("path not found or not accessible: {}", path.display()))?;

    if p.is_dir() {
        let candidate = p.join("SKILL.md");
        if candidate.exists() {
            return Ok(candidate);
        }
        return Err(anyhow!(
            "directory {} does not contain SKILL.md",
            p.display()
        ));
    }

    Ok(p)
}

/// Determine the author: `--author` flag -> git `user.name` -> `$USER`.
fn resolve_author(author_flag: Option<&str>) -> String {
    if let Some(a) = author_flag {
        if !a.trim().is_empty() {
            return a.trim().to_string();
        }
    }

    // Try git config
    if let Ok(out) = Command::new("git")
        .args(["config", "--global", "user.name"])
        .output()
    {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }

    // Fall back to $USER
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
}

/// Compute SHA-256 hex digest of bytes.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ── Command handlers ─────────────────────────────────────────────────────────

/// Dispatch a `skill` subcommand.
pub fn handle(action: SkillAction, json: bool) -> Result<()> {
    match action {
        SkillAction::Publish {
            path,
            name,
            author,
            version,
            tags,
        } => publish(path, name, author, version, tags, json),
        SkillAction::List { tag } => list(tag, json),
        SkillAction::Info { id } => info(&id, json),
    }
}

/// `skill publish` -- bundle a skill file and write it to the skills directory.
fn publish(
    path: PathBuf,
    name_flag: Option<String>,
    author_flag: Option<String>,
    version: String,
    tags_flag: Option<String>,
    json: bool,
) -> Result<()> {
    let skill_file = resolve_skill_file(&path)?;
    let content = fs::read_to_string(&skill_file)
        .with_context(|| format!("reading skill file {}", skill_file.display()))?;

    // Derive name: flag > heading > file stem
    let fallback_name = skill_file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let name = name_flag
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| extract_name(&content, &fallback_name));

    let description = extract_description(&content);

    // Tags: flag overrides parsed tags
    let tags = if let Some(t) = tags_flag {
        t.split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        extract_tags(&content)
    };

    let author = resolve_author(author_flag.as_deref());
    let id = Uuid::new_v4().to_string();
    let content_hash = sha256_hex(content.as_bytes());
    let published_at = Utc::now().to_rfc3339();
    let abs_path = skill_file.to_string_lossy().to_string();

    let bundle = SkillBundle {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        tags: tags.clone(),
        author: author.clone(),
        version: version.clone(),
        content_hash,
        published_at: published_at.clone(),
        path: abs_path,
    };

    let dir = skills_dir()?;
    let dest = bundle_path(&dir, &id);
    let serialized = serde_json::to_string_pretty(&bundle)?;
    fs::write(&dest, &serialized)
        .with_context(|| format!("writing skill bundle to {}", dest.display()))?;

    if json {
        println!("{}", serde_json::to_string(&bundle)?);
    } else {
        println!("Skill published");
        println!("  ID:          {id}");
        println!("  Name:        {name}");
        println!("  Author:      {author}");
        println!("  Version:     {version}");
        println!("  Tags:        {}", tags.join(", "));
        println!("  Description: {description}");
        println!("  Saved to:    {}", dest.display());
    }

    Ok(())
}

/// `skill list` -- list all locally published skills, with optional tag filter.
fn list(tag_filter: Option<String>, json: bool) -> Result<()> {
    let dir = skills_dir()?;
    let mut bundles: Vec<SkillBundle> = vec![];

    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        match serde_json::from_str::<SkillBundle>(&raw) {
            Ok(b) => bundles.push(b),
            Err(e) => {
                eprintln!("warning: skipping malformed bundle {}: {e}", path.display());
            }
        }
    }

    // Apply tag filter
    if let Some(ref tag) = tag_filter {
        let tag_lc = tag.to_lowercase();
        bundles.retain(|b| b.tags.iter().any(|t| t == &tag_lc));
    }

    // Sort by published_at descending (newest first)
    bundles.sort_by(|a, b| b.published_at.cmp(&a.published_at));

    if json {
        println!("{}", serde_json::to_string(&bundles)?);
        return Ok(());
    }

    if bundles.is_empty() {
        if let Some(ref tag) = tag_filter {
            println!("No skills found with tag '{tag}'.");
        } else {
            println!("No skills published yet. Run `skill publish --path <SKILL.md>` to add one.");
        }
        return Ok(());
    }

    println!(
        "{:<38} {:<24} {:<16} {:<10} Tags",
        "ID", "Name", "Author", "Version"
    );
    println!("{}", "-".repeat(110));
    for b in &bundles {
        println!(
            "{:<38} {:<24} {:<16} {:<10} {}",
            b.id,
            truncate(&b.name, 22),
            truncate(&b.author, 14),
            b.version,
            b.tags.join(", ")
        );
    }

    Ok(())
}

/// `skill info <id>` -- show full details of a skill bundle.
fn info(id: &str, json: bool) -> Result<()> {
    let dir = skills_dir()?;

    // Support prefix matching
    let bundle = find_bundle(&dir, id)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&bundle)?);
        return Ok(());
    }

    println!("ID:           {}", bundle.id);
    println!("Name:         {}", bundle.name);
    println!("Author:       {}", bundle.author);
    println!("Version:      {}", bundle.version);
    println!("Tags:         {}", bundle.tags.join(", "));
    println!("Description:  {}", bundle.description);
    println!("Content hash: {}", bundle.content_hash);
    println!("Published:    {}", bundle.published_at);
    println!("Path:         {}", bundle.path);

    Ok(())
}

// ── Utilities ────────────────────────────────────────────────────────────────

/// Find a bundle by exact ID or unambiguous prefix.
fn find_bundle(dir: &Path, id_or_prefix: &str) -> Result<SkillBundle> {
    let mut matches = vec![];

    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if stem == id_or_prefix || stem.starts_with(id_or_prefix) {
            let raw =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            if let Ok(b) = serde_json::from_str::<SkillBundle>(&raw) {
                matches.push(b);
            }
        }
    }

    match matches.len() {
        0 => Err(anyhow!(
            "no skill found with id (or prefix) '{id_or_prefix}'"
        )),
        1 => Ok(matches.remove(0)),
        n => Err(anyhow!(
            "ambiguous prefix '{id_or_prefix}' -- matches {n} skills; use a longer prefix"
        )),
    }
}

/// Truncate a string to `max` characters, appending `...` if needed.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_skill_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let p = dir.path().join(name);
        fs::write(&p, content).unwrap();
        p
    }

    // ── extract_name ──

    #[test]
    fn extract_name_from_heading() {
        let md = "# My Skill\n\nDoes things.";
        assert_eq!(extract_name(md, "fallback"), "My Skill");
    }

    #[test]
    fn extract_name_falls_back() {
        let md = "No heading here.";
        assert_eq!(extract_name(md, "my-skill"), "my-skill");
    }

    #[test]
    fn extract_name_ignores_h2() {
        let md = "## Not a top heading\n# Real";
        assert_eq!(extract_name(md, "fb"), "Real");
    }

    // ── extract_description ──

    #[test]
    fn extract_description_explicit_label() {
        let md = "# Skill\n\nDescription: Does cool things.";
        assert_eq!(extract_description(md), "Does cool things.");
    }

    #[test]
    fn extract_description_first_paragraph() {
        let md = "# Skill\n\nThis is the first paragraph.";
        assert_eq!(extract_description(md), "This is the first paragraph.");
    }

    #[test]
    fn extract_description_empty_when_nothing() {
        let md = "# Skill";
        assert_eq!(extract_description(md), "");
    }

    // ── extract_tags ──

    #[test]
    fn extract_tags_comma_separated() {
        let md = "# Skill\n\nTags: rust, cli, test";
        assert_eq!(extract_tags(md), vec!["rust", "cli", "test"]);
    }

    #[test]
    fn extract_tags_case_insensitive_key() {
        let md = "TAGS: foo, Bar";
        assert_eq!(extract_tags(md), vec!["foo", "bar"]);
    }

    #[test]
    fn extract_tags_empty_when_missing() {
        let md = "# No tags here";
        assert!(extract_tags(md).is_empty());
    }

    // ── sha256_hex ──

    #[test]
    fn sha256_is_stable() {
        let h1 = sha256_hex(b"hello");
        let h2 = sha256_hex(b"hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn sha256_differs_for_different_input() {
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
    }

    // ── resolve_skill_file ──

    #[test]
    fn resolve_direct_md_file() {
        let dir = TempDir::new().unwrap();
        let p = make_skill_file(&dir, "skill.md", "# S");
        let resolved = resolve_skill_file(&p).unwrap();
        assert_eq!(resolved, p.canonicalize().unwrap());
    }

    #[test]
    fn resolve_directory_with_skill_md() {
        let dir = TempDir::new().unwrap();
        let skill_md = dir.path().join("SKILL.md");
        fs::write(&skill_md, "# S").unwrap();
        let resolved = resolve_skill_file(dir.path()).unwrap();
        assert_eq!(resolved, skill_md.canonicalize().unwrap());
    }

    #[test]
    fn resolve_directory_missing_skill_md_errors() {
        let dir = TempDir::new().unwrap();
        let err = resolve_skill_file(dir.path()).unwrap_err();
        assert!(err.to_string().contains("does not contain SKILL.md"));
    }

    #[test]
    fn resolve_nonexistent_path_errors() {
        let err = resolve_skill_file(Path::new("/tmp/does_not_exist_xyz123.md")).unwrap_err();
        assert!(
            err.to_string().contains("not found")
                || err.to_string().contains("No such file")
                || err.to_string().contains("not accessible")
        );
    }

    // ── truncate ──

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_has_ellipsis() {
        let s = truncate("abcdefghij", 5);
        assert!(s.ends_with("..."));
    }

    // ── find_bundle ──

    fn write_bundle(dir: &Path, bundle: &SkillBundle) {
        let path = bundle_path(dir, &bundle.id);
        fs::write(path, serde_json::to_string_pretty(bundle).unwrap()).unwrap();
    }

    fn make_bundle(id: &str, name: &str, tags: &[&str]) -> SkillBundle {
        SkillBundle {
            id: id.to_string(),
            name: name.to_string(),
            description: "A test skill.".to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            author: "tester".to_string(),
            version: "1.0.0".to_string(),
            content_hash: sha256_hex(name.as_bytes()),
            published_at: "2024-01-01T00:00:00Z".to_string(),
            path: "/tmp/fake-skill.md".to_string(),
        }
    }

    #[test]
    fn find_bundle_exact_id() {
        let dir = TempDir::new().unwrap();
        let b = make_bundle("aaaa-1111", "Alpha", &["a"]);
        write_bundle(dir.path(), &b);
        let found = find_bundle(dir.path(), "aaaa-1111").unwrap();
        assert_eq!(found.id, "aaaa-1111");
    }

    #[test]
    fn find_bundle_prefix() {
        let dir = TempDir::new().unwrap();
        let b = make_bundle("bbbb-2222-cccc", "Beta", &["b"]);
        write_bundle(dir.path(), &b);
        let found = find_bundle(dir.path(), "bbbb").unwrap();
        assert_eq!(found.id, "bbbb-2222-cccc");
    }

    #[test]
    fn find_bundle_missing_errors() {
        let dir = TempDir::new().unwrap();
        let err = find_bundle(dir.path(), "no-such-id").unwrap_err();
        assert!(err.to_string().contains("no skill found"));
    }

    #[test]
    fn find_bundle_ambiguous_prefix_errors() {
        let dir = TempDir::new().unwrap();
        write_bundle(dir.path(), &make_bundle("aaa-111", "One", &[]));
        write_bundle(dir.path(), &make_bundle("aaa-222", "Two", &[]));
        let err = find_bundle(dir.path(), "aaa").unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn bundle_roundtrip_json() {
        let b = make_bundle("id-123", "MySkill", &["rust", "cli"]);
        let json = serde_json::to_string(&b).unwrap();
        let back: SkillBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, b.id);
        assert_eq!(back.tags, b.tags);
    }

    #[test]
    fn malformed_json_skipped_silently_in_find() {
        let dir = TempDir::new().unwrap();
        let good = make_bundle("good-id", "Good", &[]);
        write_bundle(dir.path(), &good);
        fs::write(dir.path().join("bad.json"), "not json at all").unwrap();

        // find_bundle should still find the good one despite bad.json
        let found = find_bundle(dir.path(), "good-id").unwrap();
        assert_eq!(found.id, "good-id");
    }
}
