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
                } else if !matches!(stack.first(), Some(Component::RootDir)) {
                    // Only add .. if we're not at root and can't pop a normal component
                    stack.push(component);
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
    pub fn build_context(playlist_root: &AbsPath) -> Self {
        Self::RelativeTo(playlist_root.0.clone())
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
    use std::env;

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

    #[test]
    fn path_normalization_edge_cases() {
        // Parent directory at root level
        let root_parent = Path::new("/../");
        assert_eq!(normalize_path(root_parent), PathBuf::from("/"));

        // Multiple parent directories at root
        let multiple_root_parent = Path::new("/../../");
        assert_eq!(normalize_path(multiple_root_parent), PathBuf::from("/"));

        // Parent directory with insufficient components
        let insufficient_parent = Path::new("a/../../b");
        assert_eq!(normalize_path(insufficient_parent), PathBuf::from("../b"));

        // Multiple consecutive parent directories
        let multiple_parent = Path::new("../../../file.txt");
        assert_eq!(normalize_path(multiple_parent), PathBuf::from("../../../file.txt"));

        // Double slash (empty components)
        let double_slash = Path::new("a//b");
        assert_eq!(normalize_path(double_slash), PathBuf::from("a/b"));

        // Root path variations
        let root = Path::new("/");
        assert_eq!(normalize_path(root), PathBuf::from("/"));

        let root_current = Path::new("/.");
        assert_eq!(normalize_path(root_current), PathBuf::from("/"));

        let root_current_slash = Path::new("/./");
        assert_eq!(normalize_path(root_current_slash), PathBuf::from("/"));

        // Current directory only
        let current_only = Path::new(".");
        assert_eq!(normalize_path(current_only), PathBuf::from(""));

        let current_slash = Path::new("./");
        assert_eq!(normalize_path(current_slash), PathBuf::from(""));

        // Mixed current and parent at root
        let mixed_root = Path::new("/./../");
        assert_eq!(normalize_path(mixed_root), PathBuf::from("/"));

        // Complex mixing with insufficient components
        let complex_insufficient = Path::new("./../../.././a/b/../c");
        assert_eq!(normalize_path(complex_insufficient), PathBuf::from("../../../a/c"));

        // Empty path after normalization
        let cancel_out = Path::new("a/../");
        assert_eq!(normalize_path(cancel_out), PathBuf::from(""));

        // Parent directories that can't be resolved
        let unresolvable = Path::new("../../..");
        assert_eq!(normalize_path(unresolvable), PathBuf::from("../../.."));

        // Mix of current and normal directories
        let mixed_current = Path::new("./a/./b/./c");
        assert_eq!(normalize_path(mixed_current), PathBuf::from("a/b/c"));

        // Root with file after parent navigation
        let root_file_parent = Path::new("/a/../b");
        assert_eq!(normalize_path(root_file_parent), PathBuf::from("/b"));
    }

    #[test]
    #[should_panic]
    fn abspath_rejects_relative_path() {
        let rel_path = Path::new("relative/path.txt");
        AbsPath::from_abs_path(rel_path);
    }

    #[test]
    fn abspath_normalization() {
        let unnormalized = Path::new("/home/user/../user/./file.txt");
        let abspath = AbsPath::from_abs_path(unnormalized);
        assert_eq!(abspath.as_ref(), Path::new("/home/user/file.txt"));
    }

    #[test]
    fn abspath_absolute_display() {
        let abs_path = Path::new("/home/user/documents/file.txt");
        let abspath = AbsPath::from_abs_path(abs_path);
        let context = PathDisplayContext::Absolute;
        assert_eq!(abspath.to_string(&context), "/home/user/documents/file.txt");
    }

    #[test]
    fn abspath_relative_display() {
        let abs_path = Path::new("/home/user/documents/file.txt");
        let abspath = AbsPath::from_abs_path(abs_path);
        let base = PathBuf::from("/home/user");
        let context = PathDisplayContext::RelativeTo(base);
        assert_eq!(abspath.to_string(&context), "documents/file.txt");
    }

    #[test]
    fn abspath_relative_display_parent() {
        let abs_path = Path::new("/home/user/file.txt");
        let abspath = AbsPath::from_abs_path(abs_path);
        let base = PathBuf::from("/home/user/documents");
        let context = PathDisplayContext::RelativeTo(base);
        assert_eq!(abspath.to_string(&context), "../file.txt");
    }

    #[test]
    fn abspath_relative_display_no_common_path() {
        let abs_path = Path::new("/var/log/file.txt");
        let abspath = AbsPath::from_abs_path(abs_path);
        let base = PathBuf::from("/home/user");
        let context = PathDisplayContext::RelativeTo(base);
        assert_eq!(abspath.to_string(&context), "../../var/log/file.txt");
    }

    #[test]
    fn path_display_context_build() {
        let playlist_root = AbsPath::from_abs_path(Path::new("/home/user/playlists"));
        let context = PathDisplayContext::build_context(&playlist_root);
        match context {
            PathDisplayContext::RelativeTo(base) => {
                assert_eq!(base, PathBuf::from("/home/user/playlists"));
            }
            _ => panic!("Expected RelativeTo context"),
        }
    }

    #[test]
    fn path_display_context_score_list_absolute() {
        let context = PathDisplayContext::score_list_context(true);
        match context {
            PathDisplayContext::Absolute => {}
            _ => panic!("Expected Absolute context"),
        }
    }

    #[test]
    fn path_display_context_score_list_relative() {
        let context = PathDisplayContext::score_list_context(false);
        let current_dir = env::current_dir().expect("Unable to get current dir");
        match context {
            PathDisplayContext::RelativeTo(base) => {
                assert_eq!(base, current_dir);
            }
            _ => panic!("Expected RelativeTo context"),
        }
    }
}
