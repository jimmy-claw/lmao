use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Default nwaku REST API URL.
pub const DEFAULT_WAKU_URL: &str = "http://localhost:8645";

/// Nested agent configuration.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub name: Option<String>,
    pub capabilities: Option<String>,
    pub description: Option<String>,
}

/// Nested presence configuration.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct PresenceConfig {
    pub ttl: Option<u64>,
    pub auto_announce: Option<bool>,
}

/// Top-level TOML configuration.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct LmaoConfig {
    pub waku_url: Option<String>,
    pub keyfile: Option<PathBuf>,
    pub encrypt: Option<bool>,
    pub json: Option<bool>,
    pub agent: Option<AgentConfig>,
    pub presence: Option<PresenceConfig>,
}

/// Load configuration from file, searching in order:
///
/// 1. Explicit path (if provided — errors when file is missing)
/// 2. `./lmao.toml`
/// 3. `~/.config/lmao/config.toml`
///
/// Returns a default (empty) config when no file is found.
pub fn load_config(explicit_path: Option<&Path>) -> Result<LmaoConfig> {
    if let Some(path) = explicit_path {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        return parse_config(&content);
    }

    let local = Path::new("lmao.toml");
    if local.exists() {
        let content = fs::read_to_string(local).context("failed to read ./lmao.toml")?;
        return parse_config(&content);
    }

    if let Some(config_dir) = dirs::config_dir() {
        let global = config_dir.join("lmao").join("config.toml");
        if global.exists() {
            let content = fs::read_to_string(&global)
                .with_context(|| format!("failed to read {}", global.display()))?;
            return parse_config(&content);
        }
    }

    Ok(LmaoConfig::default())
}

fn parse_config(content: &str) -> Result<LmaoConfig> {
    toml::from_str(content).context("invalid TOML in configuration file")
}

/// Merge CLI flags with a loaded config.
///
/// Precedence: CLI flag > config file > hard-coded default.
pub fn merge(
    cli_waku: Option<String>,
    cli_keyfile: Option<PathBuf>,
    cli_encrypt: bool,
    cli_json: bool,
    config: &LmaoConfig,
) -> (String, Option<PathBuf>, bool, bool) {
    let waku = cli_waku
        .or_else(|| config.waku_url.clone())
        .unwrap_or_else(|| DEFAULT_WAKU_URL.to_string());
    let keyfile = cli_keyfile.or_else(|| config.keyfile.clone());
    let encrypt = cli_encrypt || config.encrypt.unwrap_or(false);
    let json = cli_json || config.json.unwrap_or(false);
    (waku, keyfile, encrypt, json)
}

// ── Config subcommand helpers ───────────────────────────────────────

/// Default configuration path: `~/.config/lmao/config.toml`.
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("lmao").join("config.toml"))
}

const DEFAULT_CONFIG: &str = r#"# logos-messaging-a2a configuration file
# CLI flags override values set here.

# nwaku REST API URL
# waku_url = "http://localhost:8645"

# Path to persistent identity keyfile
# keyfile = "~/.config/lmao/identity.key"

# Enable X25519+ChaCha20-Poly1305 encryption
# encrypt = false

# Output structured JSON instead of human-readable text
# json = false

# [agent]
# name = "my-agent"
# capabilities = "text"
# description = "My A2A agent"

# [presence]
# ttl = 300
# auto_announce = false
"#;

/// Create a default configuration file at the given path.
pub fn init_config_at(target: &Path, json_mode: bool) -> Result<()> {
    if target.exists() {
        if json_mode {
            println!(
                "{}",
                serde_json::json!({
                    "status": "exists",
                    "path": target.display().to_string(),
                })
            );
        } else {
            println!("Config file already exists: {}", target.display());
        }
        return Ok(());
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    fs::write(target, DEFAULT_CONFIG)
        .with_context(|| format!("failed to write {}", target.display()))?;

    if json_mode {
        println!(
            "{}",
            serde_json::json!({
                "status": "created",
                "path": target.display().to_string(),
            })
        );
    } else {
        println!("Created config file: {}", target.display());
    }

    Ok(())
}

/// `config init` — create `~/.config/lmao/config.toml`.
pub fn init_config(json_mode: bool) -> Result<()> {
    let path = default_config_path().context("could not determine config directory")?;
    init_config_at(&path, json_mode)
}

/// `config show` — print the effective (merged) configuration.
pub fn show_config(effective: &LmaoConfig, json_mode: bool) -> Result<()> {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(effective)?);
    } else {
        print!("{}", toml::to_string_pretty(effective)?);
    }
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Parsing ──

    #[test]
    fn parse_full_config() {
        let toml_str = r#"
waku_url = "http://custom:9090"
keyfile = "/path/to/key"
encrypt = true
json = true

[agent]
name = "test-agent"
capabilities = "text,code"
description = "A test agent"

[presence]
ttl = 600
auto_announce = true
"#;
        let config: LmaoConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.waku_url.as_deref(), Some("http://custom:9090"));
        assert_eq!(config.keyfile, Some(PathBuf::from("/path/to/key")));
        assert_eq!(config.encrypt, Some(true));
        assert_eq!(config.json, Some(true));

        let agent = config.agent.unwrap();
        assert_eq!(agent.name.as_deref(), Some("test-agent"));
        assert_eq!(agent.capabilities.as_deref(), Some("text,code"));
        assert_eq!(agent.description.as_deref(), Some("A test agent"));

        let presence = config.presence.unwrap();
        assert_eq!(presence.ttl, Some(600));
        assert_eq!(presence.auto_announce, Some(true));
    }

    #[test]
    fn parse_partial_config() {
        let toml_str = r#"
waku_url = "http://mynode:8080"
encrypt = true
"#;
        let config: LmaoConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.waku_url.as_deref(), Some("http://mynode:8080"));
        assert!(config.keyfile.is_none());
        assert_eq!(config.encrypt, Some(true));
        assert!(config.json.is_none());
        assert!(config.agent.is_none());
        assert!(config.presence.is_none());
    }

    #[test]
    fn parse_empty_config() {
        let config: LmaoConfig = toml::from_str("").unwrap();
        assert!(config.waku_url.is_none());
        assert!(config.keyfile.is_none());
        assert!(config.encrypt.is_none());
        assert!(config.json.is_none());
        assert!(config.agent.is_none());
        assert!(config.presence.is_none());
    }

    #[test]
    fn invalid_toml_produces_helpful_error() {
        let result = parse_config("this is not [valid toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid TOML"), "got: {err}");
    }

    // ── File loading ──

    #[test]
    fn load_from_explicit_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");
        fs::write(&path, r#"waku_url = "http://test:1234""#).unwrap();

        let config = load_config(Some(path.as_path())).unwrap();
        assert_eq!(config.waku_url.as_deref(), Some("http://test:1234"));
    }

    #[test]
    fn explicit_path_not_found_errors() {
        let result = load_config(Some(Path::new("/nonexistent/config.toml")));
        assert!(result.is_err());
    }

    #[test]
    fn no_config_returns_default() {
        // load_config(None) should never panic; it may find a local
        // lmao.toml (CWD-dependent) or return the empty default.
        let config = load_config(None).unwrap();
        // At minimum it should be a valid LmaoConfig
        let _ = config.waku_url; // no panic
    }

    #[test]
    fn config_search_prefers_explicit_over_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("explicit.toml");
        fs::write(&path, r#"waku_url = "http://explicit:1111""#).unwrap();

        let config = load_config(Some(path.as_path())).unwrap();
        assert_eq!(config.waku_url.as_deref(), Some("http://explicit:1111"));
    }

    // ── Merge precedence ──

    #[test]
    fn merge_cli_overrides_config() {
        let config = LmaoConfig {
            waku_url: Some("http://config:8080".to_string()),
            keyfile: Some(PathBuf::from("/config/key")),
            encrypt: Some(false),
            json: Some(true),
            ..Default::default()
        };

        let (waku, keyfile, encrypt, json) = merge(
            Some("http://cli:9090".to_string()),
            Some(PathBuf::from("/cli/key")),
            true,
            false,
            &config,
        );

        assert_eq!(waku, "http://cli:9090");
        assert_eq!(keyfile, Some(PathBuf::from("/cli/key")));
        assert!(encrypt);
        assert!(json); // config true wins (CLI false || config true)
    }

    #[test]
    fn merge_config_overrides_default() {
        let config = LmaoConfig {
            waku_url: Some("http://config:8080".to_string()),
            keyfile: Some(PathBuf::from("/config/key")),
            encrypt: Some(true),
            json: Some(true),
            ..Default::default()
        };

        let (waku, keyfile, encrypt, json) = merge(None, None, false, false, &config);

        assert_eq!(waku, "http://config:8080");
        assert_eq!(keyfile, Some(PathBuf::from("/config/key")));
        assert!(encrypt);
        assert!(json);
    }

    #[test]
    fn merge_falls_back_to_defaults() {
        let config = LmaoConfig::default();

        let (waku, keyfile, encrypt, json) = merge(None, None, false, false, &config);

        assert_eq!(waku, DEFAULT_WAKU_URL);
        assert!(keyfile.is_none());
        assert!(!encrypt);
        assert!(!json);
    }

    #[test]
    fn merge_bool_flags_are_additive() {
        // If config says encrypt=true but CLI doesn't pass --encrypt,
        // the result should still be true.
        let config = LmaoConfig {
            encrypt: Some(true),
            ..Default::default()
        };

        let (_, _, encrypt, _) = merge(None, None, false, false, &config);
        assert!(encrypt);
    }

    // ── Init ──

    #[test]
    fn init_creates_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lmao").join("config.toml");

        init_config_at(&path, false).unwrap();
        assert!(path.exists());

        // File should contain valid TOML
        let content = fs::read_to_string(&path).unwrap();
        let config: LmaoConfig = toml::from_str(&content).unwrap();
        // All values are commented out → empty config
        assert!(config.waku_url.is_none());
    }

    #[test]
    fn init_does_not_overwrite_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, r#"waku_url = "http://keep:1234""#).unwrap();

        init_config_at(&path, false).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("http://keep:1234"));
    }

    #[test]
    fn default_config_template_is_valid_toml() {
        let config: LmaoConfig = toml::from_str(DEFAULT_CONFIG).unwrap();
        assert!(config.waku_url.is_none());
    }

    // ── Show ──

    #[test]
    fn show_config_json_serializable() {
        let config = LmaoConfig {
            waku_url: Some("http://test:8645".to_string()),
            encrypt: Some(false),
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("http://test:8645"));
    }

    #[test]
    fn show_config_toml_serializable() {
        let config = LmaoConfig {
            waku_url: Some("http://test:8645".to_string()),
            encrypt: Some(false),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("http://test:8645"));
    }

    #[test]
    fn default_config_path_returns_some() {
        // On most systems dirs::config_dir() returns Some
        let path = default_config_path();
        if let Some(p) = path {
            assert!(p.ends_with("lmao/config.toml"));
        }
    }
}
