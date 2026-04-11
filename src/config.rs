use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_WORKTREE_DIR: &str = "..";

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub worktree: WorktreeConfig,
    #[serde(default = "default_protected_branches")]
    pub protected_branches: Vec<String>,
}

fn default_protected_branches() -> Vec<String> {
    vec!["main".into(), "master".into(), "develop".into()]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            worktree: WorktreeConfig::default(),
            protected_branches: default_protected_branches(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum PostCreateAction {
    #[serde(rename = "copy")]
    Copy { from: String, to: String },
    #[serde(rename = "symlink")]
    Symlink { from: String, to: String },
    #[serde(rename = "command")]
    Command { command: String },
}

#[derive(Debug, Deserialize)]
pub struct WorktreeConfig {
    #[serde(default = "default_worktree_dir")]
    pub dir: String,
    #[serde(default)]
    pub post_create: Vec<PostCreateAction>,
}

fn default_worktree_dir() -> String {
    DEFAULT_WORKTREE_DIR.to_string()
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            dir: default_worktree_dir(),
            post_create: Vec::new(),
        }
    }
}

impl Config {
    /// Build the worktree path for a given branch name.
    /// Default produces `../{branch_name}` (e.g. `../feature/auth`).
    /// Custom dir produces `{dir}/{branch_name}`.
    /// Slashes in branch names become directory separators.
    pub fn worktree_path(&self, branch_name: &str) -> String {
        let dir = self.worktree.dir.trim();
        let base = if dir.is_empty() {
            DEFAULT_WORKTREE_DIR
        } else {
            dir
        };
        Path::new(base)
            .join(branch_name)
            .to_string_lossy()
            .to_string()
    }
}

/// Run post-create actions after worktree creation.
/// Returns a list of error messages (empty if all succeeded).
pub fn run_post_create(
    actions: &[PostCreateAction],
    repo_root: &Path,
    wt_path: &Path,
) -> Vec<String> {
    let mut errors = Vec::new();
    for action in actions {
        match action {
            PostCreateAction::Copy { from, to } => {
                let src = repo_root.join(from);
                let dst = wt_path.join(to);
                if let Err(e) = copy_path(&src, &dst) {
                    errors.push(format!("copy {} → {}: {e}", from, to));
                }
            }
            PostCreateAction::Symlink { from, to } => {
                let src = repo_root.join(from);
                let dst = wt_path.join(to);
                if let Err(e) = create_symlink(&src, &dst) {
                    errors.push(format!("symlink {} → {}: {e}", from, to));
                }
            }
            PostCreateAction::Command { command } => {
                if let Err(e) = run_command(command, wt_path) {
                    errors.push(format!("command `{command}`: {e}"));
                }
            }
        }
    }
    errors
}

fn copy_path(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = fs::metadata(src)?;
    if meta.is_dir() {
        copy_dir_recursive(src, dst)
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
        Ok(())
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.metadata()?.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn create_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    // Resolve to absolute canonical path so the symlink target is valid from the new worktree
    let abs_src = fs::canonicalize(src)?;
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&abs_src, dst)?;
    }
    #[cfg(windows)]
    {
        if abs_src.is_dir() {
            std::os::windows::fs::symlink_dir(&abs_src, dst)?;
        } else {
            std::os::windows::fs::symlink_file(&abs_src, dst)?;
        }
    }
    Ok(())
}

fn run_command(command: &str, work_dir: &Path) -> std::io::Result<()> {
    let output = std::process::Command::new("sh")
        .args(["-c", command])
        .current_dir(work_dir)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let msg = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("exited with {}", output.status)
        };
        return Err(std::io::Error::other(msg));
    }
    Ok(())
}

/// Load config from the first valid file found:
/// 1. `.gct.toml` (project-local, git repository root via `git rev-parse --show-toplevel`)
/// 2. `~/.config/gct/config.toml` (global)
/// 3. `~/.gct.toml` (global)
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
    if let Some(root) = git_repo_root() {
        paths.push(root.join(".gct.toml"));
    }
    if let Some(home) = home_dir() {
        paths.extend(config_paths_for_home(&home));
    }
    paths
}

fn git_repo_root() -> Option<PathBuf> {
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
}

fn config_paths_for_home(home: &Path) -> Vec<PathBuf> {
    vec![home.join(".config/gct/config.toml"), home.join(".gct.toml")]
}

pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return Some(PathBuf::from(home));
    }
    #[cfg(windows)]
    {
        let drive = std::env::var_os("HOMEDRIVE");
        let path = std::env::var_os("HOMEPATH");
        if let (Some(d), Some(p)) = (drive, path) {
            let mut home = PathBuf::from(d);
            home.push(p);
            return Some(home);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.worktree.dir, DEFAULT_WORKTREE_DIR);
    }

    #[test]
    fn test_default_worktree_path() {
        let config = Config::default();
        assert_eq!(config.worktree_path("feature/auth"), "../feature/auth");
    }

    #[test]
    fn test_custom_worktree_path() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "../wt".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let expected = Path::new("../wt").join("feature/auth");
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
        assert_eq!(config.worktree.dir, DEFAULT_WORKTREE_DIR);
    }

    #[test]
    fn test_config_paths_for_home() {
        let home = Path::new("/tmp/fakehome");
        let paths = config_paths_for_home(home);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], home.join(".config/gct/config.toml"));
        assert_eq!(paths[1], home.join(".gct.toml"));
    }

    #[test]
    fn test_config_paths_includes_local() {
        let paths = config_paths();
        // When run inside a git repo, first entry should be .gct.toml at repo root
        if git_repo_root().is_some() {
            assert!(paths[0].ends_with(".gct.toml"));
        }
        // Global paths should be present if home is available
        if home_dir().is_some() {
            assert!(paths.len() >= 2);
        }
    }

    #[test]
    fn test_empty_dir_falls_back_to_default() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "  ".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.worktree_path("feature/auth"), "../feature/auth");
    }

    #[test]
    fn test_default_protected_branches() {
        let config = Config::default();
        assert_eq!(
            config.protected_branches,
            vec![
                "main".to_string(),
                "master".to_string(),
                "develop".to_string()
            ]
        );
    }

    #[test]
    fn test_parse_protected_branches() {
        let toml_str = r#"
protected_branches = ["main", "develop", "staging"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.protected_branches,
            vec![
                "main".to_string(),
                "develop".to_string(),
                "staging".to_string()
            ]
        );
    }

    #[test]
    fn test_parse_config_uses_default_protected() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(
            config.protected_branches,
            vec![
                "main".to_string(),
                "master".to_string(),
                "develop".to_string()
            ]
        );
    }

    #[test]
    fn test_parse_post_create() {
        let toml_str = r#"
[worktree]
dir = ".."

[[worktree.post_create]]
type = "copy"
from = ".env"
to = ".env"

[[worktree.post_create]]
type = "symlink"
from = ".bin"
to = ".bin"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.worktree.post_create.len(), 2);
    }

    #[test]
    fn test_parse_no_post_create() {
        let toml_str = r#"
[worktree]
dir = ".."
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.worktree.post_create.is_empty());
    }

    #[test]
    fn test_run_post_create_copy_file() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let wt = tmp.path().join("wt");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::write(repo.join(".env"), "SECRET=123").unwrap();

        let actions = vec![PostCreateAction::Copy {
            from: ".env".to_string(),
            to: ".env".to_string(),
        }];
        let errors = run_post_create(&actions, &repo, &wt);
        assert!(errors.is_empty());
        assert_eq!(fs::read_to_string(wt.join(".env")).unwrap(), "SECRET=123");
    }

    #[test]
    fn test_run_post_create_copy_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let wt = tmp.path().join("wt");
        fs::create_dir_all(repo.join(".idea")).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::write(repo.join(".idea/workspace.xml"), "<xml/>").unwrap();

        let actions = vec![PostCreateAction::Copy {
            from: ".idea".to_string(),
            to: ".idea".to_string(),
        }];
        let errors = run_post_create(&actions, &repo, &wt);
        assert!(errors.is_empty());
        assert_eq!(
            fs::read_to_string(wt.join(".idea/workspace.xml")).unwrap(),
            "<xml/>"
        );
    }

    #[test]
    fn test_run_post_create_missing_source() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let wt = tmp.path().join("wt");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&wt).unwrap();

        let actions = vec![PostCreateAction::Copy {
            from: ".env".to_string(),
            to: ".env".to_string(),
        }];
        let errors = run_post_create(&actions, &repo, &wt);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains(".env"));
    }

    #[test]
    fn test_parse_symlink_action() {
        let toml_str = r#"
[[worktree.post_create]]
type = "symlink"
from = ".bin"
to = ".bin"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.worktree.post_create.len(), 1);
        assert!(matches!(
            &config.worktree.post_create[0],
            PostCreateAction::Symlink { from, to } if from == ".bin" && to == ".bin"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_run_post_create_symlink_file() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let wt = tmp.path().join("wt");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::write(repo.join(".env"), "SECRET=123").unwrap();

        let actions = vec![PostCreateAction::Symlink {
            from: ".env".to_string(),
            to: ".env".to_string(),
        }];
        let errors = run_post_create(&actions, &repo, &wt);
        assert!(errors.is_empty());
        let link = wt.join(".env");
        assert!(link.is_symlink());
        assert_eq!(fs::read_to_string(&link).unwrap(), "SECRET=123");
    }

    #[cfg(unix)]
    #[test]
    fn test_run_post_create_symlink_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let wt = tmp.path().join("wt");
        fs::create_dir_all(repo.join(".bin")).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::write(repo.join(".bin/tool"), "#!/bin/sh").unwrap();

        let actions = vec![PostCreateAction::Symlink {
            from: ".bin".to_string(),
            to: ".bin".to_string(),
        }];
        let errors = run_post_create(&actions, &repo, &wt);
        assert!(errors.is_empty());
        let link = wt.join(".bin");
        assert!(link.is_symlink());
        assert_eq!(fs::read_to_string(link.join("tool")).unwrap(), "#!/bin/sh");
    }

    #[test]
    fn test_parse_command_action() {
        let toml_str = r#"
[[worktree.post_create]]
type = "command"
command = "npm ci"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.worktree.post_create.len(), 1);
        assert!(matches!(
            &config.worktree.post_create[0],
            PostCreateAction::Command { command } if command == "npm ci"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_run_post_create_command_success() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let wt = tmp.path().join("wt");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&wt).unwrap();

        let actions = vec![PostCreateAction::Command {
            command: "echo hello > test.txt".to_string(),
        }];
        let errors = run_post_create(&actions, &repo, &wt);
        assert!(errors.is_empty());
        assert_eq!(
            fs::read_to_string(wt.join("test.txt")).unwrap().trim(),
            "hello"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_run_post_create_command_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let wt = tmp.path().join("wt");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&wt).unwrap();

        let actions = vec![PostCreateAction::Command {
            command: "exit 1".to_string(),
        }];
        let errors = run_post_create(&actions, &repo, &wt);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("exit 1"));
    }
}
