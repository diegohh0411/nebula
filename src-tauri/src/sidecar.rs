use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

pub struct SidecarProcess {
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

pub struct SidecarManager {
    process: Option<SidecarProcess>,
    ready: bool,
}

impl SidecarManager {
    pub fn new() -> Self {
        SidecarManager {
            process: None,
            ready: false,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.process.is_some() {
            self.shutdown()?;
        }

        let script_path = std::env::var("CARGO_MANIFEST_DIR")
            .map(|d| {
                std::path::Path::new(&d)
                    .parent()
                    .map(|p| p.join("sidecar").join("main.py"))
            })
            .ok()
            .flatten()
            .unwrap_or_else(|| std::path::PathBuf::from("../sidecar/main.py"));

        let python = if cfg!(windows) { "python" } else { "python3" };

        let mut child = Command::new(python)
            .arg(&script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                format!(
                    "Failed to start sidecar: {}. Path: {}",
                    e,
                    script_path.display()
                )
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or("Failed to get sidecar stdin")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Failed to get sidecar stdout")?;
        let mut reader = BufReader::new(stdout);

        // Wait for loading signal
        let mut ready_line = String::new();
        reader
            .read_line(&mut ready_line)
            .map_err(|e| format!("Failed to read sidecar ready: {}", e))?;
        let ready_msg: Value = serde_json::from_str(ready_line.trim())
            .map_err(|e| format!("Invalid sidecar ready message: {}", e))?;

        if ready_msg["action"] == "ready" && ready_msg["status"] == "loading" {
            // Model is loading, wait for the real ready
            ready_line.clear();
            reader
                .read_line(&mut ready_line)
                .map_err(|e| format!("Failed to read sidecar ready: {}", e))?;
            let msg: Value = serde_json::from_str(ready_line.trim())
                .map_err(|e| format!("Invalid sidecar ready message: {}", e))?;
            if msg["status"] != "ok" {
                return Err(format!("Sidecar failed to initialize: {}", msg));
            }
        }

        self.process = Some(SidecarProcess {
            stdin,
            stdout: reader,
        });
        self.ready = true;

        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        if let Some(ref mut _proc) = self.process {
            let _ = self.send_raw(&json!({"action": "shutdown"}));
            // Give the process a moment to exit gracefully, then drop (which kills it)
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        self.process = None;
        self.ready = false;
        Ok(())
    }

    pub fn send_request(&mut self, request: &Value) -> Result<Value, String> {
        if !self.ready || self.process.is_none() {
            return Err("Sidecar is not running".to_string());
        }
        self.send_raw(request)?;

        let proc = self
            .process
            .as_mut()
            .ok_or("Sidecar process lost")?;
        let mut line = String::new();
        proc.stdout
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read sidecar response: {}", e))?;

        if line.trim().is_empty() {
            return Err("Sidecar returned empty response".to_string());
        }

        let response: Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("Invalid sidecar response: {} - {}", e, line.trim()))?;

        if response["status"] == "error" {
            return Err(
                response["message"]
                    .as_str()
                    .unwrap_or("Unknown error")
                    .to_string(),
            );
        }

        Ok(response)
    }

    fn send_raw(&mut self, request: &Value) -> Result<(), String> {
        let proc = self
            .process
            .as_mut()
            .ok_or("Sidecar process lost")?;
        let msg = format!(
            "{}\n",
            serde_json::to_string(request).map_err(|e| e.to_string())?
        );
        proc.stdin
            .write_all(msg.as_bytes())
            .map_err(|e| format!("Failed to write to sidecar: {}", e))?;
        proc.stdin
            .flush()
            .map_err(|e| format!("Failed to flush sidecar stdin: {}", e))?;
        Ok(())
    }
}
