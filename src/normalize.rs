use std::path::{Component, MAIN_SEPARATOR, Path, PathBuf};

pub fn canonicalize_path(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        canon
    } else {
        let abs_path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        normalize_abs_path(&abs_path)
    }
}

/// Normalize an absolute path by resolving . and .. components
fn normalize_abs_path(path: &Path) -> PathBuf {
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

/// Normalizes a file path by performing the following transformations:
/// 1. Converts to lowercase
/// 2. Replaces non-alphanumeric characters (except path separators) with spaces
/// 3. Collapses consecutive special characters into single spaces
/// 4. Removes apostrophes and trailing spaces
/// 5. Maintains single path separators while cleaning surrounding spaces
/// Convert path to canonical form, falling back to absolute + normalized if path doesn't exist
pub fn normalize(file: &Path) -> String {
    let file = file.to_string_lossy().to_lowercase();
    let mut ret = String::new();
    for c in file.chars() {
        if c == MAIN_SEPARATOR {
            if ret.ends_with(" ") || ret.ends_with(MAIN_SEPARATOR) {
                ret.pop();
            }
            ret.push(c);
        } else if c.is_alphanumeric() {
            ret.push(c);
        } else if c != '\''
            && !ret.is_empty()
            && !ret.ends_with(" ")
            && !ret.ends_with(MAIN_SEPARATOR)
        {
            ret.push(' ');
        }
    }
    if ret.ends_with(" ") {
        ret.pop();
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_canonicalization() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("test.txt");
        std::fs::write(&file, "").unwrap();

        // Existing path
        let existing = canonicalize_path(&file);
        assert_eq!(existing, file.canonicalize().unwrap());

        // Non-existing path
        let non_existing = Path::new("/non/existing/../path.txt");
        assert_eq!(
            canonicalize_path(non_existing),
            PathBuf::from("/non/path.txt")
        );

        // Relative path
        let relative = Path::new("test/../file.txt");
        let expected = std::env::current_dir().unwrap().join("file.txt");
        assert_eq!(canonicalize_path(relative), expected);

        // Complex normalization
        let complex = Path::new("/a/b/../c/./d/../../e");
        assert_eq!(canonicalize_path(complex), PathBuf::from("/a/e"));
    }

    #[test]
    fn special_characters() {
        assert_eq!(
            normalize(Path::new("/path/to/special@file!.mp4")),
            "/path/to/special file mp4"
        );
    }

    #[test]
    fn unicode_characters() {
        assert_eq!(
            normalize(Path::new("/path/to/ünîcødé.mp4")),
            "/path/to/ünîcødé mp4"
        );
    }

    #[test]
    fn empty_and_minimal_paths() {
        assert_eq!(normalize(Path::new("")), "");
        assert_eq!(normalize(Path::new("a")), "a");
    }

    #[test]
    fn consecutive_special_characters() {
        assert_eq!(
            normalize(Path::new("/path//to///file.mp4")),
            "/path/to/file mp4"
        );
        assert_eq!(
            normalize(Path::new("/path/to/special!!!file.mp4")),
            "/path/to/special file mp4"
        );
    }

    #[test]
    fn very_long_paths() {
        let long_path = "/path".repeat(100) + "file.mp4";
        let expected = "/path".repeat(100) + "file mp4";
        assert_eq!(normalize(Path::new(&long_path)), expected);
    }

    #[test]
    fn paths_with_only_special_characters() {
        assert_eq!(normalize(Path::new("***")), "");
        assert_eq!(normalize(Path::new("///")), "/");
    }

    #[test]
    fn directory_names_with_trailing_special_characters() {
        assert_eq!(
            normalize(Path::new("/path/to/special_folder!!!/file.mp4")),
            "/path/to/special folder/file mp4"
        );
    }

    #[test]
    fn mixed_case_paths() {
        assert_eq!(
            normalize(Path::new("/Path/To/FILE.Mp4")),
            "/path/to/file mp4"
        );
    }

    #[test]
    fn file_extensions() {
        assert_eq!(
            normalize(Path::new("/path/to/file.extension")),
            "/path/to/file extension"
        );
    }

    #[test]
    fn appros() {
        assert_eq!(
            normalize(Path::new("couldn't don't it's.mp4")),
            "couldnt dont its mp4"
        );
    }
}
