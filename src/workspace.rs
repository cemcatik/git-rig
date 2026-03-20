use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const MANIFEST: &str = ".rig.json";

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    /// Deprecated — retained for migration from old manifests.
    #[serde(default, skip_serializing)]
    base_dir: Option<PathBuf>,
    pub repos: Vec<RepoEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    /// Absolute path to the local git clone this worktree is based on.
    #[serde(default)]
    pub source: PathBuf,
    pub branch: String,
    pub default_branch: String,
    /// Remote to fetch from (default: "origin")
    #[serde(default = "default_remote")]
    pub remote: String,
}

fn default_remote() -> String {
    "origin".to_string()
}

impl Manifest {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            base_dir: None,
            repos: Vec::new(),
        }
    }

    pub fn load(workspace_dir: &Path) -> Result<Self> {
        let path = workspace_dir.join(MANIFEST);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut manifest: Self = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;

        // Migrate old manifests: derive source from base_dir + repo name
        if let Some(ref base_dir) = manifest.base_dir {
            for repo in &mut manifest.repos {
                if repo.source.as_os_str().is_empty() {
                    repo.source = base_dir.join(&repo.name);
                }
            }
            manifest.base_dir = None;
            // Write back migrated manifest
            manifest.save(workspace_dir)?;
        }

        Ok(manifest)
    }

    pub fn save(&self, workspace_dir: &Path) -> Result<()> {
        let path = workspace_dir.join(MANIFEST);
        let tmp_path = workspace_dir.join(format!("{MANIFEST}.tmp"));
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp_path, content)
            .with_context(|| format!("failed to write {}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, &path)
            .with_context(|| format!("failed to rename {} to {}", tmp_path.display(), path.display()))
    }

    #[allow(clippy::unused_self)]
    pub fn worktree_dir(&self, ws_dir: &Path, repo_name: &str) -> PathBuf {
        ws_dir.join(repo_name)
    }

    pub fn add_repo(&mut self, entry: RepoEntry) {
        self.repos.push(entry);
    }

    pub fn remove_repo(&mut self, name: &str) -> Option<RepoEntry> {
        if let Some(pos) = self.repos.iter().position(|r| r.name == name) {
            Some(self.repos.remove(pos))
        } else {
            None
        }
    }

    pub fn find_repo(&self, name: &str) -> Option<&RepoEntry> {
        self.repos.iter().find(|r| r.name == name)
    }

    pub fn has_repo(&self, name: &str) -> bool {
        self.find_repo(name).is_some()
    }
}

// ---------------------------------------------------------------------------
// Workspace resolution
// ---------------------------------------------------------------------------

/// Walk up from `start` looking for a `.rig.json` file.
pub(crate) fn find_ws_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(MANIFEST).exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Resolve a workspace directory and its manifest.
///
/// - `Some(name)`: look for `<cwd>/<name>` or `<base_dir>/<name>` (if CWD is a workspace).
/// - `None`: walk up from CWD to find the nearest `.rig.json`.
pub fn resolve_workspace(name: Option<&str>) -> Result<(PathBuf, Manifest)> {
    let cwd = std::env::current_dir()?;
    resolve_workspace_from(&cwd, name)
}

/// Like [`resolve_workspace`], but starts from `start_dir` instead of CWD.
pub fn resolve_workspace_from(start_dir: &Path, name: Option<&str>) -> Result<(PathBuf, Manifest)> {
    match name {
        Some(name) => {
            // 1. Try start_dir/<name>
            let candidate = start_dir.join(name);
            if candidate.join(MANIFEST).exists() {
                let manifest = Manifest::load(&candidate)?;
                return Ok((candidate, manifest));
            }

            // 2. If start_dir is inside a workspace, try sibling: <ws_root>/../<name>
            if let Some(ws_root) = find_ws_root(start_dir)
                && let Some(parent) = ws_root.parent()
            {
                let candidate = parent.join(name);
                if candidate.join(MANIFEST).exists() {
                    let manifest = Manifest::load(&candidate)?;
                    return Ok((candidate, manifest));
                }
            }

            Err(anyhow!("rig '{name}' not found"))
        }
        None => {
            // Walk up from start_dir
            if let Some(ws_root) = find_ws_root(start_dir) {
                let manifest = Manifest::load(&ws_root)?;
                Ok((ws_root, manifest))
            } else {
                Err(anyhow!(
                    "not inside a rig (no {MANIFEST} found in any parent directory)"
                ))
            }
        }
    }
}

/// Determine the base directory for listing workspaces.
///
/// If CWD is inside a workspace, returns the workspace dir's parent.
/// Otherwise returns CWD.
pub fn resolve_base_dir() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    Ok(resolve_base_dir_from(&cwd))
}

/// Like [`resolve_base_dir`], but starts from `start_dir` instead of CWD.
pub fn resolve_base_dir_from(start_dir: &Path) -> PathBuf {
    if let Some(ws_root) = find_ws_root(start_dir)
        && let Some(parent) = ws_root.parent()
    {
        return parent.to_path_buf();
    }

    start_dir.to_path_buf()
}

/// Find all workspaces (directories containing `.rig.json`) in `base_dir`.
pub fn find_workspaces(base_dir: &Path) -> Result<Vec<Manifest>> {
    let mut workspaces = Vec::new();
    for entry in std::fs::read_dir(base_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir()
            && path.join(MANIFEST).exists()
            && let Ok(manifest) = Manifest::load(&path)
        {
            workspaces.push(manifest);
        }
    }
    workspaces.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(workspaces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_repo_entry(name: &str) -> RepoEntry {
        RepoEntry {
            name: name.to_string(),
            source: PathBuf::from("/some/path"),
            branch: "rig/test".to_string(),
            default_branch: "main".to_string(),
            remote: "origin".to_string(),
        }
    }

    fn create_ws(dir: &Path, name: &str) {
        let m = Manifest::new(name);
        m.save(dir).unwrap();
    }

    // -----------------------------------------------------------------------
    // Manifest data operations
    // -----------------------------------------------------------------------

    #[test]
    fn manifest_new_defaults() {
        let m = Manifest::new("my-ws");
        assert_eq!(m.name, "my-ws");
        assert!(m.repos.is_empty());
    }

    #[test]
    fn manifest_add_repo() {
        let mut m = Manifest::new("ws");
        m.add_repo(make_repo_entry("repo-a"));
        assert_eq!(m.repos.len(), 1);
        assert_eq!(m.repos[0].name, "repo-a");
    }

    #[test]
    fn manifest_remove_repo_found_preserves_order() {
        let mut m = Manifest::new("ws");
        m.add_repo(make_repo_entry("repo-a"));
        m.add_repo(make_repo_entry("repo-b"));
        m.add_repo(make_repo_entry("repo-c"));
        let removed = m.remove_repo("repo-b");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "repo-b");
        assert_eq!(m.repos[0].name, "repo-a");
        assert_eq!(m.repos[1].name, "repo-c");
    }

    #[test]
    fn manifest_remove_repo_not_found() {
        let mut m = Manifest::new("ws");
        m.add_repo(make_repo_entry("repo-a"));
        let removed = m.remove_repo("repo-z");
        assert!(removed.is_none());
        assert_eq!(m.repos.len(), 1);
    }

    #[test]
    fn manifest_find_repo_found() {
        let mut m = Manifest::new("ws");
        m.add_repo(make_repo_entry("repo-a"));
        let found = m.find_repo("repo-a");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "repo-a");
    }

    #[test]
    fn manifest_find_repo_not_found() {
        let m = Manifest::new("ws");
        assert!(m.find_repo("repo-a").is_none());
    }

    #[test]
    fn manifest_has_repo_true() {
        let mut m = Manifest::new("ws");
        m.add_repo(make_repo_entry("repo-a"));
        assert!(m.has_repo("repo-a"));
    }

    #[test]
    fn manifest_has_repo_false() {
        let m = Manifest::new("ws");
        assert!(!m.has_repo("repo-a"));
    }

    #[test]
    fn manifest_worktree_dir() {
        let m = Manifest::new("ws");
        let ws_dir = Path::new("/base/my-ws");
        assert_eq!(m.worktree_dir(ws_dir, "repo-a"), PathBuf::from("/base/my-ws/repo-a"));
    }

    #[test]
    fn manifest_save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();

        let mut m = Manifest::new("my-ws");
        m.add_repo(RepoEntry {
            name: "repo-a".to_string(),
            source: PathBuf::from("/src/repo-a"),
            branch: "rig/my-ws".to_string(),
            default_branch: "main".to_string(),
            remote: "origin".to_string(),
        });
        m.save(&ws_dir).unwrap();

        let loaded = Manifest::load(&ws_dir).unwrap();
        assert_eq!(loaded.name, "my-ws");
        assert_eq!(loaded.repos.len(), 1);
        let r = &loaded.repos[0];
        assert_eq!(r.name, "repo-a");
        assert_eq!(r.source, PathBuf::from("/src/repo-a"));
        assert_eq!(r.branch, "rig/my-ws");
        assert_eq!(r.default_branch, "main");
        assert_eq!(r.remote, "origin");
    }

    #[test]
    fn manifest_load_missing_file() {
        let tmp = TempDir::new().unwrap();
        assert!(Manifest::load(tmp.path()).is_err());
    }

    #[test]
    fn manifest_load_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();
        std::fs::write(ws_dir.join(".rig.json"), "not valid json {{").unwrap();
        assert!(Manifest::load(&ws_dir).is_err());
    }

    #[test]
    fn manifest_migration_base_dir() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();
        let base_dir = "/some/base";

        // Hand-craft old-format JSON with base_dir and empty source
        let old_json = serde_json::json!({
            "name": "my-ws",
            "base_dir": base_dir,
            "repos": [
                {
                    "name": "repo-a",
                    "source": "",
                    "branch": "rig/my-ws",
                    "default_branch": "main"
                }
            ]
        });
        std::fs::write(
            ws_dir.join(".rig.json"),
            serde_json::to_string_pretty(&old_json).unwrap(),
        )
        .unwrap();

        let loaded = Manifest::load(&ws_dir).unwrap();
        // source should be migrated from base_dir + repo name
        assert_eq!(loaded.repos[0].source, PathBuf::from(base_dir).join("repo-a"));

        // Written-back manifest must not contain base_dir
        let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
        assert!(!raw.contains("base_dir"));
    }

    #[test]
    fn serde_remote_defaults_to_origin_when_missing() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();

        let json = serde_json::json!({
            "name": "my-ws",
            "repos": [{
                "name": "repo-a",
                "source": "/src/repo-a",
                "branch": "rig/my-ws",
                "default_branch": "main"
                // no "remote" field
            }]
        });
        std::fs::write(ws_dir.join(".rig.json"), serde_json::to_string_pretty(&json).unwrap()).unwrap();

        let loaded = Manifest::load(&ws_dir).unwrap();
        assert_eq!(loaded.repos[0].remote, "origin");
    }

    #[test]
    fn serde_custom_remote_preserved() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();

        let mut m = Manifest::new("my-ws");
        m.add_repo(RepoEntry {
            name: "repo-a".to_string(),
            source: PathBuf::from("/src/repo-a"),
            branch: "rig/my-ws".to_string(),
            default_branch: "main".to_string(),
            remote: "upstream".to_string(),
        });
        m.save(&ws_dir).unwrap();

        let loaded = Manifest::load(&ws_dir).unwrap();
        assert_eq!(loaded.repos[0].remote, "upstream");
    }

    // -----------------------------------------------------------------------
    // Workspace resolution
    // -----------------------------------------------------------------------

    #[test]
    fn find_ws_root_at_root() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();
        create_ws(&ws_dir, "my-ws");

        assert_eq!(find_ws_root(&ws_dir), Some(ws_dir));
    }

    #[test]
    fn find_ws_root_from_child() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();
        create_ws(&ws_dir, "my-ws");

        let child = ws_dir.join("repo-a");
        std::fs::create_dir_all(&child).unwrap();

        assert_eq!(find_ws_root(&child), Some(ws_dir));
    }

    #[test]
    fn find_ws_root_from_deeply_nested() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();
        create_ws(&ws_dir, "my-ws");

        let nested = ws_dir.join("repo-a").join("src").join("lib");
        std::fs::create_dir_all(&nested).unwrap();

        assert_eq!(find_ws_root(&nested), Some(ws_dir));
    }

    #[test]
    fn find_ws_root_not_found() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        assert!(find_ws_root(&dir).is_none());
    }

    #[test]
    fn find_ws_root_stops_at_nearest() {
        let tmp = TempDir::new().unwrap();
        let outer = tmp.path().canonicalize().unwrap();
        create_ws(&outer, "outer-ws");

        let inner = outer.join("inner-ws");
        std::fs::create_dir_all(&inner).unwrap();
        create_ws(&inner, "inner-ws");

        // Should return the inner, not the outer
        assert_eq!(find_ws_root(&inner), Some(inner));
    }

    #[test]
    fn resolve_workspace_from_none_inside_workspace() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();
        create_ws(&ws_dir, "my-ws");

        let (path, manifest) = resolve_workspace_from(&ws_dir, None).unwrap();
        assert_eq!(path, ws_dir);
        assert_eq!(manifest.name, "my-ws");
    }

    #[test]
    fn resolve_workspace_from_none_inside_repo_worktree() {
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().canonicalize().unwrap();
        create_ws(&ws_dir, "my-ws");

        let repo_dir = ws_dir.join("repo-a");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let (path, manifest) = resolve_workspace_from(&repo_dir, None).unwrap();
        assert_eq!(path, ws_dir);
        assert_eq!(manifest.name, "my-ws");
    }

    #[test]
    fn resolve_workspace_from_none_outside_workspace() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        assert!(resolve_workspace_from(&dir, None).is_err());
    }

    #[test]
    fn resolve_workspace_from_some_child_workspace() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let ws_dir = base.join("my-ws");
        std::fs::create_dir_all(&ws_dir).unwrap();
        create_ws(&ws_dir, "my-ws");

        let (path, manifest) = resolve_workspace_from(&base, Some("my-ws")).unwrap();
        assert_eq!(path, ws_dir);
        assert_eq!(manifest.name, "my-ws");
    }

    #[test]
    fn resolve_workspace_from_some_sibling_workspace() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let ws_a = base.join("ws-a");
        std::fs::create_dir_all(&ws_a).unwrap();
        create_ws(&ws_a, "ws-a");

        let ws_b = base.join("ws-b");
        std::fs::create_dir_all(&ws_b).unwrap();
        create_ws(&ws_b, "ws-b");

        // Start from inside ws-a, look for sibling ws-b
        let (path, manifest) = resolve_workspace_from(&ws_a, Some("ws-b")).unwrap();
        assert_eq!(path, ws_b);
        assert_eq!(manifest.name, "ws-b");
    }

    #[test]
    fn resolve_workspace_from_some_not_found() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        assert!(resolve_workspace_from(&dir, Some("nonexistent")).is_err());
    }

    #[test]
    fn resolve_base_dir_from_outside_ws() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        assert_eq!(resolve_base_dir_from(&dir), dir);
    }

    #[test]
    fn resolve_base_dir_from_inside_ws() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let ws_dir = base.join("my-ws");
        std::fs::create_dir_all(&ws_dir).unwrap();
        create_ws(&ws_dir, "my-ws");

        assert_eq!(resolve_base_dir_from(&ws_dir), base);
    }

    #[test]
    fn find_workspaces_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        let workspaces = find_workspaces(&dir).unwrap();
        assert!(workspaces.is_empty());
    }

    #[test]
    fn find_workspaces_multiple_sorted() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        for name in &["ws-charlie", "ws-alpha", "ws-bravo"] {
            let ws_dir = base.join(name);
            std::fs::create_dir_all(&ws_dir).unwrap();
            create_ws(&ws_dir, name);
        }

        let workspaces = find_workspaces(&base).unwrap();
        assert_eq!(workspaces.len(), 3);
        assert_eq!(workspaces[0].name, "ws-alpha");
        assert_eq!(workspaces[1].name, "ws-bravo");
        assert_eq!(workspaces[2].name, "ws-charlie");
    }

    #[test]
    fn find_workspaces_ignores_non_workspace_dirs() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let ws_dir = base.join("my-ws");
        std::fs::create_dir_all(&ws_dir).unwrap();
        create_ws(&ws_dir, "my-ws");

        std::fs::create_dir_all(base.join("plain-dir")).unwrap();

        let workspaces = find_workspaces(&base).unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].name, "my-ws");
    }
}
