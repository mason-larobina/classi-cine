use crate::Error;
use log::*;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::SystemTime;

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct File {
    pub dir: Arc<PathBuf>,
    pub file_name: PathBuf,
    pub size: u64,
    pub inode: u64,
    pub created: SystemTime,
}

pub struct Walk {
    exts: Arc<HashSet<OsString>>,
    tx: Arc<Sender<File>>,
    rx: Receiver<File>,
}

impl Walk {
    pub fn new<'a, T: IntoIterator<Item = &'a str>>(video_exts: T) -> Self {
        let mut exts: HashSet<OsString> = HashSet::new();
        for e in video_exts {
            let mut e = OsString::from(e);
            e.make_ascii_lowercase();
            exts.insert(e);
        }

        let (tx, rx) = std::sync::mpsc::channel();

        Walk {
            exts: Arc::new(exts),
            tx: Arc::new(tx),
            rx,
        }
    }

    pub fn walk_dir(&self, dir: &Path) -> Result<(), Error> {
        let dir = dir.canonicalize()?;
        let dir = Arc::new(dir);
        Self::inner_walk_dir(Arc::clone(&self.exts), Arc::clone(&self.tx), dir);
        Ok(())
    }

    pub fn into_rx(self) -> Receiver<File> {
        self.rx
    }

    fn inner_walk_dir(exts: Arc<HashSet<OsString>>, tx: Arc<Sender<File>>, dir: Arc<PathBuf>) {
        let entries = match std::fs::read_dir(&*dir) {
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
            let path = path.canonicalize().unwrap_or_else(|_| {
                warn!("Unable to canonicalize path: {:?}", path);
                path
            });

            let ft = metadata.file_type();
            if ft.is_dir() {
                let exts = Arc::clone(&exts);
                let tx = Arc::clone(&tx);
                let child_dir = Arc::new(path);
                rayon::spawn(move || {
                    Self::inner_walk_dir(exts, tx, child_dir);
                });
            } else if ft.is_file() {
                if let Some(ext) = path.extension().map(OsStr::to_ascii_lowercase) {
                    if !exts.contains(&ext) {
                        continue;
                    }
                } else {
                    continue;
                }
                let file_name = PathBuf::from(path.file_name().unwrap());
                let created = match metadata.created() {
                    Ok(time) => time,
                    Err(e) => {
                        warn!("Could not get creation time for {:?}: {}", path, e);
                        SystemTime::UNIX_EPOCH // Default to epoch if creation time is unavailable
                    }
                };
                let file = File {
                    dir: Arc::clone(&dir),
                    file_name,
                    size: metadata.len(),
                    inode: metadata.ino(),
                    created,
                };
                tx.send(file).unwrap();
            }
        }
    }
}
