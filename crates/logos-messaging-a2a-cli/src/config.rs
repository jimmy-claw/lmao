use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level TOML configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub waku_url: Option<String>,
    pub keyfile: Option<String>,
    pub encrypt: Option<bool>,
    pub json: Option<bool>,

    #[serde(default)]
    pub agent: AgentConfig,

    #[serde(default)]
    pub presence: PresenceConfig,
}

/// Agent-specific configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    pub name: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub description: Option<String>,
}

/// Presence-specific configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PresenceConfig {
    pub ttl: Option<u64>,
    pub auto_announce: Option<bool>,
}

/// Resolved configuration after merging file + CLI.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedConfig {
    pub waku_url: String,
    pub keyfile: Option<PathBuf>,
    pub encrypt: bool,
    pub json: bool,
    pub agent: AgentConfig,
    pub presence: PresenceConfig,
}

/// Search order for config files:
/// 1. Explicit `--config <path>` (if provided)
/// 2. `./lmao.toml`
/// 3. `~/.config/lmao/config.toml`
///
/// Returns the parsed config and the path it was loaded from.
pub fn load_config(explicit_path: Option<&Path>) -> Result<(Config, Option<PathBuf>)> {
    if let Some(path) = explicit_path {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("failed to parse config file: {}", path.display()))?;
        return Ok((config, Some(path.to_path_buf())));
    }

    // Try ./lmao.toml
    let local = PathBuf::from("lmao.toml");
    if local.exists() {
        let content = std::fs::read_to_string(&local)
            .with_context(|| format!("failed to read config file: {}", local.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("failed to parse config file: {}", local.display()))?;
        return Ok((config, Some(local)));
    }

    // Try ~/.config/lmao/config.toml
    if let Some(config_dir) = dirs::config_dir() {
        let global = config_dir.join("lmao").join("config.toml");
        if global.exists() {
            let content = std::fs::read_to_string(&global)
                .with_context(|| format!("failed to read config file: {}", global.display()))?;
            let config: Config = toml::from_str(&content)
                .with_context(|| format!("failed to parse config file: {}", global.display()))?;
            return Ok((config, Some(global)));
        }
    }

    // No config file found — use defaults
    Ok((Config::default(), None))
}

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Merge CLI flags on top of config file values.
///
/// Precedence: CLI flag > config file > built-in default.
///
/// A CLI flag is considered "set" when:
/// - `--waku` differs from the clap default (`http://localhost:8645`)
/// - `--keyfile` is `Some`
/// - `--encrypt` / `--json` are `true` (boolean flags)
pub fn merge(config: &Config, cli: &crate::cli::Cli) -> ResolvedConfig {
    let waku_default = "http://localhost:8645";

    let waku_url = if cli.waku != waku_default {
        cli.waku.clone()
    } else {
        config
            .waku_url
            .clone()
            .unwrap_or_else(|| waku_default.to_string())
    };

    let keyfile = if cli.keyfile.is_some() {
        cli.keyfile.clone()
    } else {
        config.keyfile.as_ref().map(|s| expand_tilde(s))
    };

    let encrypt = if cli.encrypt {
        true
    } else {
        config.encrypt.unwrap_or(false)
    };

    let json = if cli.json {
        true
    } else {
        config.json.unwrap_or(false)
    };

    ResolvedConfig {
        waku_url,
        keyfile,
        encrypt,
        json,
        agent: config.agent.clone(),
        presence: config.presence.clone(),
    }
}

/// Generate a default config file at the given path.
pub fn init_config(path: &Path) -> Result<()> {
    let default = r#"# LMAO configuration file
# See: https://github.com/jimmy-claw/lmao

waku_url = "http://localhost:8645"
# keyfile = "~/.config/lmao/identity.key"
# encrypt = true
# json = false

[agent]
# name = "my-agent"
# capabilities = ["text", "code", "summarize"]
# description = "My LMAO agent"

[presence]
# ttl = 300
# auto_announce = true
"#;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory: {}", parent.display()))?;
    }
    std::fs::write(path, default)
        .with_context(|| format!("failed to write config file: {}", path.display()))?;
    Ok(())
}

/// Return the default config path (`~/.config/lmao/config.toml`).
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("lmao").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;
    use std::io::Write;

    fn parse_cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    // ── Config parsing ──

    #[test]
    fn parse_full_config() {
        let toml = r#"
waku_url = "http://node:9090"
keyfile = "~/.config/lmao/identity.key"
encrypt = true
json = false

[agent]
name = "my-agent"
capabilities = ["text", "code"]
description = "Test agent"

[presence]
ttl = 600
auto_announce = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.waku_url.as_deref(), Some("http://node:9090"));
        assert_eq!(
            config.keyfile.as_deref(),
            Some("~/.config/lmao/identity.key")
        );
        assert_eq!(config.encrypt, Some(true));
        assert_eq!(config.json, Some(false));
        assert_eq!(config.agent.name.as_deref(), Some("my-agent"));
        assert_eq!(
            config.agent.capabilities,
            Some(vec!["text".to_string(), "code".to_string()])
        );
        assert_eq!(config.agent.description.as_deref(), Some("Test agent"));
        assert_eq!(config.presence.ttl, Some(600));
        assert_eq!(config.presence.auto_announce, Some(true));
    }

    #[test]
    fn parse_partial_config() {
        let toml = r#"
waku_url = "http://node:9090"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.waku_url.as_deref(), Some("http://node:9090"));
        assert!(config.keyfile.is_none());
        assert!(config.encrypt.is_none());
        assert!(config.agent.name.is_none());
        assert!(config.presence.ttl.is_none());
    }

    #[test]
    fn parse_empty_config() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn invalid_toml_returns_error() {
        let result = toml::from_str::<Config>("not valid [[[toml");
        assert!(result.is_err());
    }

    // ── Merge precedence ──

    #[test]
    fn cli_overrides_config_waku() {
        let config = Config {
            waku_url: Some("http://from-config:1234".to_string()),
            ..Default::default()
        };
        let cli = parse_cli(&["cli", "--waku", "http://from-cli:5678", "health"]);
        let resolved = merge(&config, &cli);
        assert_eq!(resolved.waku_url, "http://from-cli:5678");
    }

    #[test]
    fn config_used_when_cli_has_default_waku() {
        let config = Config {
            waku_url: Some("http://from-config:1234".to_string()),
            ..Default::default()
        };
        let cli = parse_cli(&["cli", "health"]);
        let resolved = merge(&config, &cli);
        assert_eq!(resolved.waku_url, "http://from-config:1234");
    }

    #[test]
    fn builtin_default_when_neither_set() {
        let config = Config::default();
        let cli = parse_cli(&["cli", "health"]);
        let resolved = merge(&config, &cli);
        assert_eq!(resolved.waku_url, "http://localhost:8645");
    }

    #[test]
    fn cli_keyfile_overrides_config() {
        let config = Config {
            keyfile: Some("~/.config/lmao/config.key".to_string()),
            ..Default::default()
        };
        let cli = parse_cli(&["cli", "--keyfile", "/tmp/cli.key", "health"]);
        let resolved = merge(&config, &cli);
        assert_eq!(resolved.keyfile, Some(PathBuf::from("/tmp/cli.key")));
    }

    #[test]
    fn config_keyfile_with_tilde_expansion() {
        let config = Config {
            keyfile: Some("~/my.key".to_string()),
            ..Default::default()
        };
        let cli = parse_cli(&["cli", "health"]);
        let resolved = merge(&config, &cli);
        // Should have expanded the tilde
        let path = resolved.keyfile.unwrap();
        assert!(!path.to_str().unwrap().starts_with('~'));
    }

    #[test]
    fn cli_encrypt_overrides_config() {
        let config = Config {
            encrypt: Some(false),
            ..Default::default()
        };
        let cli = parse_cli(&["cli", "--encrypt", "health"]);
        let resolved = merge(&config, &cli);
        assert!(resolved.encrypt);
    }

    #[test]
    fn config_encrypt_used_when_cli_false() {
        let config = Config {
            encrypt: Some(true),
            ..Default::default()
        };
        let cli = parse_cli(&["cli", "health"]);
        let resolved = merge(&config, &cli);
        assert!(resolved.encrypt);
    }

    #[test]
    fn cli_json_overrides_config() {
        let config = Config {
            json: Some(false),
            ..Default::default()
        };
        let cli = parse_cli(&["cli", "--json", "health"]);
        let resolved = merge(&config, &cli);
        assert!(resolved.json);
    }

    #[test]
    fn config_json_used_when_cli_false() {
        let config = Config {
            json: Some(true),
            ..Default::default()
        };
        let cli = parse_cli(&["cli", "health"]);
        let resolved = merge(&config, &cli);
        assert!(resolved.json);
    }

    // ── File loading ──

    #[test]
    fn load_explicit_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.toml");
        std::fs::write(&path, "waku_url = \"http://custom:1111\"\n").unwrap();

        let (config, loaded_from) = load_config(Some(&path)).unwrap();
        assert_eq!(config.waku_url.as_deref(), Some("http://custom:1111"));
        assert_eq!(loaded_from, Some(path));
    }

    #[test]
    fn load_missing_explicit_path_errors() {
        let result = load_config(Some(Path::new("/nonexistent/config.toml")));
        assert!(result.is_err());
    }

    #[test]
    fn load_invalid_toml_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "not [[[valid toml").unwrap();

        let result = load_config(Some(&path));
        assert!(result.is_err());
    }

    #[test]
    fn load_no_config_returns_default() {
        // Use a temp dir as CWD to ensure no lmao.toml exists
        let dir = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new(dir.path());

        let (config, loaded_from) = load_config(None).unwrap();
        assert_eq!(config, Config::default());
        assert!(loaded_from.is_none());
    }

    // ── Config init ──

    #[test]
    fn init_creates_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");

        init_config(&path).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("waku_url"));
        assert!(content.contains("[agent]"));
        assert!(content.contains("[presence]"));

        // The generated file should be valid TOML
        let _config: Config = toml::from_str(&content).unwrap();
    }

    #[test]
    fn init_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "old content").unwrap();

        init_config(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("waku_url"));
    }

    // ── Expand tilde ──

    #[test]
    fn expand_tilde_with_home() {
        let expanded = expand_tilde("~/foo/bar");
        assert!(!expanded.to_str().unwrap().starts_with('~'));
        assert!(expanded.to_str().unwrap().ends_with("foo/bar"));
    }

    #[test]
    fn expand_tilde_absolute_unchanged() {
        let expanded = expand_tilde("/absolute/path");
        assert_eq!(expanded, PathBuf::from("/absolute/path"));
    }

    // ── Config show (serialisation round-trip) ──

    #[test]
    fn config_round_trip() {
        let config = Config {
            waku_url: Some("http://node:9090".to_string()),
            keyfile: Some("/tmp/key".to_string()),
            encrypt: Some(true),
            json: Some(false),
            agent: AgentConfig {
                name: Some("test".to_string()),
                capabilities: Some(vec!["text".to_string()]),
                description: Some("A test agent".to_string()),
            },
            presence: PresenceConfig {
                ttl: Some(300),
                auto_announce: Some(true),
            },
        };
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }

    // Helper to temporarily change CWD for tests.
    struct CwdGuard {
        prev: PathBuf,
    }

    impl CwdGuard {
        fn new(dir: &Path) -> Self {
            let prev = std::env::current_dir().unwrap();
            std::env::set_current_dir(dir).unwrap();
            CwdGuard { prev }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.prev);
        }
    }

    // ── Local file discovery ──

    #[test]
    fn load_discovers_local_lmao_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lmao.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "waku_url = \"http://local:2222\"").unwrap();

        let _guard = CwdGuard::new(dir.path());
        let (config, loaded_from) = load_config(None).unwrap();
        assert_eq!(config.waku_url.as_deref(), Some("http://local:2222"));
        assert_eq!(loaded_from, Some(PathBuf::from("lmao.toml")));
    }
}
