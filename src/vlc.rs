use crate::Error;
use log::*;
use rand::distr::{Alphanumeric, Distribution};
use serde::Deserialize;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};

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

#[derive(Debug)]
enum ControlMessage {
    StartPlayback {
        path: PathBuf,
        file_name: Option<String>,
    },
    Shutdown,
}

#[derive(Debug)]
enum StatusMessage {
    Classification(Classification),
    Error(String),
}

pub struct VlcController {
    control_tx: Sender<ControlMessage>,
    status_rx: Receiver<StatusMessage>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct VlcBackground {
    control_rx: Receiver<ControlMessage>,
    status_tx: Sender<StatusMessage>,
    args: crate::VlcArgs,
    current_playback: Option<PlaybackState>,
}

struct PlaybackState {
    child: Child,
    status_url: String,
    file_name: Option<String>,
    waiting_initial: bool,
    attempts_remaining: u32,
}

impl Drop for PlaybackState {
    fn drop(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl VlcBackground {
    fn new(
        control_rx: Receiver<ControlMessage>,
        status_tx: Sender<StatusMessage>,
        args: crate::VlcArgs,
    ) -> Self {
        Self {
            control_rx,
            status_tx,
            args,
            current_playback: None,
        }
    }

    fn run(&mut self) {
        loop {
            self.handle_control();

            self.poll_status();

            // Sleep before next iteration
            std::thread::sleep(std::time::Duration::from_millis(
                self.args.vlc_poll_interval,
            ));
        }
    }

    fn handle_control(&mut self) {
        match self.control_rx.try_recv() {
            Ok(ControlMessage::StartPlayback { path, file_name }) => {
                // If there's a current playback, kill it and send Skipped
                self.current_playback = None;

                // Start new playback
                match self.spawn_vlc(&path) {
                    Ok((child, status_url)) => {
                        self.current_playback = Some(PlaybackState {
                            child,
                            status_url,
                            file_name,
                            waiting_initial: true,
                            attempts_remaining: ((self.args.vlc_timeout * 1000)
                                / self.args.vlc_poll_interval)
                                as u32,
                        });
                    }
                    Err(e) => {
                        self.status_tx
                            .send(StatusMessage::Error(e.to_string()))
                            .unwrap();
                    }
                }
            }
            Ok(ControlMessage::Shutdown) => {
                self.current_playback = None;
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    fn poll_status(&mut self) {
        let mut terminate = false;
        let mut send_msg: Option<StatusMessage> = None;

        if let Some(ref mut state) = self.current_playback.as_mut() {
            match VlcBackground::get_status(&state.status_url) {
                Ok(status) => {
                    if state.waiting_initial {
                        if let Some(vlc_filename) = status.file_name() {
                            let mismatch = state
                                .file_name
                                .as_ref()
                                .map_or(false, |expected| vlc_filename != *expected);
                            if mismatch {
                                terminate = true;
                                send_msg =
                                    Some(StatusMessage::Error("Filename mismatch".to_string()));
                            } else {
                                state.waiting_initial = false;
                            }
                        } else {
                            state.attempts_remaining = state.attempts_remaining.saturating_sub(1);
                            if state.attempts_remaining == 0 {
                                terminate = true;
                                send_msg = Some(StatusMessage::Error(format!(
                                    "VLC did not respond with valid status after {} seconds",
                                    self.args.vlc_timeout
                                )));
                            }
                        }
                    } else {
                        // Polling for classification
                        let classification = match status.state() {
                            "stopped" => Some(Classification::Positive),
                            "paused" => Some(Classification::Negative),
                            _ => None,
                        };
                        if let Some(c) = classification {
                            terminate = true;
                            send_msg = Some(StatusMessage::Classification(c));
                        }
                    }
                }
                Err(e) => {
                    debug!("Status check failed: {:?}", e);
                    if !state.waiting_initial {
                        // For classification phase, treat error as Skipped
                        terminate = true;
                        send_msg = Some(StatusMessage::Classification(Classification::Skipped));
                    }
                }
            }
        }

        if terminate {
            self.current_playback = None;
        }

        if let Some(msg) = send_msg {
            self.status_tx.send(msg).unwrap();
        }
    }

    fn spawn_vlc(&self, path: &Path) -> Result<(Child, String), Error> {
        // Find an available port
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        // Drop the listener so VLC can use the port
        drop(listener);

        let mut rng = rand::rng();
        let password: String = Alphanumeric
            .sample_iter(&mut rng)
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

        if self.args.fullscreen {
            command.arg("--fullscreen");
        }

        debug!("Spawn {:?}", command);

        let child = command.spawn().map_err(|e| Error::ProcessFailed(e))?;

        let status_url = format!("http://:{password}@localhost:{port}/requests/status.json");

        Ok((child, status_url))
    }

    fn get_status(status_url: &str) -> Result<Status, Error> {
        let response = reqwest::blocking::get(status_url)
            .map_err(|e| Error::VLCNotResponding(format!("Failed to connect to VLC: {}", e)))?;

        let text = response
            .text()
            .map_err(|e| Error::VLCNotResponding(format!("Failed to get response text: {}", e)))?;

        debug!("Response: {}", text);

        Ok(serde_json::from_str(&text).map_err(Error::SerdeJson)?)
    }
}

impl VlcController {
    pub fn new(args: crate::VlcArgs) -> Self {
        let (control_tx, control_rx) = channel();
        let (status_tx, status_rx) = channel();

        let args_clone = args.clone();
        let thread = std::thread::spawn(move || {
            let mut bg = VlcBackground::new(control_rx, status_tx, args_clone);
            bg.run();
        });

        Self {
            control_tx,
            status_rx,
            thread: Some(thread),
        }
    }

    pub fn start_playback(&self, path: &Path, file_name: Option<String>) -> Result<(), Error> {
        self.control_tx
            .send(ControlMessage::StartPlayback {
                path: path.to_path_buf(),
                file_name,
            })
            .map_err(|e| Error::VLCNotResponding(e.to_string()))
    }

    pub fn try_recv_classification(&self) -> Result<Option<Classification>, Error> {
        match self.status_rx.try_recv() {
            Ok(StatusMessage::Classification(c)) => Ok(Some(c)),
            Ok(StatusMessage::Error(e)) => Err(Error::VLCNotResponding(e)),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(e) => Err(Error::VLCNotResponding(e.to_string())),
        }
    }
}

impl Drop for VlcController {
    fn drop(&mut self) {
        self.control_tx.send(ControlMessage::Shutdown).unwrap();
        if let Err(e) = self.thread.take().unwrap().join() {
            error!("Failed to join VLC thread: {:?}", e);
        }
    }
}
