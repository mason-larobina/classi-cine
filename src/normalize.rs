use std::path::{Path, MAIN_SEPARATOR};

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
