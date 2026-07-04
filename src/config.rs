use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_WORKTREE_DIR: &str = "..";

#[derive(Debug, Deserialize, Clone, Default)]
pub struct WorkspaceConfig {
    /// Optional root path to look up local clones for cross-repo entries.
    /// `~` is expanded at lookup time; absent means cross-repo Worktree
    /// actions degrade to read-only when auto-detection also fails.
    #[serde(default)]
    pub clone_root: Option<String>,
}

impl WorkspaceConfig {
    pub fn clone_root_expanded(&self) -> Option<PathBuf> {
        let raw = self.clone_root.as_deref()?;
        let expanded = shellexpand::tilde(raw);
        Some(PathBuf::from(expanded.into_owned()))
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub worktree: WorktreeConfig,
    #[serde(default = "default_protected_branches")]
    pub protected_branches: Vec<String>,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
}

fn default_protected_branches() -> Vec<String> {
    vec!["main".into(), "master".into(), "develop".into()]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            worktree: WorktreeConfig::default(),
            protected_branches: default_protected_branches(),
            workspace: WorkspaceConfig::default(),
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

#[derive(Debug, Deserialize, Clone)]
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
    /// Resolve the configured base dir, substituting the `{repo}` placeholder.
    /// Trims whitespace; empty (or whitespace-only) falls back to
    /// `DEFAULT_WORKTREE_DIR`. When `{repo}` is absent the string is returned
    /// unchanged, so existing configs behave exactly as before.
    fn resolved_base(&self, repo_name: &str) -> String {
        let dir = self.worktree.dir.trim();
        let base = if dir.is_empty() {
            DEFAULT_WORKTREE_DIR
        } else {
            dir
        };
        base.replace("{repo}", repo_name)
    }

    /// Build the worktree path for a given branch name.
    /// Default produces `../{branch_name}` (e.g. `../feature/auth`).
    /// Custom dir produces `{dir}/{branch_name}`.
    /// The `{repo}` token in `dir` expands to the repository name, so
    /// `dir = "../wt/{repo}"` produces `../wt/{repo}/{branch_name}`.
    /// Slashes in branch names become directory separators.
    pub fn worktree_path(&self, repo_name: &str, branch_name: &str) -> String {
        let base = self.resolved_base(repo_name);
        Path::new(&base)
            .join(branch_name)
            .to_string_lossy()
            .to_string()
    }

    /// Build a worktree path relative to a specific repo root.
    /// `dir = ".."` produces `<repo_root>/../<branch>` (= sibling of repo).
    /// The `{repo}` token in `dir` expands to the repository name, so
    /// `dir = "../wt/{repo}"` produces `<repo_root>/../wt/{repo}/<branch>`.
    pub fn worktree_path_for(
        &self,
        repo_root: &Path,
        repo_name: &str,
        branch_name: &str,
    ) -> String {
        let base = self.resolved_base(repo_name);
        repo_root
            .join(&base)
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

/// Load the effective config for the current directory's repository:
/// global layers overlaid with the repo-local `.gct.toml` (if any).
///
/// Layers are deep-merged in priority order (later overrides earlier):
/// 1. `~/.gct.toml` (global, lowest priority)
/// 2. `~/.config/gct/config.toml` (global)
/// 3. `.gct.toml` at the git repository root (project-local, highest priority)
///
/// Nested tables (`[worktree]`, `[workspace]`) are merged key-by-key, so a
/// project-local file can override individual settings while inheriting the
/// rest from the global config. Scalars and arrays are replaced wholesale,
/// except `worktree.post_create`, which is additive like gitignore: entries
/// from every layer run, lowest priority (global) first.
///
/// Must be called before TUI initialization (eprintln warnings).
pub fn load_config() -> Config {
    resolve_config(&load_global_layers(), git_repo_root().as_deref())
}

/// Read a TOML file and deep-merge it into `merged`. A missing file is ignored;
/// read/parse errors are warned about and skipped.
fn read_and_merge(path: &Path, merged: &mut toml::Table) {
    match fs::read_to_string(path) {
        Ok(content) => match toml::from_str::<toml::Table>(&content) {
            Ok(t) => merge_tables(merged, t),
            Err(e) => eprintln!("Warning: failed to parse {}: {e}", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("Warning: failed to read {}: {e}", path.display()),
    }
}

/// Deep-merge only the global (home-dir) config files into a raw table.
/// Order: `~/.gct.toml` (low) then `~/.config/gct/config.toml` (high).
/// The result is the base onto which any repo-local `.gct.toml` is overlaid.
pub fn load_global_layers() -> toml::Table {
    let mut merged = toml::Table::new();
    if let Some(home) = home_dir() {
        // config_paths_for_home() is ordered high→low; reverse to apply low→high.
        for path in config_paths_for_home(&home).into_iter().rev() {
            read_and_merge(&path, &mut merged);
        }
    }
    merged
}

/// Resolve the effective Config for a specific repository: the global layers
/// overlaid with `<repo_root>/.gct.toml` (if present). `repo_root = None`
/// yields the global-only config. This is what cross-repo worktree operations
/// use so the target repo's own `.gct.toml` applies (the launching repo's
/// project-local config does not leak into other repos).
pub fn resolve_config(global: &toml::Table, repo_root: Option<&Path>) -> Config {
    let mut merged = global.clone();
    if let Some(root) = repo_root {
        read_and_merge(&root.join(".gct.toml"), &mut merged);
    }
    toml::Value::Table(merged).try_into().unwrap_or_else(|e| {
        eprintln!("Warning: invalid merged config: {e}");
        Config::default()
    })
}

/// Key whose array entries accumulate across config layers instead of being
/// replaced (gitignore-style: every layer's hooks apply, global first).
const APPEND_MERGE_KEY: &str = "post_create";

/// Key listing hook names a layer opts out of. Like a gitignore `!` pattern,
/// it removes matching entries accumulated from lower-priority layers.
const DISABLE_KEY: &str = "disable_post_create";

/// Deep-merge `overlay` into `base`. Nested tables are merged key-by-key;
/// scalars and arrays from `overlay` replace those in `base`, except
/// `post_create` arrays, which are concatenated (base layer's entries first)
/// so hooks from every config layer run.
///
/// A layer can opt out of individual lower-layer hooks by naming them:
/// `disable_post_create = ["hook-name"]` removes already-accumulated entries
/// whose `name` matches, before the layer's own hooks are appended. Unnamed
/// hooks cannot be disabled, and a layer cannot disable its own hooks.
fn merge_tables(base: &mut toml::Table, overlay: toml::Table) {
    if let Some(toml::Value::Array(names)) = overlay.get(DISABLE_KEY) {
        let disabled: Vec<&str> = names.iter().filter_map(|n| n.as_str()).collect();
        if let Some(toml::Value::Array(hooks)) = base.get_mut(APPEND_MERGE_KEY) {
            hooks.retain(|hook| {
                hook.as_table()
                    .and_then(|t| t.get("name"))
                    .and_then(|n| n.as_str())
                    .is_none_or(|name| !disabled.contains(&name))
            });
        }
    }
    for (k, v) in overlay {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(bt)), toml::Value::Table(ot)) => merge_tables(bt, ot),
            (Some(toml::Value::Array(ba)), toml::Value::Array(oa)) if k == APPEND_MERGE_KEY => {
                ba.extend(oa);
            }
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

pub fn git_repo_root() -> Option<PathBuf> {
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
        assert_eq!(
            config.worktree_path("name", "feature/auth"),
            "../feature/auth"
        );
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
            config.worktree_path("name", "feature/auth"),
            expected.to_string_lossy()
        );
    }

    #[test]
    fn test_worktree_path_repo_placeholder() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "../wt/{repo}".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let expected = Path::new("../wt/myrepo").join("feature/auth");
        assert_eq!(
            config.worktree_path("myrepo", "feature/auth"),
            expected.to_string_lossy()
        );
    }

    #[test]
    fn test_worktree_path_no_placeholder_ignores_repo_name() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "../wt".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        // Passing any repo name must not change a dir without `{repo}`.
        let expected = Path::new("../wt").join("feature/auth");
        assert_eq!(
            config.worktree_path("anything", "feature/auth"),
            expected.to_string_lossy()
        );
    }

    #[test]
    fn test_worktree_path_empty_dir_with_repo_name() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "  ".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            config.worktree_path("myrepo", "feature/auth"),
            "../feature/auth"
        );
    }

    #[test]
    fn test_worktree_path_placeholder_only() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "{repo}".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let expected = Path::new("myrepo").join("br");
        assert_eq!(
            config.worktree_path("myrepo", "br"),
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
    fn test_empty_dir_falls_back_to_default() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "  ".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            config.worktree_path("name", "feature/auth"),
            "../feature/auth"
        );
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

    #[test]
    fn parse_workspace_clone_root() {
        let toml_str = r#"
[workspace]
clone_root = "~/workspace"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.workspace.clone_root.as_deref(), Some("~/workspace"));
    }

    #[test]
    fn workspace_default_empty() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.workspace.clone_root.is_none());
    }

    #[test]
    fn workspace_clone_root_expanded_strips_tilde() {
        let cfg = WorkspaceConfig {
            clone_root: Some("~/foo".into()),
        };
        let path = cfg.clone_root_expanded().unwrap();
        // No raw "~" should remain; the final component should be "foo".
        assert!(!path.to_string_lossy().contains('~'));
        assert!(path.ends_with("foo"));
    }

    #[test]
    fn worktree_path_for_default_dir() {
        let config = Config::default();
        let root = Path::new("/repos/owner/name");
        let p = config.worktree_path_for(root, "name", "feature/auth");
        let expected = Path::new("/repos/owner/name")
            .join("..")
            .join("feature/auth");
        assert_eq!(p, expected.to_string_lossy().to_string());
    }

    #[test]
    fn worktree_path_for_custom_dir() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "wt".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let root = Path::new("/repos/owner/name");
        let p = config.worktree_path_for(root, "name", "feature/x");
        let expected = Path::new("/repos/owner/name").join("wt").join("feature/x");
        assert_eq!(p, expected.to_string_lossy().to_string());
    }

    #[test]
    fn test_worktree_path_for_repo_placeholder() {
        let config = Config {
            worktree: WorktreeConfig {
                dir: "../wt/{repo}".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let root = Path::new("/repos/owner/name");
        let p = config.worktree_path_for(root, "name", "feature/x");
        let expected = Path::new("/repos/owner/name")
            .join("../wt/name")
            .join("feature/x");
        assert_eq!(p, expected.to_string_lossy().to_string());
    }

    // --- config merge ---

    /// Deep-merge TOML source layers (low→high priority) into a Config,
    /// mirroring `read_and_merge` semantics (invalid layers are skipped).
    fn merge_strs(layers: &[&str]) -> Config {
        let mut merged = toml::Table::new();
        for content in layers {
            if let Ok(t) = toml::from_str::<toml::Table>(content) {
                merge_tables(&mut merged, t);
            }
        }
        toml::Value::Table(merged)
            .try_into()
            .unwrap_or_else(|_| Config::default())
    }

    #[test]
    fn merge_local_overrides_global_scalar() {
        let config = merge_strs(&[
            "[worktree]\ndir = \"..\"\n",
            "[worktree]\ndir = \"../wt\"\n",
        ]);
        assert_eq!(config.worktree.dir, "../wt");
    }

    #[test]
    fn merge_keeps_global_when_local_absent() {
        let config = merge_strs(&[
            "[workspace]\nclone_root = \"~/workspace\"\n\n[worktree]\ndir = \"..\"\n",
            "[worktree]\ndir = \"../wt\"\n",
        ]);
        // local overrides dir, but global's clone_root is preserved.
        assert_eq!(config.worktree.dir, "../wt");
        assert_eq!(config.workspace.clone_root.as_deref(), Some("~/workspace"));
    }

    #[test]
    fn merge_nested_table_preserves_sibling_keys() {
        let config = merge_strs(&[
            "[worktree]\ndir = \"..\"\n\n[[worktree.post_create]]\ntype = \"command\"\ncommand = \"npm ci\"\n",
            "[worktree]\ndir = \"../wt\"\n",
        ]);
        assert_eq!(config.worktree.dir, "../wt");
        // post_create from global survives because local only set `dir`.
        assert_eq!(config.worktree.post_create.len(), 1);
    }

    #[test]
    fn merge_local_array_replaces_global() {
        let config = merge_strs(&[
            "protected_branches = [\"main\"]\n",
            "protected_branches = [\"x\", \"y\"]\n",
        ]);
        assert_eq!(
            config.protected_branches,
            vec!["x".to_string(), "y".to_string()]
        );
    }

    #[test]
    fn merge_post_create_concatenates_global_first() {
        let config = merge_strs(&[
            "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"global-hook\"\n",
            "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"project-hook\"\n",
        ]);
        // Both layers' hooks survive, lowest-priority (global) layer first.
        assert_eq!(config.worktree.post_create.len(), 2);
        assert!(matches!(
            &config.worktree.post_create[0],
            PostCreateAction::Command { command } if command == "global-hook"
        ));
        assert!(matches!(
            &config.worktree.post_create[1],
            PostCreateAction::Command { command } if command == "project-hook"
        ));
    }

    #[test]
    fn merge_post_create_concatenates_across_three_layers() {
        let config = merge_strs(&[
            "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"a\"\n",
            "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"b\"\n",
            "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"c\"\n",
        ]);
        let commands: Vec<&str> = config
            .worktree
            .post_create
            .iter()
            .map(|a| match a {
                PostCreateAction::Command { command } => command.as_str(),
                _ => panic!("expected command action"),
            })
            .collect();
        assert_eq!(commands, vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_post_create_with_name() {
        // `name` is a merge-level identifier; deserialization ignores it.
        let toml_str = r#"
[[worktree.post_create]]
name = "copy-env"
type = "copy"
from = ".env"
to = ".env"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.worktree.post_create.len(), 1);
        assert!(matches!(
            &config.worktree.post_create[0],
            PostCreateAction::Copy { from, .. } if from == ".env"
        ));
    }

    #[test]
    fn parse_config_ignores_disable_key() {
        let toml_str = "[worktree]\ndisable_post_create = [\"copy-env\"]\n";
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.worktree.post_create.is_empty());
    }

    #[test]
    fn merge_disable_removes_named_lower_layer_hook() {
        let config = merge_strs(&[
            concat!(
                "[[worktree.post_create]]\nname = \"copy-env\"\ntype = \"copy\"\nfrom = \".env\"\nto = \".env\"\n",
                "[[worktree.post_create]]\nname = \"install\"\ntype = \"command\"\ncommand = \"npm ci\"\n",
            ),
            concat!(
                "[worktree]\ndisable_post_create = [\"copy-env\"]\n",
                "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"project-hook\"\n",
            ),
        ]);
        // copy-env is negated; the other global hook and the project hook remain.
        assert_eq!(config.worktree.post_create.len(), 2);
        assert!(matches!(
            &config.worktree.post_create[0],
            PostCreateAction::Command { command } if command == "npm ci"
        ));
        assert!(matches!(
            &config.worktree.post_create[1],
            PostCreateAction::Command { command } if command == "project-hook"
        ));
    }

    #[test]
    fn merge_disable_ignores_unnamed_and_unknown() {
        let config = merge_strs(&[
            "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"unnamed\"\n",
            "[worktree]\ndisable_post_create = [\"unnamed\", \"no-such-hook\"]\n",
        ]);
        // Unnamed hooks are never matched by the disable list.
        assert_eq!(config.worktree.post_create.len(), 1);
    }

    #[test]
    fn merge_disable_does_not_affect_own_layer() {
        let config = merge_strs(&[
            "[worktree]\ndir = \"..\"\n",
            concat!(
                "[worktree]\ndisable_post_create = [\"mine\"]\n",
                "[[worktree.post_create]]\nname = \"mine\"\ntype = \"command\"\ncommand = \"x\"\n",
            ),
        ]);
        // The disable list only filters lower layers, not the layer's own hooks.
        assert_eq!(config.worktree.post_create.len(), 1);
    }

    #[test]
    fn merge_later_layer_can_readd_disabled_name() {
        let config = merge_strs(&[
            "[[worktree.post_create]]\nname = \"x\"\ntype = \"command\"\ncommand = \"old\"\n",
            "[worktree]\ndisable_post_create = [\"x\"]\n",
            "[[worktree.post_create]]\nname = \"x\"\ntype = \"command\"\ncommand = \"new\"\n",
        ]);
        // Like gitignore, later layers win: the re-added hook survives.
        assert_eq!(config.worktree.post_create.len(), 1);
        assert!(matches!(
            &config.worktree.post_create[0],
            PostCreateAction::Command { command } if command == "new"
        ));
    }

    #[test]
    fn merge_post_create_single_layer_unchanged() {
        let config = merge_strs(&[
            "[worktree]\ndir = \"..\"\n",
            "[[worktree.post_create]]\ntype = \"copy\"\nfrom = \".env\"\nto = \".env\"\n",
        ]);
        assert_eq!(config.worktree.post_create.len(), 1);
    }

    #[test]
    fn merge_empty_layers_is_default() {
        let config = merge_strs(&[]);
        let default = Config::default();
        assert_eq!(config.worktree.dir, default.worktree.dir);
        assert_eq!(config.protected_branches, default.protected_branches);
        assert!(config.workspace.clone_root.is_none());
    }

    #[test]
    fn merge_single_layer_applies_defaults() {
        let config = merge_strs(&["[worktree]\ndir = \"../wt\"\n"]);
        assert_eq!(config.worktree.dir, "../wt");
        // Unspecified fields fall back to serde defaults.
        assert_eq!(config.protected_branches, default_protected_branches());
    }

    #[test]
    fn merge_skips_invalid_layer() {
        let config = merge_strs(&[
            "this is = not = valid toml\n",
            "[worktree]\ndir = \"../wt\"\n",
        ]);
        assert_eq!(config.worktree.dir, "../wt");
    }

    #[test]
    fn merge_tables_recurses_and_replaces_arrays() {
        let mut base: toml::Table = toml::from_str("[a]\nx = 1\ny = 2\narr = [1, 2]\n").unwrap();
        let overlay: toml::Table = toml::from_str("[a]\ny = 9\narr = [3]\n").unwrap();
        merge_tables(&mut base, overlay);
        let a = base["a"].as_table().unwrap();
        assert_eq!(a["x"].as_integer(), Some(1)); // untouched sibling kept
        assert_eq!(a["y"].as_integer(), Some(9)); // overridden
        // array replaced wholesale, not appended
        assert_eq!(a["arr"].as_array().unwrap().len(), 1);
    }

    // --- resolve_config (per-repo overlay) ---

    #[test]
    fn resolve_config_repo_root_none() {
        let global: toml::Table = toml::from_str("[worktree]\ndir = \"../g\"\n").unwrap();
        let config = resolve_config(&global, None);
        assert_eq!(config.worktree.dir, "../g");
    }

    #[test]
    fn resolve_config_no_local_file() {
        let tmp = tempfile::tempdir().unwrap();
        let global: toml::Table = toml::from_str("[worktree]\ndir = \"../g\"\n").unwrap();
        // No .gct.toml in tmp dir → global values stay.
        let config = resolve_config(&global, Some(tmp.path()));
        assert_eq!(config.worktree.dir, "../g");
    }

    #[test]
    fn resolve_config_overlays_repo_local() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(".gct.toml"),
            "[worktree]\ndir = \"../custom\"\n",
        )
        .unwrap();
        let global: toml::Table = toml::from_str("[worktree]\ndir = \"..\"\n").unwrap();
        let config = resolve_config(&global, Some(tmp.path()));
        assert_eq!(config.worktree.dir, "../custom");
    }

    #[test]
    fn resolve_config_concatenates_post_create() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(".gct.toml"),
            "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"project-hook\"\n",
        )
        .unwrap();
        let global: toml::Table = toml::from_str(
            "[[worktree.post_create]]\ntype = \"command\"\ncommand = \"global-hook\"\n",
        )
        .unwrap();
        let config = resolve_config(&global, Some(tmp.path()));
        assert_eq!(config.worktree.post_create.len(), 2);
        assert!(matches!(
            &config.worktree.post_create[0],
            PostCreateAction::Command { command } if command == "global-hook"
        ));
    }

    #[test]
    fn resolve_config_inherits_global_when_key_absent() {
        let tmp = tempfile::tempdir().unwrap();
        // Repo-local only overrides worktree.dir.
        fs::write(
            tmp.path().join(".gct.toml"),
            "[worktree]\ndir = \"../custom\"\n",
        )
        .unwrap();
        let global: toml::Table = toml::from_str(
            "[workspace]\nclone_root = \"~/workspace\"\n\n[worktree]\ndir = \"..\"\n",
        )
        .unwrap();
        let config = resolve_config(&global, Some(tmp.path()));
        assert_eq!(config.worktree.dir, "../custom");
        // clone_root is inherited from the global layers.
        assert_eq!(config.workspace.clone_root.as_deref(), Some("~/workspace"));
    }
}
