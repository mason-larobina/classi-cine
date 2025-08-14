use std::ops::Deref;
use std::path::Component;
use std::path::{Path, PathBuf};

pub fn normalize_path(path: &Path) -> PathBuf {
    let mut stack = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(Component::Normal(_)) = stack.last() {
                    stack.pop();
                }
            }
            other => stack.push(other),
        }
    }
    let mut normalized = PathBuf::new();
    for component in stack {
        normalized.push(component.as_os_str());
    }
    normalized
}

/// Context-aware path wrapper that enforces correct path display modes
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbsPath(PathBuf);

impl AbsPath {
    pub fn from_abs_path(abs_path: &Path) -> Self {
        assert!(abs_path.is_absolute());
        Self(normalize_path(abs_path))
    }

    pub fn to_string(&self, context: &PathDisplayContext) -> String {
        match context {
            PathDisplayContext::Absolute => self.0.to_string_lossy().to_string(),
            PathDisplayContext::RelativeTo(base) => {
                let display_path =
                    pathdiff::diff_paths(&self.0, base).unwrap_or_else(|| self.0.clone());
                display_path.to_string_lossy().to_string()
            }
        }
    }
}

impl AsRef<Path> for AbsPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Deref for AbsPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Display context determines how paths should be shown
#[derive(Debug, Clone)]
pub enum PathDisplayContext {
    /// Show absolute path
    Absolute,
    /// Show relative to specified base path (e.g., playlist directory for build, CWD for score/list)
    RelativeTo(PathBuf),
}

impl PathDisplayContext {
    /// Create display context for build command (relative to playlist directory)
    pub fn build_context(playlist_root: &Path) -> Self {
        assert!(playlist_root.is_absolute());
        Self::RelativeTo(normalize_path(playlist_root))
    }

    /// Create display context for score/list commands
    pub fn score_list_context(absolute: bool) -> Self {
        if absolute {
            Self::Absolute
        } else {
            let cwd = std::env::current_dir().expect("Unable to get current working dir");
            Self::RelativeTo(cwd)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_normalization() {
        // Non-existing path
        let non_existing = Path::new("/non/existing/../path.txt");
        assert_eq!(normalize_path(non_existing), PathBuf::from("/non/path.txt"));

        // Relative path
        let relative = Path::new("test/../file.txt");
        assert_eq!(normalize_path(relative), PathBuf::from("file.txt"));

        // Complex normalization
        let complex = Path::new("/a/b/../c/./d/../../e");
        assert_eq!(normalize_path(complex), PathBuf::from("/a/e"));

        // Path with trailing slash
        let trailing = Path::new("a/b/");
        assert_eq!(normalize_path(trailing), PathBuf::from("a/b/"));
    }
}
