use crate::Error;
use log::*;
use rand::Rng;
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
    args: crate::VlcArgs,
}

impl VLCProcessHandle {
    pub fn new(
        args: &crate::VlcArgs,
        path: &Path,
        file_name: Option<String>,
    ) -> Result<Self, Error> {
        // Find an available port
        let listener = TcpListener::bind("127.0.0.1:0").map_err(Error::Io)?;
        let port = listener.local_addr().map_err(Error::Io)?.port();
        // Drop the listener so VLC can use the port
        drop(listener);

        let password: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(16)
            .map(char::from)
            .collect();

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
                &password,
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

        let child = command.spawn().map_err(|e| Error::ProcessFailed(e))?;

        Ok(VLCProcessHandle {
            handle: Some(child),
            status_url: format!("http://:{password}@localhost:{port}/requests/status.json"),
            file_name,
            args: args.clone(),
        })
    }

    pub fn status(&self) -> Result<Status, Error> {
        let response = reqwest::blocking::get(&self.status_url)
            .map_err(|e| Error::VLCNotResponding(format!("Failed to connect to VLC: {}", e)))?;
        
        let text = response.text()
            .map_err(|e| Error::VLCNotResponding(format!("Failed to get response text: {}", e)))?;
        
        debug!("Response: {}", text);
        
        Ok(serde_json::from_str(&text).map_err(Error::SerdeJson)?)
    }

    pub fn wait_for_status(&self) -> Result<Status, Error> {
        let attempts = (self.args.vlc_timeout * 1000) / self.args.vlc_poll_interval;
        for _attempt in 0..attempts {
            std::thread::sleep(std::time::Duration::from_millis(
                self.args.vlc_poll_interval,
            ));

            match self.status() {
                Ok(status) => {
                    // Wait until we get a filename from VLC
                    if let Some(vlc_filename) = status.file_name() {
                        // If we have an expected filename, verify it matches
                        if let Some(ref expected) = self.file_name {
                            if vlc_filename != *expected {
                                return Err(Error::FilenameMismatch {
                                    expected: expected.clone(),
                                    got: vlc_filename,
                                });
                            }
                        }
                        return Ok(status);
                    }
                }
                Err(e) => {
                    debug!("Status check failed: {:?}", e);
                }
            }
        }
        Err(Error::Timeout(format!(
            "VLC did not respond with valid status after {} seconds",
            self.args.vlc_timeout
        )))
    }

    /// Get classification from user via VLC controls
    pub fn get_classification(&self) -> Result<Classification, Error> {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));

            let status = match self
                .status()
                .map_err(|e| Error::VLCNotResponding(e.to_string()))
            {
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
