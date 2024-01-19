use log::*;
use rayon::ThreadPool;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::sync::Mutex;
use walkdir::WalkDir;

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum Entry {
    File {
        file: PathBuf,
        size: u64,
        inode: u64,
    },
    Dir {
        dir: PathBuf,
    },
}

fn inner_walk_dir(
    root: Arc<PathBuf>,
    dir: PathBuf,
    exts: Arc<HashSet<OsString>>,
    tx: Arc<Sender<Entry>>,
) {
    debug!("Walk dir: {:?}", dir);

    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => {
            error!("Error reading directory: {:?}", e);
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
            let rel_dir = path.strip_prefix(&*root).unwrap().to_path_buf();
            tx.send(Entry::Dir { dir: rel_dir }).unwrap();

            let root = root.clone();
            let child_dir = path.to_path_buf();
            let exts = exts.clone();
            let tx = tx.clone();
            rayon::spawn(move || {
                inner_walk_dir(root, child_dir, exts, tx);
            });

            continue;
        }

        if ft.is_file() {
            match path.extension() {
                Some(ext) => {
                    let ext = ext.to_ascii_lowercase();
                    if !exts.contains(&ext) {
                        continue;
                    }
                }
                None => continue,
            }

            let rel_file = path.strip_prefix(&*root).unwrap().to_path_buf();
            let size = metadata.len();
            let inode = metadata.ino();
            tx.send(Entry::File {
                file: rel_file,
                size,
                inode,
            })
            .unwrap();
        }
    }
}

pub fn walk_root(video_exts: &Vec<String>, root: Arc<PathBuf>) -> Vec<Entry> {
    let mut exts: HashSet<OsString> = HashSet::new();
    for e in video_exts {
        let mut e = OsString::from(e);
        e.make_ascii_lowercase();
        exts.insert(e);
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let tx = Arc::new(tx);
    let exts = Arc::new(exts);

    inner_walk_dir(root.clone(), (*root).clone(), exts.clone(), tx.clone());

    drop(tx);
    let mut ret = Vec::new();
    while let Ok(e) = rx.recv() {
        ret.push(e);
    }

    ret.sort();
    ret
}
