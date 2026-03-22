use serde::Deserialize;
use std::fs;

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

pub fn load_config() -> Config {
    fs::read_to_string(".gct.toml")
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
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
}
