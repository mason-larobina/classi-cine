use log::*;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::sync::Mutex;
use walkdir::WalkDir;

pub struct Walk {
    exts: HashSet<OsString>,
    tx: Arc<Sender<Vec<(PathBuf, u64)>>>,
    rx: Mutex<Receiver<Vec<(PathBuf, u64)>>>,
}

impl Walk {
    pub fn new(video_exts: &Vec<String>) -> Self {
        let mut exts: HashSet<OsString> = HashSet::new();
        for e in video_exts {
            let mut e = OsString::from(e);
            e.make_ascii_lowercase();
            exts.insert(e);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Arc::new(tx);
        let rx = Mutex::new(rx);
        Self { exts, tx, rx }
    }

    pub fn root(&self, root: &Path) {
        info!("Walk {:?}", root);

        rayon::scope(|s| {
            let mut files = Vec::new();
            for e in WalkDir::new(root).max_depth(1) {
                let e = e.unwrap();
                let path = e.path();
                let ft = e.file_type();

                if ft.is_dir() && e.depth() == 1 {
                    let path = path.to_path_buf();
                    s.spawn(move |_| {
                        self.root(&path);
                    });
                } else if ft.is_file() {
                    match path.extension() {
                        Some(ext) => {
                            if !self.exts.contains(ext) {
                                continue;
                            }
                        }
                        None => continue,
                    }
                    let canon = std::fs::canonicalize(path).unwrap();
                    let size = e.metadata().unwrap().len();
                    files.push((canon, size));
                }
            }
            self.tx.send(files).unwrap();
        });
    }

    pub fn collect(self) -> HashMap<PathBuf, u64> {
        drop(self.tx);
        let mut ret = HashMap::new();
        let rx = self.rx.lock().unwrap();
        while let Ok(vec) = rx.recv() {
            for (k, v) in vec {
                ret.insert(k, v);
            }
        }
        ret
    }
}
