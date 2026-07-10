use crate::path::AbsPath;
use globset::{GlobBuilder, GlobSetBuilder};
use log::*;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::SystemTime;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct File {
    pub path: AbsPath,
    pub size: u64,
    pub created: SystemTime,
}

/// Compiled set of `--exclude` glob patterns, matched against the absolute
/// path of each file and directory encountered during the walk.
///
/// Matching follows gitignore-flavored rules so common idioms Just Work:
///
/// * A pattern containing **no slash** is matched against the file/directory
///   *name* (its final path component), so `*.tmp` excludes any `.tmp` file
///   at any depth and `sample` excludes/prunes any directory *or* file named
///   `sample` anywhere.
/// * A pattern containing **a slash** is matched against the full absolute
///   path with full-match semantics, so `**/trailers/**` excludes files under
///   any `trailers` directory and `/abs/dir/**` anchors to a specific path.
///
/// Globbing uses `globset` with `literal_separator(true)`: `*` matches a
/// single path component and `**` spans any number of them (including the
/// leading absolute root). A directory whose own path/name matches is pruned
/// entirely — its subtree is never descended into.
#[derive(Debug, Clone, Default)]
pub struct Excludes {
    /// Slash-free patterns, matched against the basename of each path.
    basename: Option<globset::GlobSet>,
    /// Patterns containing a `/`, matched against the full absolute path.
    full_path: Option<globset::GlobSet>,
}

impl Excludes {
    /// Compile a list of glob patterns into a matcher. Empty input yields a
    /// no-op matcher (no allocation, no matching work per entry).
    pub fn new(patterns: &[String]) -> Result<Self, globset::Error> {
        if patterns.is_empty() {
            return Ok(Self::default());
        }
        let mut basename = GlobSetBuilder::new();
        let mut full_path = GlobSetBuilder::new();
        let mut bn = 0usize;
        let mut fp = 0usize;
        for p in patterns {
            let glob = GlobBuilder::new(p).literal_separator(true).build()?;
            if p.contains('/') {
                full_path.add(glob);
                fp += 1;
            } else {
                basename.add(glob);
                bn += 1;
            }
        }
        Ok(Self {
            basename: (bn > 0).then(|| basename.build()).transpose()?,
            full_path: (fp > 0).then(|| full_path.build()).transpose()?,
        })
    }

    /// Whether `path` (an absolute, normalized path) is excluded by any glob.
    /// Slash-free patterns are tested against the final component only.
    pub fn is_excluded(&self, path: &Path) -> bool {
        if let Some(set) = &self.full_path
            && set.is_match(path)
        {
            return true;
        }
        if let Some(set) = &self.basename
            && let Some(name) = path.file_name()
            && set.is_match(name)
        {
            return true;
        }
        false
    }
}

pub struct Walk {
    exts: Arc<HashSet<OsString>>,
    excludes: Arc<Excludes>,
    tx: Arc<Sender<File>>,
    rx: Receiver<File>,
}

impl Walk {
    pub fn new<'a, T: IntoIterator<Item = &'a str>>(video_exts: T, excludes: Excludes) -> Self {
        let mut exts: HashSet<OsString> = HashSet::new();
        for e in video_exts {
            let mut e = OsString::from(e);
            e.make_ascii_lowercase();
            exts.insert(e);
        }

        let (tx, rx) = std::sync::mpsc::channel();

        Walk {
            exts: Arc::new(exts),
            excludes: Arc::new(excludes),
            tx: Arc::new(tx),
            rx,
        }
    }

    pub fn walk_dir(&self, dir: &Path) {
        // Ensure all directory paths are absolute for internal processing
        let abs_dir = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir().unwrap().join(dir)
        };
        let abs_dir = crate::path::normalize_path(&abs_dir);
        Self::inner_walk_dir(
            Arc::clone(&self.exts),
            Arc::clone(&self.excludes),
            Arc::clone(&self.tx),
            abs_dir,
        );
    }

    pub fn into_rx(self) -> Receiver<File> {
        self.rx
    }

    fn inner_walk_dir(
        exts: Arc<HashSet<OsString>>,
        excludes: Arc<Excludes>,
        tx: Arc<Sender<File>>,
        dir: PathBuf,
    ) {
        // Prune the subtree: if the directory's own absolute path is excluded,
        // skip descending into it entirely (saves statting every file below).
        if excludes.is_excluded(&dir) {
            debug!("Excluded directory (pruned): {:?}", dir);
            return;
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                error!("Error reading directory {:?} {:?}", dir, e);
                return;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    error!("Error reading entry: {:?}", e);
                    continue;
                }
            };

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(e) => {
                    error!("Error reading metadata: {:?}", e);
                    continue;
                }
            };

            let path = entry.path();

            let ft = metadata.file_type();
            if ft.is_dir() {
                let exts = Arc::clone(&exts);
                let excludes = Arc::clone(&excludes);
                let tx = Arc::clone(&tx);
                rayon::spawn(move || {
                    Self::inner_walk_dir(exts, excludes, tx, path);
                });
            } else if ft.is_file() {
                // Skip files whose absolute path is excluded by any glob.
                if excludes.is_excluded(&path) {
                    debug!("Excluded file: {:?}", path);
                    continue;
                }
                if let Some(ext) = path.extension().map(OsStr::to_ascii_lowercase) {
                    if !exts.contains(&ext) {
                        continue;
                    }
                } else {
                    continue;
                }
                let created = match metadata.modified() {
                    Ok(time) => time,
                    Err(e) => {
                        warn!("Could not get creation time for {:?}: {}", path, e);
                        SystemTime::UNIX_EPOCH // Default to epoch if creation time is unavailable
                    }
                };
                let abs_path = AbsPath::from_abs_path(&path);
                let file = File {
                    path: abs_path,
                    size: metadata.len(),
                    created,
                };
                tx.send(file).unwrap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excludes(patterns: &[&str]) -> Excludes {
        Excludes::new(&patterns.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .expect("valid patterns")
    }

    #[test]
    fn empty_excludes_matches_nothing() {
        let ex = Excludes::new(&[]).unwrap();
        assert!(!ex.is_excluded(Path::new("/anything/at/all.mkv")));
    }

    #[test]
    fn prune_directory_by_own_path() {
        // A slash-free pattern matches the basename anywhere — so `sample`
        // prunes any directory (or file) named `sample` at any depth.
        let ex = excludes(&["sample"]);
        assert!(ex.is_excluded(Path::new("/media/sample")));
        assert!(ex.is_excluded(Path::new("/a/b/sample")));
        assert!(!ex.is_excluded(Path::new("/media/movies")));
    }

    #[test]
    fn exclude_files_under_directory() {
        // `**/sample/**` (has a slash) matches files inside `sample/` against
        // the full absolute path. It does not match the bare directory; prune
        // the directory itself with a slash-free `sample` (above) instead.
        let ex = excludes(&["**/sample/**"]);
        assert!(ex.is_excluded(Path::new("/media/sample/x.mkv")));
        assert!(ex.is_excluded(Path::new("/a/b/sample/c/d.mkv")));
        assert!(!ex.is_excluded(Path::new("/media/sample")));
    }

    #[test]
    fn basename_star_matches_anywhere() {
        // `*.tmp` is slash-free, so it matches the basename at any depth.
        let ex = excludes(&["*.tmp"]);
        assert!(ex.is_excluded(Path::new("/x/y.tmp")));
        assert!(ex.is_excluded(Path::new("/a/b/c.tmp")));
        assert!(ex.is_excluded(Path::new("y.tmp")));
        assert!(!ex.is_excluded(Path::new("/x/y/z.mkv")));
    }

    #[test]
    fn anchored_absolute_path() {
        let ex = excludes(&["/media/trailers/**"]);
        assert!(ex.is_excluded(Path::new("/media/trailers/a/b.mp4")));
        assert!(!ex.is_excluded(Path::new("/other/trailers/a.mp4")));
    }

    #[test]
    fn invalid_glob_is_reported() {
        assert!(Excludes::new(&["[unclosed".to_string()]).is_err());
    }
}
