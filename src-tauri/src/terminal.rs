use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tauri::Emitter;

use crate::get_extended_path;

#[derive(Default)]
pub struct TerminalManager {
    sessions: Mutex<HashMap<String, TerminalSession>>,
    next_id: AtomicU64,
}

struct TerminalSession {
    id: String,
    cwd: String,
    issue_id: Option<String>,
    shell: String,
    cols: u16,
    rows: u16,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminalCreateRequest {
    pub cwd: String,
    #[serde(rename = "issueId")]
    pub issue_id: Option<String>,
    pub shell: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalSessionInfo {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    #[serde(rename = "issueId")]
    pub issue_id: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalEvent {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub kind: String,
    pub data: Option<String>,
    pub message: Option<String>,
    pub code: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminalWriteRequest {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminalResizeRequest {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminalSessionRequest {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

pub trait TerminalEventSink: Clone + Send + Sync + 'static {
    fn emit(&self, event: TerminalEvent);
}

#[derive(Clone)]
pub struct TauriTerminalEventSink {
    app: tauri::AppHandle,
}

impl TauriTerminalEventSink {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl TerminalEventSink for TauriTerminalEventSink {
    fn emit(&self, event: TerminalEvent) {
        let event_name = match event.kind.as_str() {
            "data" => "terminal:data",
            "exit" => "terminal:exit",
            "error" => "terminal:error",
            _ => "terminal:event",
        };

        if let Err(err) = self.app.emit(event_name, event) {
            log::error!("[terminal] Failed to emit {}: {}", event_name, err);
        }
    }
}

impl TerminalManager {
    pub fn create_session<S: TerminalEventSink>(
        &self,
        request: TerminalCreateRequest,
        sink: S,
    ) -> Result<TerminalSessionInfo, String> {
        let id = self.next_session_id();
        self.create_session_with_id(id, request, sink)
    }

    pub fn write(&self, session_id: &str, data: &str) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Terminal session not found: {}", session_id))?;

        session
            .writer
            .write_all(data.as_bytes())
            .map_err(|e| format!("Failed to write to terminal session {}: {}", session_id, e))?;
        session
            .writer
            .flush()
            .map_err(|e| format!("Failed to flush terminal session {}: {}", session_id, e))?;
        Ok(())
    }

    pub fn close(&self, session_id: &str) -> Result<(), String> {
        let mut session = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            sessions
                .remove(session_id)
                .ok_or_else(|| format!("Terminal session not found: {}", session_id))?
        };

        session.terminate();
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<TerminalSessionInfo>, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        Ok(sessions.values().map(TerminalSession::info).collect())
    }

    pub fn resize(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalSessionInfo, String> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Terminal session not found: {}", session_id))?;

        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to resize terminal session {}: {}", session_id, e))?;
        session.cols = cols;
        session.rows = rows;
        Ok(session.info())
    }

    pub fn restart<S: TerminalEventSink>(
        &self,
        session_id: &str,
        sink: S,
    ) -> Result<TerminalSessionInfo, String> {
        let mut previous = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            sessions
                .remove(session_id)
                .ok_or_else(|| format!("Terminal session not found: {}", session_id))?
        };

        let request = TerminalCreateRequest {
            cwd: previous.cwd.clone(),
            issue_id: previous.issue_id.clone(),
            shell: Some(previous.shell.clone()),
            cols: Some(previous.cols),
            rows: Some(previous.rows),
        };
        let id = previous.id.clone();
        previous.terminate();

        self.create_session_with_id(id, request, sink)
    }

    fn next_session_id(&self) -> String {
        let next = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        format!("term-{}", next)
    }

    fn create_session_with_id<S: TerminalEventSink>(
        &self,
        id: String,
        request: TerminalCreateRequest,
        sink: S,
    ) -> Result<TerminalSessionInfo, String> {
        let cwd = validate_cwd(&request.cwd)?;
        let cols = request.cols.unwrap_or(80).max(1);
        let rows = request.rows.unwrap_or(24).max(1);
        let shell = request.shell.clone().unwrap_or_else(default_shell);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to open PTY: {}", e))?;

        let mut command = CommandBuilder::new(&shell);
        command.cwd(cwd.clone());
        command.env("PATH", get_extended_path());
        command.env("TERM", "xterm-256color");
        command.env("BEADS_PATH", cwd.to_string_lossy().to_string());
        command.env("BORABR_TERMINAL_SESSION_ID", &id);
        if let Some(issue_id) = &request.issue_id {
            command.env("BORABR_ISSUE_ID", issue_id);
        }

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone PTY reader: {}", e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to open PTY writer: {}", e))?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("Failed to spawn shell {}: {}", shell, e))?;

        spawn_reader_thread(id.clone(), reader, sink);

        let info = TerminalSessionInfo {
            session_id: id.clone(),
            cwd: cwd.to_string_lossy().to_string(),
            issue_id: request.issue_id.clone(),
            cols,
            rows,
        };

        let session = TerminalSession {
            id: id.clone(),
            cwd: info.cwd.clone(),
            issue_id: request.issue_id,
            shell,
            cols,
            rows,
            master: pair.master,
            writer,
            child,
        };

        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        sessions.insert(id, session);

        Ok(info)
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        if let Ok(sessions) = self.sessions.get_mut() {
            for session in sessions.values_mut() {
                session.terminate();
            }
        }
    }
}

impl TerminalSession {
    fn info(&self) -> TerminalSessionInfo {
        TerminalSessionInfo {
            session_id: self.id.clone(),
            cwd: self.cwd.clone(),
            issue_id: self.issue_id.clone(),
            cols: self.cols,
            rows: self.rows,
        }
    }

    fn terminate(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn validate_cwd(cwd: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(cwd);
    if !path.exists() {
        return Err(format!("Terminal cwd does not exist: {}", cwd));
    }
    if !path.is_dir() {
        return Err(format!("Terminal cwd is not a directory: {}", cwd));
    }
    path.canonicalize()
        .map_err(|e| format!("Failed to resolve terminal cwd {}: {}", cwd, e))
}

fn default_shell() -> String {
    if cfg!(target_os = "windows") {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn spawn_reader_thread<S: TerminalEventSink>(
    session_id: String,
    mut reader: Box<dyn Read + Send>,
    sink: S,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];

        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    sink.emit(TerminalEvent {
                        session_id: session_id.clone(),
                        kind: "exit".to_string(),
                        data: None,
                        message: None,
                        code: None,
                    });
                    break;
                }
                Ok(n) => {
                    sink.emit(TerminalEvent {
                        session_id: session_id.clone(),
                        kind: "data".to_string(),
                        data: Some(String::from_utf8_lossy(&buffer[..n]).to_string()),
                        message: None,
                        code: None,
                    });
                }
                Err(e) => {
                    log::error!("[terminal] Reader error for {}: {}", session_id, e);
                    sink.emit(TerminalEvent {
                        session_id: session_id.clone(),
                        kind: "error".to_string(),
                        data: None,
                        message: Some(e.to_string()),
                        code: None,
                    });
                    break;
                }
            }
        }
    });
}

#[tauri::command]
pub async fn terminal_create(
    request: TerminalCreateRequest,
    app: tauri::AppHandle,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<TerminalSessionInfo, String> {
    log::info!("[terminal] Creating session in {}", request.cwd);
    manager
        .create_session(request, TauriTerminalEventSink::new(app))
        .map_err(|e| {
            log::error!("[terminal] Create failed: {}", e);
            e
        })
}

#[tauri::command]
pub async fn terminal_write(
    request: TerminalWriteRequest,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), String> {
    manager
        .write(&request.session_id, &request.data)
        .map_err(|e| {
            log::error!("[terminal] Write failed for {}: {}", request.session_id, e);
            e
        })
}

#[tauri::command]
pub async fn terminal_resize(
    request: TerminalResizeRequest,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<TerminalSessionInfo, String> {
    manager
        .resize(&request.session_id, request.cols, request.rows)
        .map_err(|e| {
            log::error!("[terminal] Resize failed for {}: {}", request.session_id, e);
            e
        })
}

#[tauri::command]
pub async fn terminal_restart(
    request: TerminalSessionRequest,
    app: tauri::AppHandle,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<TerminalSessionInfo, String> {
    manager
        .restart(&request.session_id, TauriTerminalEventSink::new(app))
        .map_err(|e| {
            log::error!(
                "[terminal] Restart failed for {}: {}",
                request.session_id,
                e
            );
            e
        })
}

#[tauri::command]
pub async fn terminal_close(
    request: TerminalSessionRequest,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), String> {
    manager.close(&request.session_id).map_err(|e| {
        log::error!("[terminal] Close failed for {}: {}", request.session_id, e);
        e
    })
}

#[tauri::command]
pub async fn terminal_list(
    manager: tauri::State<'_, TerminalManager>,
) -> Result<Vec<TerminalSessionInfo>, String> {
    manager.list().map_err(|e| {
        log::error!("[terminal] List failed: {}", e);
        e
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::time::{Duration, Instant};

    #[derive(Clone)]
    struct TestSink {
        tx: Sender<TerminalEvent>,
    }

    impl TerminalEventSink for TestSink {
        fn emit(&self, event: TerminalEvent) {
            let _ = self.tx.send(event);
        }
    }

    fn test_sink() -> (TestSink, Receiver<TerminalEvent>) {
        let (tx, rx) = channel();
        (TestSink { tx }, rx)
    }

    fn test_shell() -> String {
        std::env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "powershell.exe".to_string()
            } else {
                "/bin/sh".to_string()
            }
        })
    }

    fn read_until(rx: &Receiver<TerminalEvent>, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = String::new();

        while Instant::now() < deadline {
            if let Ok(event) = rx.recv_timeout(Duration::from_millis(100)) {
                if event.kind == "data" {
                    if let Some(data) = event.data {
                        output.push_str(&data);
                        if output.contains(needle) {
                            return output;
                        }
                    }
                }
            }
        }

        output
    }

    #[test]
    fn create_session_starts_shell_in_requested_cwd_and_streams_output() {
        let test_dir =
            std::env::temp_dir().join(format!("borabr-terminal-test-{}", std::process::id()));
        fs::create_dir_all(&test_dir).unwrap();

        let (sink, rx) = test_sink();
        let manager = TerminalManager::default();
        let cwd = test_dir.to_string_lossy().to_string();

        let session = manager
            .create_session(
                TerminalCreateRequest {
                    cwd: cwd.clone(),
                    issue_id: Some("borabr-test".to_string()),
                    shell: Some(test_shell()),
                    cols: Some(80),
                    rows: Some(24),
                },
                sink,
            )
            .unwrap();

        manager.write(&session.session_id, "pwd\r").unwrap();
        let output = read_until(&rx, &cwd);
        let _ = manager.close(&session.session_id);

        assert!(
            output.contains(&cwd),
            "expected terminal output to contain cwd {cwd:?}, got {output:?}"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn resize_updates_pty_size_visible_to_shell() {
        let test_dir = std::env::temp_dir().join(format!(
            "borabr-terminal-resize-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&test_dir).unwrap();

        let (sink, rx) = test_sink();
        let manager = TerminalManager::default();

        let session = manager
            .create_session(
                TerminalCreateRequest {
                    cwd: test_dir.to_string_lossy().to_string(),
                    issue_id: None,
                    shell: Some(test_shell()),
                    cols: Some(80),
                    rows: Some(24),
                },
                sink,
            )
            .unwrap();

        manager.resize(&session.session_id, 100, 40).unwrap();
        manager.write(&session.session_id, "stty size\r").unwrap();
        let output = read_until(&rx, "40 100");
        let _ = manager.close(&session.session_id);

        assert!(
            output.contains("40 100"),
            "expected resized PTY size in output, got {output:?}"
        );
    }

    #[test]
    fn restart_replaces_shell_but_keeps_session_identity() {
        let test_dir = std::env::temp_dir().join(format!(
            "borabr-terminal-restart-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&test_dir).unwrap();

        let (sink, rx) = test_sink();
        let manager = TerminalManager::default();
        let cwd = test_dir.to_string_lossy().to_string();

        let session = manager
            .create_session(
                TerminalCreateRequest {
                    cwd,
                    issue_id: Some("borabr-restart".to_string()),
                    shell: Some(test_shell()),
                    cols: Some(80),
                    rows: Some(24),
                },
                sink.clone(),
            )
            .unwrap();

        let restarted = manager.restart(&session.session_id, sink).unwrap();
        assert_eq!(restarted.session_id, session.session_id);
        assert_eq!(restarted.issue_id.as_deref(), Some("borabr-restart"));

        manager
            .write(&session.session_id, "echo restarted\r")
            .unwrap();
        let output = read_until(&rx, "restarted");
        let _ = manager.close(&session.session_id);

        assert!(
            output.contains("restarted"),
            "expected restarted shell output, got {output:?}"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn session_environment_includes_project_and_issue_context() {
        let test_dir =
            std::env::temp_dir().join(format!("borabr-terminal-env-test-{}", std::process::id()));
        fs::create_dir_all(&test_dir).unwrap();

        let (sink, rx) = test_sink();
        let manager = TerminalManager::default();
        let cwd = test_dir.to_string_lossy().to_string();

        let session = manager
            .create_session(
                TerminalCreateRequest {
                    cwd: cwd.clone(),
                    issue_id: Some("borabr-env".to_string()),
                    shell: Some(test_shell()),
                    cols: Some(80),
                    rows: Some(24),
                },
                sink,
            )
            .unwrap();

        manager
            .write(
                &session.session_id,
                "printf '%s|%s\\n' \"$BEADS_PATH\" \"$BORABR_ISSUE_ID\"\r",
            )
            .unwrap();
        let output = read_until(&rx, "borabr-env");
        let _ = manager.close(&session.session_id);

        assert!(
            output.contains(&format!("{cwd}|borabr-env")),
            "expected project and issue context in output, got {output:?}"
        );
    }

    #[test]
    fn close_removes_session_and_rejects_later_writes() {
        let test_dir =
            std::env::temp_dir().join(format!("borabr-terminal-close-test-{}", std::process::id()));
        fs::create_dir_all(&test_dir).unwrap();

        let (sink, _rx) = test_sink();
        let manager = TerminalManager::default();

        let session = manager
            .create_session(
                TerminalCreateRequest {
                    cwd: test_dir.to_string_lossy().to_string(),
                    issue_id: None,
                    shell: Some(test_shell()),
                    cols: Some(80),
                    rows: Some(24),
                },
                sink,
            )
            .unwrap();

        assert_eq!(manager.list().unwrap().len(), 1);
        manager.close(&session.session_id).unwrap();
        assert!(manager.list().unwrap().is_empty());
        assert!(manager
            .write(&session.session_id, "echo after close\r")
            .is_err());
    }
}
