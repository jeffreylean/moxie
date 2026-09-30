use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded};
use serde::{Deserialize, Serialize};

pub(super) const PROTOCOL: u32 = 1;
const MAX_RECORD: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct ModelChoice {
    pub provider: String,
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Command {
    Initialize { protocol: u32, cwd: PathBuf },
    Prompt { id: u64, text: String },
    Cancel { id: u64 },
    SetModel { provider: String, model: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Event {
    Ready {
        protocol: u32,
        models: Vec<ModelChoice>,
        selected: Option<ModelChoice>,
    },
    ModelChanged {
        model: ModelChoice,
    },
    AssistantStart {
        id: u64,
    },
    Text {
        id: u64,
        delta: String,
    },
    AssistantEnd {
        id: u64,
        text: String,
    },
    Finished {
        id: u64,
        cancelled: bool,
    },
    Error {
        id: Option<u64>,
        message: String,
    },
}

pub(super) struct Worker {
    child: Child,
    commands: Sender<Command>,
    pub events: Receiver<Result<Event, String>>,
    started: Instant,
}

impl Worker {
    pub fn spawn(cwd: &Path) -> Result<Self, String> {
        let entry = worker_path();
        if !entry.is_file() {
            return Err(
                "Pi is not built. Run npm ci and npm run build in agent/."
                    .into(),
            );
        }
        let entry = entry.canonicalize().map_err(|error| {
            format!("Could not locate the Pi worker: {error}")
        })?;
        let node =
            std::env::var_os("MOXIE_NODE").unwrap_or_else(|| {
                let bundled = std::env::current_exe()
                    .ok()
                    .and_then(|exe| {
                        exe.parent().map(Path::to_path_buf)
                    })
                    .map(|dir| {
                        dir.join("agent").join(if cfg!(windows) {
                            "node.exe"
                        } else {
                            "node"
                        })
                    });
                bundled
                    .filter(|path| path.is_file())
                    .map(PathBuf::into_os_string)
                    .unwrap_or_else(|| "node".into())
            });
        let mut command = ProcessCommand::new(node);
        command
            .arg(entry)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()
            .map_err(|error| format!("Could not start Pi. Install Node.js 22.19 or newer: {error}"))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (commands, requests) = bounded::<Command>(16);
        let (send_events, events) = bounded(256);
        let reader_events = send_events.clone();
        let worker = Self {
            child,
            commands,
            events,
            started: Instant::now(),
        };

        thread::Builder::new()
            .name("pi-output".into())
            .spawn(move || {
                let mut output = BufReader::new(stdout);
                loop {
                    let mut record = Vec::new();
                    let event = match output.by_ref().take(MAX_RECORD + 1).read_until(b'\n', &mut record) {
                        Ok(0) => Err("Pi disconnected. Check the agent setup and start a new chat.".into()),
                        Ok(_) if record.len() as u64 > MAX_RECORD => Err("Pi sent an oversized response.".into()),
                        Ok(_) => serde_json::from_slice(&record)
                            .map_err(|error| format!("Invalid Pi response: {error}")),
                        Err(error) => Err(format!("Could not read from Pi: {error}")),
                    };
                    let failed = event.is_err();
                    if reader_events.send(event).is_err() || failed {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;

        thread::Builder::new()
            .name("pi-input".into())
            .spawn(move || {
                let mut input = stdin;
                for command in requests {
                    let result =
                        serde_json::to_writer(&mut input, &command)
                            .map_err(std::io::Error::other)
                            .and_then(|()| input.write_all(b"\n"))
                            .and_then(|()| input.flush());
                    if let Err(error) = result {
                        let _ = send_events.send(Err(format!(
                            "Could not send to Pi: {error}"
                        )));
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;

        worker.send(Command::Initialize {
            protocol: PROTOCOL,
            cwd: cwd.to_path_buf(),
        })?;
        Ok(worker)
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        self.commands.try_send(command).map_err(|error| {
            format!("Pi is not accepting messages: {error}")
        })
    }

    pub fn startup_timed_out(&self) -> bool {
        self.started.elapsed() > Duration::from_secs(30)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn worker_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MOXIE_PI_WORKER") {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let bundled = dir.join("agent/dist/index.js");
        if bundled.is_file() {
            return bundled;
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../agent/dist/index.js")
}
