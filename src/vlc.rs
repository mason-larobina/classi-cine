use crate::Error;
use log::*;
use serde::Deserialize;
use std::path::Path;
use std::process::{Child, Command, Stdio};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Status {
    state: String,
    information: Option<Information>,
    position: f64,
    length: f64,
}

impl Status {
    pub fn file_name(&self) -> Option<String> {
        self.information
            .as_ref()
            .map(|i| i.category.meta.filename.clone())
    }

    pub fn state(&self) -> &str {
        self.state.as_str()
    }
}

#[derive(Debug, Deserialize)]
pub struct Information {
    category: Category,
}

#[derive(Debug, Deserialize)]
pub struct Category {
    meta: Meta,
}

#[derive(Debug, Deserialize)]
pub struct Meta {
    filename: String,
}

pub struct VLCProcessHandle {
    handle: Option<Child>,
    status_url: String,
}

impl VLCProcessHandle {
    pub fn new(args: &crate::Args, path: &Path) -> Self {
        let mut command = Command::new("vlc");
        command
            .args([
                "-I",
                "http",
                "--no-random",
                "--no-loop",
                "--repeat",
                "--no-play-and-exit",
                "--http-host",
                "localhost",
                "--http-password",
                "password",
                "--http-port",
            ])
            .arg(format!("{}", args.vlc_port))
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if args.fullscreen {
            command.arg("--fullscreen");
        }

        debug!("Spawn {:?}", command);

        let child = command.spawn().expect("Failed to start VLC process");

        VLCProcessHandle {
            handle: Some(child),
            status_url: format!(
                "http://:password@localhost:{}/requests/status.json",
                args.vlc_port
            ),
        }
    }

    pub fn status(&self) -> Result<Status, Error> {
        let response = reqwest::blocking::get(&self.status_url)?;
        let text = response.text()?;
        debug!("Response: {}", text);
        Ok(serde_json::from_str(&text)?)
    }

    pub fn wait_for_status(&self) -> Result<Status, Error> {
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(status) = self.status() {
                if status.file_name().is_some() && status.length > 0.0 && status.position > 0.0 {
                    return Ok(status);
                }
            }
        }
        Err(Error::Timeout)
    }
}

impl Drop for VLCProcessHandle {
    fn drop(&mut self) {
        if let Some(mut child) = self.handle.take() {
            let kill_result = child.kill();
            debug!("kill {:?}", kill_result);
            let wait_result = child.wait();
            debug!("wait {:?}", wait_result);
        }
    }
}
