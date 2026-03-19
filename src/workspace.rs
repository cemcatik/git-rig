use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MANIFEST: &str = ".ws.json";

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
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write {}", path.display()))
    }

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
        self.repos.iter().any(|r| r.name == name)
    }
}

// ---------------------------------------------------------------------------
// Workspace resolution
// ---------------------------------------------------------------------------

/// Walk up from `start` looking for a `.ws.json` file.
fn find_ws_root(start: &Path) -> Option<PathBuf> {
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
/// - `None`: walk up from CWD to find the nearest `.ws.json`.
pub fn resolve_workspace(name: Option<&str>) -> Result<(PathBuf, Manifest)> {
    let cwd = std::env::current_dir()?;

    match name {
        Some(name) => {
            // 1. Try CWD/<name>
            let candidate = cwd.join(name);
            if candidate.join(MANIFEST).exists() {
                let manifest = Manifest::load(&candidate)?;
                return Ok((candidate, manifest));
            }

            // 2. If CWD is inside a workspace, try sibling: <ws_root>/../<name>
            if let Some(ws_root) = find_ws_root(&cwd)
                && let Some(parent) = ws_root.parent()
            {
                let candidate = parent.join(name);
                if candidate.join(MANIFEST).exists() {
                    let manifest = Manifest::load(&candidate)?;
                    return Ok((candidate, manifest));
                }
            }

            Err(anyhow!("workspace '{}' not found", name))
        }
        None => {
            // Walk up from CWD
            if let Some(ws_root) = find_ws_root(&cwd) {
                let manifest = Manifest::load(&ws_root)?;
                Ok((ws_root, manifest))
            } else {
                Err(anyhow!(
                    "not inside a workspace (no {} found in any parent directory)",
                    MANIFEST
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

    if let Some(ws_root) = find_ws_root(&cwd)
        && let Some(parent) = ws_root.parent()
    {
        return Ok(parent.to_path_buf());
    }

    Ok(cwd)
}

/// Find all workspaces (directories containing `.ws.json`) in `base_dir`.
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
