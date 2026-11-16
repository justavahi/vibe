use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The session used when a directory has no sessions yet.
pub const DEFAULT_SESSION: &str = "session";

/// Length of the random nickname given to every session.
const NICKNAME_LEN: usize = 8;

/// A single message stored in a session's history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// `user` or `assistant`.
    pub role: String,
    pub content: String,
}

/// Metadata stored alongside a session's history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    /// Random 8-character nickname assigned when the session was created.
    #[serde(default)]
    pub nickname: String,
    /// Unix timestamp (seconds) of creation.
    #[serde(default)]
    pub created: u64,
    /// Unix timestamp (seconds) of the last message.
    #[serde(default)]
    pub last_used: u64,
    /// The model that was selected when the session was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// The session store for the current working directory.
///
/// Sessions live in two places at once:
/// - locally in `./.vibe/sessions/<name>/files`
/// - globally in `~/.vibe/dirs/<dir>/sessions/<name>/files`
///
/// where `<dir>` is the current directory's path with `$HOME` replaced by `~`
/// and every `/` replaced by `-`.
pub struct Store {
    /// The sanitized directory name used under `~/.vibe/dirs` and `~/.vibe/trash`.
    pub dir_name: String,
    local_root: PathBuf,
    local_sessions: PathBuf,
    global_sessions: PathBuf,
    trash_sessions: PathBuf,
}

impl Store {
    /// Builds a store rooted at the current working directory.
    pub fn new() -> io::Result<Self> {
        let cwd = env::current_dir()?;
        let dir_name = sanitize_dir_name(&cwd);

        let local_root = cwd.join(".vibe");
        let local_sessions = local_root.join("sessions");
        let global_sessions = vibe_home().join("dirs").join(&dir_name).join("sessions");
        let trash_sessions = vibe_home().join("trash").join(&dir_name).join("sessions");

        Ok(Self {
            dir_name,
            local_root,
            local_sessions,
            global_sessions,
            trash_sessions,
        })
    }

    /// Records which session is currently active for this directory.
    pub fn set_current(&self, name: &str) -> io::Result<()> {
        fs::create_dir_all(&self.local_root)?;
        fs::write(self.local_root.join("current"), name)
    }

    /// Returns the currently active session for this directory, if any.
    pub fn current(&self) -> Option<String> {
        fs::read_to_string(self.local_root.join("current"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Clears the current-session pointer if it refers to `name`.
    fn clear_current_if(&self, name: &str) {
        if self.current().as_deref() == Some(name) {
            let _ = fs::remove_file(self.local_root.join("current"));
        }
    }

    /// Creates the local and global session directories if missing.
    pub fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(&self.local_sessions)?;
        fs::create_dir_all(&self.global_sessions)?;
        Ok(())
    }

    /// Lists the session names that exist for this directory, sorted.
    pub fn list(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();

        if self.local_sessions.exists() {
            for entry in fs::read_dir(&self.local_sessions)? {
                let entry = entry?;
                if entry.file_type()?.is_dir()
                    && let Some(name) = entry.file_name().to_str()
                {
                    names.push(name.to_string());
                }
            }
        }

        names.sort();
        Ok(names)
    }

    /// Whether a session with this name exists locally.
    pub fn exists(&self, name: &str) -> bool {
        self.local_sessions.join(name).is_dir()
    }

    /// Creates a session in both the local and global stores.
    ///
    /// Fails if the name is invalid or already exists.
    pub fn create(&self, name: &str, model: Option<&str>) -> io::Result<()> {
        self.validate_name(name)?;

        if self.exists(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("session `{name}` already exists"),
            ));
        }

        fs::create_dir_all(self.local_files(name))?;
        fs::create_dir_all(self.global_files(name))?;

        let now = now_secs();
        let meta = Meta {
            nickname: random_nickname(),
            created: now,
            last_used: now,
            model: model.map(str::to_string),
        };

        self.write_meta(name, &meta)?;
        self.save_history(name, &[])?;
        Ok(())
    }

    /// Creates the session only if it does not already exist.
    pub fn ensure_session(&self, name: &str, model: Option<&str>) -> io::Result<()> {
        if self.exists(name) {
            return Ok(());
        }
        self.create(name, model)
    }

    /// Renames a session in both the local and global stores.
    pub fn rename(&self, old: &str, new: &str) -> io::Result<()> {
        self.validate_name(new)?;

        if !self.exists(old) {
            return Err(not_found(old));
        }
        if self.exists(new) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("session `{new}` already exists"),
            ));
        }

        fs::rename(self.local_session(old), self.local_session(new))?;

        if self.global_session(old).exists() {
            if let Some(parent) = self.global_session(new).parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(self.global_session(old), self.global_session(new))?;
        }

        // Keep the current-session pointer valid across the rename.
        if self.current().as_deref() == Some(old) {
            self.set_current(new)?;
        }

        Ok(())
    }

    /// Removes a session: deletes it locally and moves the global copy to trash.
    pub fn remove(&self, name: &str) -> io::Result<()> {
        let local = self.local_session(name);
        let global = self.global_session(name);

        if !local.exists() && !global.exists() {
            return Err(not_found(name));
        }

        if local.exists() {
            fs::remove_dir_all(&local)?;
        }

        if global.exists() {
            fs::create_dir_all(&self.trash_sessions)?;
            let dest = self.trash_sessions.join(name);
            if dest.exists() {
                fs::remove_dir_all(&dest)?;
            }
            fs::rename(&global, &dest)?;
        }

        self.clear_current_if(name);
        Ok(())
    }

    /// Empties this directory's trash (`~/.vibe/trash/<dir>/sessions`).
    pub fn clean(&self) -> io::Result<()> {
        if self.trash_sessions.exists() {
            fs::remove_dir_all(&self.trash_sessions)?;
        }
        Ok(())
    }

    /// Deletes a session completely, from both the local and global stores.
    pub fn delete(&self, name: &str) -> io::Result<()> {
        let local = self.local_session(name);
        let global = self.global_session(name);

        if !local.exists() && !global.exists() {
            return Err(not_found(name));
        }

        if local.exists() {
            fs::remove_dir_all(&local)?;
        }
        if global.exists() {
            fs::remove_dir_all(&global)?;
        }

        self.clear_current_if(name);
        Ok(())
    }

    /// Loads a session's history, returning an empty list if there is none.
    pub fn load_history(&self, name: &str) -> io::Result<Vec<HistoryEntry>> {
        let path = self.history_path_local(name);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let contents = fs::read_to_string(&path)?;
        serde_json::from_str(&contents).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse {}: {e}", path.display()),
            )
        })
    }

    /// Writes a session's history to both the local and global stores.
    pub fn save_history(&self, name: &str, history: &[HistoryEntry]) -> io::Result<()> {
        let json = serde_json::to_string_pretty(history).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize history: {e}"),
            )
        })?;

        fs::create_dir_all(self.local_files(name))?;
        fs::create_dir_all(self.global_files(name))?;

        fs::write(self.history_path_local(name), &json)?;
        fs::write(self.global_files(name).join("history.json"), &json)?;

        let mut meta = self.read_meta(name)?;
        meta.last_used = now_secs();
        self.write_meta(name, &meta)?;

        Ok(())
    }

    /// Reads a session's metadata (from the local copy).
    pub fn read_meta(&self, name: &str) -> io::Result<Meta> {
        let path = self.local_files(name).join("meta.json");
        if !path.exists() {
            return Ok(Meta::default());
        }

        let contents = fs::read_to_string(&path)?;
        serde_json::from_str(&contents).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse {}: {e}", path.display()),
            )
        })
    }

    fn write_meta(&self, name: &str, meta: &Meta) -> io::Result<()> {
        let json = serde_json::to_string_pretty(meta).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize meta: {e}"),
            )
        })?;

        fs::write(self.local_files(name).join("meta.json"), &json)?;
        fs::write(self.global_files(name).join("meta.json"), &json)?;
        Ok(())
    }

    fn validate_name(&self, name: &str) -> io::Result<()> {
        let invalid = name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name == "."
            || name == "..";

        if invalid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid session name `{name}`"),
            ));
        }

        Ok(())
    }

    fn local_session(&self, name: &str) -> PathBuf {
        self.local_sessions.join(name)
    }

    fn global_session(&self, name: &str) -> PathBuf {
        self.global_sessions.join(name)
    }

    fn local_files(&self, name: &str) -> PathBuf {
        self.local_session(name).join("files")
    }

    fn global_files(&self, name: &str) -> PathBuf {
        self.global_session(name).join("files")
    }

    fn history_path_local(&self, name: &str) -> PathBuf {
        self.local_files(name).join("history.json")
    }
}

/// The root of vibe's global state: `~/.vibe`.
fn vibe_home() -> PathBuf {
    home().join(".vibe")
}

/// The user's home directory, falling back to the current directory.
fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Turns a directory path into a single valid folder name.
///
/// `$HOME` is replaced by `~` and every `/` by `-`, so `/home/you/a/b` becomes
/// `~-a-b`.
pub fn sanitize_dir_name(path: &Path) -> String {
    let text = match path.strip_prefix(home()) {
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    };

    text.replace('/', "-")
}

/// A random nickname of `NICKNAME_LEN` lowercase alphanumeric characters.
pub fn random_nickname() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

    let mut bytes = [0u8; NICKNAME_LEN];

    let filled = fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok();

    if !filled {
        // Fallback when /dev/urandom is unavailable: a simple time-seeded PRNG.
        let mut seed = now_secs().wrapping_mul(1_000_000_007).wrapping_add(1);
        for byte in &mut bytes {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *byte = (seed >> 33) as u8;
        }
    }

    bytes
        .iter()
        .map(|b| CHARS[(*b as usize) % CHARS.len()] as char)
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn not_found(name: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("session `{name}` not found"))
}
