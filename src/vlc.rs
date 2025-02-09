use crate::Error;
use log::*;
use serde::Deserialize;
use std::net::TcpListener;
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

#[derive(Debug)]
pub enum Classification {
    Positive,
    Negative,
    Skipped,
}

pub struct VLCProcessHandle {
    handle: Option<Child>,
    status_url: String,
    file_name: Option<String>,
}

impl VLCProcessHandle {
    pub fn new(args: &crate::Args, path: &Path, file_name: Option<String>) -> Self {
        // Find an available port
        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to address");
        let port = listener
            .local_addr()
            .expect("Failed to get local address")
            .port();

        // Drop the listener so VLC can use the port
        drop(listener);

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
            .arg(format!("{}", port))
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
            status_url: format!("http://:password@localhost:{}/requests/status.json", port),
            file_name,
        }
    }

    pub fn status(&self) -> Result<Status, Error> {
        let response = reqwest::blocking::get(&self.status_url)?;
        let text = response.text()?;
        debug!("Response: {}", text);
        Ok(serde_json::from_str(&text)?)
    }

    pub fn wait_for_status(&self, timeout_secs: u64) -> Result<Status, Error> {
        let attempts = (timeout_secs * 1000) / 100; // Convert to 100ms intervals
        for _ in 0..attempts {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(status) = self.status() {
                // Verify filename matches if we have one
                if let Some(ref expected) = self.file_name {
                    if status.file_name() != Some(expected.clone()) {
                        error!(
                            "Filename mismatch {:?} {:?}, skipping",
                            self.file_name,
                            status.file_name()
                        );
                        return Err(Error::FilenameMismatch);
                    }
                }
                if status.file_name().is_some() && status.length > 0.0 {
                    return Ok(status);
                }
            }
        }
        Err(Error::Timeout)
    }

    /// Get classification from user via VLC controls
    pub fn get_classification(&self) -> Result<Classification, Error> {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));

            let status = match self.status() {
                Ok(status) => {
                    debug!("{:?}", status);
                    status
                }
                Err(e) => {
                    error!("Status error: {:?}", e);
                    return Ok(Classification::Skipped);
                }
            };

            match status.state() {
                "stopped" => return Ok(Classification::Positive),
                "paused" => return Ok(Classification::Negative),
                _ => {}
            }
        }
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
