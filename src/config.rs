use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_WORKTREE_PREFIX: &str = "../gct-wt";

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
    DEFAULT_WORKTREE_PREFIX.to_string()
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            dir: default_worktree_dir(),
        }
    }
}

impl Config {
    /// Build the worktree path for a given branch name.
    /// Default produces `../gct-wt-{safe_name}` for backward compatibility.
    /// Custom dir produces `{dir}/{safe_name}`.
    pub fn worktree_path(&self, branch_name: &str) -> String {
        let safe_name = branch_name.replace('/', "-");
        if self.worktree.dir == DEFAULT_WORKTREE_PREFIX {
            // Backward compatible: ../gct-wt-{branch}
            format!("{DEFAULT_WORKTREE_PREFIX}-{safe_name}")
        } else {
            Path::new(&self.worktree.dir)
                .join(&safe_name)
                .to_string_lossy()
                .to_string()
        }
    }
}

/// Load config from the first valid file found:
/// 1. `~/.config/gct/config.toml`
/// 2. `~/.gct.toml`
///
/// Must be called before TUI initialization (eprintln warnings).
pub fn load_config() -> Config {
    let candidates = config_paths();
    for path in &candidates {
        match fs::read_to_string(path) {
            Ok(content) => match toml::from_str::<Config>(&content) {
                Ok(config) => return config,
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {e}", path.display());
                    continue;
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
        assert_eq!(config.worktree.dir, DEFAULT_WORKTREE_PREFIX);
    }

    #[test]
    fn test_default_worktree_path() {
        let config = Config::default();
        assert_eq!(
            config.worktree_path("feature/auth"),
            "../gct-wt-feature-auth"
        );
    }

    #[test]
    fn test_custom_worktree_path() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "../wt".to_string(),
            },
        };
        let expected = Path::new("../wt").join("feature-auth");
        assert_eq!(
            config.worktree_path("feature/auth"),
            expected.to_string_lossy()
        );
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
        assert_eq!(config.worktree.dir, DEFAULT_WORKTREE_PREFIX);
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
