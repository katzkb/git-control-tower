use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub worktree: WorktreeConfig,
}

#[derive(Debug, Deserialize)]
pub struct WorktreeConfig {
    #[serde(default = "default_worktree_dir")]
    pub dir: String,
}

fn default_worktree_dir() -> String {
    "../gct-wt".to_string()
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            dir: default_worktree_dir(),
        }
    }
}

/// Load config from the first file found:
/// 1. `~/.config/gct/config.toml`
/// 2. `~/.gct.toml`
pub fn load_config() -> Config {
    let candidates = config_paths();
    for path in &candidates {
        match fs::read_to_string(path) {
            Ok(content) => match toml::from_str::<Config>(&content) {
                Ok(config) => return config,
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {e}", path.display());
                    return Config::default();
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                eprintln!("Warning: failed to read {}: {e}", path.display());
                continue;
            }
        }
    }
    Config::default()
}

fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home_dir() {
        paths.push(home.join(".config/gct/config.toml"));
        paths.push(home.join(".gct.toml"));
    }
    paths
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.worktree.dir, "../gct-wt");
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[worktree]
dir = "../wt"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.worktree.dir, "../wt");
    }

    #[test]
    fn test_parse_empty_config() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.worktree.dir, "../gct-wt");
    }

    #[test]
    fn test_config_paths() {
        let paths = config_paths();
        assert!(paths.len() <= 2);
        if let Some(home) = home_dir() {
            assert_eq!(paths[0], home.join(".config/gct/config.toml"));
            assert_eq!(paths[1], home.join(".gct.toml"));
        }
    }
}
