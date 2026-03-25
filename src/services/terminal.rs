use portable_pty::{native_pty_system, CommandBuilder, PtySize, MasterPty, Child};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// Maximum number of concurrent terminal sessions.
const MAX_SESSIONS: usize = 10;

/// Handle stored for each active terminal session.
/// The reader is handed off to the WebSocket task at creation time;
/// we keep the writer (for keystrokes), master (for resize), and child.
pub struct TerminalSessionHandle {
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub master: Box<dyn MasterPty + Send>,
}

/// Global manager for PTY terminal sessions.
pub struct TerminalManager {
    sessions: Mutex<HashMap<String, TerminalSessionHandle>>,
}

/// Singleton accessor for the global TerminalManager.
pub fn manager() -> &'static TerminalManager {
    static INSTANCE: OnceLock<TerminalManager> = OnceLock::new();
    INSTANCE.get_or_init(|| TerminalManager {
        sessions: Mutex::new(HashMap::new()),
    })
}

/// Detect the user's default shell.
///
/// - Windows: `pwsh` (PowerShell 7) → `%COMSPEC%` → `cmd.exe`
/// - macOS:   `$SHELL` → `/bin/zsh`
/// - Linux:   `$SHELL` → `/bin/bash`
/// - Other:   returns an error message asking users to open an issue
fn default_shell() -> Result<String, String> {
    if cfg!(target_os = "windows") {
        // Prefer PowerShell 7+ (pwsh) if available, then COMSPEC, then cmd.exe
        if which_exists("pwsh") {
            Ok("pwsh".to_string())
        } else {
            Ok(std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string()))
        }
    } else if cfg!(target_os = "macos") {
        Ok(std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string()))
    } else if cfg!(target_os = "linux") || cfg!(target_os = "freebsd") {
        Ok(std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()))
    } else {
        Err(format!(
            "Unsupported platform: {} {}. \
             Terminal sessions are not yet supported on this OS. \
             Please open an issue at https://github.com/getforma-dev/kmd/issues \
             so we can add support.",
            std::env::consts::OS,
            std::env::consts::ARCH,
        ))
    }
}

/// Check if a command exists on PATH.
fn which_exists(cmd: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("where")
            .arg(cmd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("which")
            .arg(cmd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

impl TerminalManager {
    /// Create a new PTY session.
    ///
    /// Returns `(session_id, reader)` where the reader produces raw terminal
    /// output bytes.  The caller (WebSocket handler) owns the reader and
    /// forwards data to the client.
    pub fn create_session(
        &self,
        cwd: &Path,
        cols: u16,
        rows: u16,
    ) -> Result<(String, Box<dyn Read + Send>), String> {
        {
            let sessions = self.sessions.lock().expect("terminal sessions mutex poisoned");
            if sessions.len() >= MAX_SESSIONS {
                return Err(format!(
                    "Maximum number of terminal sessions ({MAX_SESSIONS}) reached. Kill an existing session first."
                ));
            }
        }

        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to open PTY: {e}"))?;

        let shell = default_shell()?;
        let mut cmd = CommandBuilder::new(&shell);
        // Start an interactive login shell (Unix only — Windows shells don't use -l)
        if !cfg!(target_os = "windows") {
            cmd.arg("-l");
        }
        cmd.cwd(cwd);

        // Spawn the child on the slave side
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to spawn shell ({shell}): {e}"))?;

        // Must drop slave after spawn so reads on master see EOF when child exits
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone PTY reader: {e}"))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to take PTY writer: {e}"))?;

        let session_id = Uuid::new_v4().to_string();

        let handle = TerminalSessionHandle {
            writer,
            child,
            master: pair.master,
        };

        self.sessions
            .lock()
            .expect("terminal sessions mutex poisoned")
            .insert(session_id.clone(), handle);

        tracing::info!("Terminal session created: {session_id} (shell={shell}, cwd={})", cwd.display());

        Ok((session_id, reader))
    }

    /// Write keystrokes (or any raw bytes) into a session's PTY.
    pub fn write_to_session(&self, id: &str, data: &[u8]) -> Result<(), String> {
        let mut sessions = self.sessions.lock().expect("terminal sessions mutex poisoned");
        let handle = sessions
            .get_mut(id)
            .ok_or_else(|| format!("Terminal session not found: {id}"))?;

        handle
            .writer
            .write_all(data)
            .map_err(|e| format!("Write to PTY failed: {e}"))?;
        handle
            .writer
            .flush()
            .map_err(|e| format!("Flush PTY failed: {e}"))?;

        Ok(())
    }

    /// Resize a session's PTY to new dimensions.
    pub fn resize_session(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let sessions = self.sessions.lock().expect("terminal sessions mutex poisoned");
        let handle = sessions
            .get(id)
            .ok_or_else(|| format!("Terminal session not found: {id}"))?;

        handle
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Resize PTY failed: {e}"))?;

        Ok(())
    }

    /// Kill a terminal session and remove it from the manager.
    pub fn kill_session(&self, id: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().expect("terminal sessions mutex poisoned");
        let mut handle = sessions
            .remove(id)
            .ok_or_else(|| format!("Terminal session not found: {id}"))?;

        // Attempt to kill the child process; ignore errors if already dead
        let _ = handle.child.kill();
        tracing::info!("Terminal session killed: {id}");
        Ok(())
    }

    /// List all active session IDs.
    pub fn list_sessions(&self) -> Vec<String> {
        self.sessions
            .lock()
            .expect("terminal sessions mutex poisoned")
            .keys()
            .cloned()
            .collect()
    }
}
