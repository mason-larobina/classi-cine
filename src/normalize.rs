use std::path::MAIN_SEPARATOR;

/// Normalizes a file path by performing the following transformations:
/// 1. Converts to lowercase
/// 2. Replaces non-alphanumeric characters (except path separators) with spaces
/// 3. Collapses consecutive special characters into single spaces
/// 4. Removes apostrophes and trailing spaces
/// 5. Maintains single path separators while cleaning surrounding spaces
///
/// Convert path to canonical form, falling back to absolute + normalized if path doesn't exist
pub fn normalize(file: &str) -> String {
    let file = file.to_lowercase();
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
    fn special_characters() {
        assert_eq!(
            normalize("/path/to/special@file!.mp4"),
            "/path/to/special file mp4"
        );
    }

    #[test]
    fn unicode_characters() {
        assert_eq!(normalize("/path/to/ünîcødé.mp4"), "/path/to/ünîcødé mp4");
    }

    #[test]
    fn empty_and_minimal_paths() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("a"), "a");
    }

    #[test]
    fn consecutive_special_characters() {
        assert_eq!(normalize("/path//to///file.mp4"), "/path/to/file mp4");
        assert_eq!(
            normalize("/path/to/special!!!file.mp4"),
            "/path/to/special file mp4"
        );
    }

    #[test]
    fn very_long_paths() {
        let long_path = "/path".repeat(100) + "file.mp4";
        let expected = "/path".repeat(100) + "file mp4";
        assert_eq!(normalize(&long_path), expected);
    }

    #[test]
    fn paths_with_only_special_characters() {
        assert_eq!(normalize("***"), "");
        assert_eq!(normalize("///"), "/");
    }

    #[test]
    fn directory_names_with_trailing_special_characters() {
        assert_eq!(
            normalize("/path/to/special_folder!!!/file.mp4"),
            "/path/to/special folder/file mp4"
        );
    }

    #[test]
    fn mixed_case_paths() {
        assert_eq!(normalize("/Path/To/FILE.Mp4"), "/path/to/file mp4");
    }

    #[test]
    fn file_extensions() {
        assert_eq!(
            normalize("/path/to/file.extension"),
            "/path/to/file extension"
        );
    }

    #[test]
    fn appros() {
        assert_eq!(normalize("couldn't don't it's.mp4"), "couldnt dont its mp4");
    }
}
