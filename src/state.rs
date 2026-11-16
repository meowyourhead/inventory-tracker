//! Core desktop backend: data file access, cross-machine edit lock,
//! auto-backups and events. This is a direct port of the Electron main process
//! logic (see ../../electron/main.js) so the existing HTML UI keeps working.

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
        mpsc::Sender,
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};
use tao::event_loop::EventLoopProxy;

pub const BASE_W: f64 = 1496.0;
pub const BASE_H: f64 = 946.0;
pub const MIN_W: f64 = 1080.0;
pub const MIN_H: f64 = 720.0;

/// A lock whose owner has shown no activity for this long is considered
/// abandoned. The heartbeat is only renewed while someone is actually using
/// the app, so an open-but-idle session releases edit rights after this.
const STALE_MS: i64 = 30_000;

#[derive(Debug, Clone)]
pub enum AppEvent {
    Push {
        channel: &'static str,
        payload: Value,
    },
    Resize {
        factor: f64,
        reply: Sender<Value>,
    },
    Window {
        command: &'static str,
        reply: Option<Sender<Value>>,
    },
    WindowResize {
        direction: tao::window::ResizeDirection,
        reply: Sender<Value>,
    },
    Show,
    /// The executable was replaced by an update: clean up and relaunch.
    Restart,
}

/// Parsed contents of `inventory.json`, keyed by the file's mtime and size.
/// The value is shared behind an `Arc` so readers never deep-clone the
/// (multi-megabyte) document.
struct CachedData {
    mtime: Option<SystemTime>,
    size: u64,
    value: Arc<Value>,
}

#[derive(Default)]
struct Store {
    cache: Option<CachedData>,
}

impl Store {
    fn set(&mut self, meta: Option<(Option<SystemTime>, u64)>, value: Arc<Value>) {
        let (mtime, size) = meta.unwrap_or((None, 0));
        self.cache = Some(CachedData { mtime, size, value });
    }

    fn matches(&self, meta: Option<(Option<SystemTime>, u64)>) -> bool {
        matches!(
            (&self.cache, meta),
            (Some(cached), Some((mtime, size))) if cached.mtime == mtime && cached.size == size
        )
    }
}

#[derive(serde::Serialize)]
struct LoadResponse<'a> {
    ok: bool,
    data: &'a Value,
    status: Value,
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub data_file: PathBuf,
    pub lock_file: PathBuf,
    pub backups_dir: PathBuf,
    pub user: String,
    pub pid: u32,
    /// Identifies this process across computers. A PID alone is not unique
    /// on a shared data folder: two PCs can run the app under the same PID.
    session: String,
    shared_data_mode: bool,
    store: Mutex<Store>,
    did_save: AtomicBool,
    heartbeat_gen: AtomicU64,
    pub hwnd: AtomicIsize,
    proxy: Mutex<EventLoopProxy<AppEvent>>,
    /// Set while an update is downloading/installing (one at a time).
    updating: AtomicBool,
    /// Version this process was updated from, when started by the updater.
    pub updated_from: Mutex<Option<String>>,
}

impl AppState {
    pub fn new(proxy: EventLoopProxy<AppEvent>) -> Self {
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        // Shared mode (several PCs, one dataset): the data folder comes from
        // INVENTORY_DATA_DIR or from `dataDir` in inventory.config.json beside
        // the exe. Otherwise portable mode: data next to the executable.
        let shared_dir = env::var_os("INVENTORY_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| configured_data_dir(&exe_dir));
        let shared_data_mode = shared_dir.is_some();
        let data_dir = shared_dir.unwrap_or(exe_dir);

        let data_file = data_dir.join("inventory.json");
        let lock_file = data_dir.join("inventory.lock.json");
        let backups_dir = data_dir.join("AutoBackups");

        let user = env::var("USERNAME")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| env::var("USER").ok().filter(|v| !v.trim().is_empty()))
            .unwrap_or_else(|| "User".to_string());

        let pid = std::process::id();
        let host = env::var("COMPUTERNAME").unwrap_or_default();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session = format!("{host}-{pid}-{nanos:x}");

        Self {
            data_dir,
            data_file,
            lock_file,
            backups_dir,
            user,
            pid,
            session,
            shared_data_mode,
            store: Mutex::new(Store::default()),
            did_save: AtomicBool::new(false),
            heartbeat_gen: AtomicU64::new(0),
            hwnd: AtomicIsize::new(0),
            proxy: Mutex::new(proxy),
            updating: AtomicBool::new(false),
            updated_from: Mutex::new(None),
        }
    }

    pub fn empty_data() -> Value {
        json!({
            "items": [],
            "transactions": [],
            "settings": {
                "theme": "glass",
                "mode": "light",
                "font": "sfpro",
                "fontSize": 14,
                "quickLogoutMode": "fefo",
                "appName": "Inventory"
            }
        })
    }

    /// Mirrors main.js ensureDataDir: in shared mode an unreachable or
    /// unwritable data folder must fail loudly instead of silently starting a
    /// divergent local dataset.
    pub fn ensure_ready(&self) -> Result<(), String> {
        if self.shared_data_mode {
            fs::create_dir_all(&self.data_dir).map_err(|e| {
                format!(
                    "Cannot reach the shared inventory data folder:\n{}\n\n{e}\n\nCheck that the network share is available and try again.",
                    self.data_dir.display()
                )
            })?;
            let probe = self.data_dir.join(".inventory-write-test");
            match fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&probe)
            {
                Ok(mut f) => {
                    let _ = f.write_all(b"test");
                    let _ = fs::remove_file(&probe);
                }
                Err(e) => {
                    return Err(format!(
                        "Cannot write to the shared inventory data folder:\n{}\n\n{e}\n\nCheck the network share permissions and try again.",
                        self.data_dir.display()
                    ));
                }
            }
        } else {
            fs::create_dir_all(&self.data_dir).map_err(|e| {
                format!(
                    "Cannot create the inventory data folder:\n{}\n\n{e}",
                    self.data_dir.display()
                )
            })?;
        }

        if !self.data_file.exists() {
            write_json_atomic(&self.data_file, &Self::empty_data())
                .map_err(|e| format!("Cannot create {}:\n{e}", self.data_file.display()))?;
        }
        // Best effort: make the configured backup folder visible/ready. A
        // missing network path must not stop the app from opening.
        let _ = fs::create_dir_all(self.backup_dir());
        Ok(())
    }

    /// Warm the parsed-data cache before the window appears.
    pub fn prewarm(&self) {
        let _ = self.load_data();
    }

    pub fn set_hwnd(&self, hwnd: isize) {
        self.hwnd.store(hwnd, Ordering::Relaxed);
    }

    pub fn show(&self) {
        self.send_event(AppEvent::Show);
    }

    fn send_event(&self, event: AppEvent) {
        if let Ok(proxy) = self.proxy.lock() {
            let _ = proxy.send_event(event);
        }
    }

    pub fn push(&self, channel: &'static str, payload: Value) {
        self.send_event(AppEvent::Push { channel, payload });
    }

    pub fn request_resize(&self, factor: f64) -> Value {
        let (tx, rx) = std::sync::mpsc::channel();
        self.send_event(AppEvent::Resize { factor, reply: tx });
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| json!({ "ok": false }))
    }

    fn request_window(&self, command: &'static str) -> Value {
        let (tx, rx) = std::sync::mpsc::channel();
        self.send_event(AppEvent::Window {
            command,
            reply: Some(tx),
        });
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| json!({ "ok": false }))
    }

    pub fn window_state(&self) -> Value {
        self.request_window("state")
    }

    pub fn window_minimize(&self) -> Value {
        self.request_window("minimize")
    }

    pub fn window_toggle_maximize(&self) -> Value {
        self.request_window("toggle-maximize")
    }

    pub fn window_close(&self) -> Value {
        self.request_window("close")
    }

    pub fn window_resize(&self, direction: tao::window::ResizeDirection) -> Value {
        let (tx, rx) = std::sync::mpsc::channel();
        self.send_event(AppEvent::WindowResize {
            direction,
            reply: tx,
        });
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| json!({ "ok": false }))
    }

    pub fn window_drag(&self) {
        self.send_event(AppEvent::Window {
            command: "drag",
            reply: None,
        });
    }

    // ---------------------------------------------------------------- data --

    fn metadata(&self) -> Option<(Option<SystemTime>, u64)> {
        fs::metadata(&self.data_file)
            .ok()
            .map(|m| (m.modified().ok(), m.len()))
    }

    /// Locks the data cache, recovering from poisoning: a panic in one API
    /// thread must not make every later request panic too.
    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn load_data(&self) -> Arc<Value> {
        let mut store = self.store();
        let meta = self.metadata();

        if store.matches(meta) {
            if let Some(cached) = &store.cache {
                return Arc::clone(&cached.value);
            }
        }
        // File disappeared: fall through and return empty data, same as
        // readData() -> fallback in the Electron process.
        let parsed = fs::read(&self.data_file)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok());

        match parsed {
            Some(value) => {
                let value = Arc::new(value);
                if meta.is_some() {
                    store.set(meta, Arc::clone(&value));
                }
                value
            }
            None => Arc::new(Self::empty_data()),
        }
    }

    /// Serialized `load` reply. Writes the cached document straight into the
    /// response body instead of cloning it into a temporary `Value`.
    pub fn load_response_bytes(&self) -> Vec<u8> {
        let data = self.load_data();
        let reply = LoadResponse {
            ok: true,
            data: &data,
            status: self.status_value(),
        };
        serde_json::to_vec(&reply).unwrap_or_else(|_| {
            br#"{"ok":false,"error":"Could not encode response"}"#.to_vec()
        })
    }

    pub fn save(&self, data: Value) -> Value {
        if !self.owned() {
            return json!({
                "ok": false,
                "error": "Editing lock is not owned by this session",
                "status": self.status_value()
            });
        }
        if let Err(e) = write_json_atomic(&self.data_file, &data) {
            return json!({ "ok": false, "error": e.to_string() });
        }
        // A freshly chosen backup folder should exist (and show as available)
        // as soon as the setting is saved.
        if let Some(dir) = backup_dir_setting(&data) {
            let _ = fs::create_dir_all(dir);
        }
        let meta = self.metadata();
        self.store().set(meta, Arc::new(data));
        self.did_save.store(true, Ordering::SeqCst);
        json!({ "ok": true })
    }

    // ---------------------------------------------------------------- lock --

    fn lock_info(&self) -> Option<Value> {
        let raw = fs::read_to_string(&self.lock_file).ok()?;
        let value: Value = serde_json::from_str(&raw).ok()?;
        if self.is_mine(&value) {
            return Some(value);
        }
        let heartbeat = value
            .get("heartbeat")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as i64;
        if now_ms() - heartbeat > STALE_MS {
            // Only delete the lock we judged stale: if another computer has
            // rewritten it in the meantime, leave its fresh lock alone.
            if fs::read_to_string(&self.lock_file).is_ok_and(|now| now == raw) {
                let _ = fs::remove_file(&self.lock_file);
            }
            return None;
        }
        Some(value)
    }

    fn is_mine(&self, lock: &Value) -> bool {
        lock.get("session").and_then(Value::as_str) == Some(self.session.as_str())
    }

    fn new_lock(&self) -> Value {
        json!({
            "pid": self.pid,
            "session": self.session,
            "user": self.user,
            "startedAt": iso_now(),
            "heartbeat": now_ms()
        })
    }

    /// Creates the lock file only if it does not exist yet. `create_new` is
    /// atomic on local disks and SMB shares, so two computers that both saw
    /// "no lock" cannot both end up believing they own it.
    fn create_lock_exclusive(&self) -> std::io::Result<()> {
        let body = serde_json::to_vec_pretty(&self.new_lock())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.lock_file)?;
        file.write_all(&body)?;
        file.sync_all()
    }

    pub fn owned(&self) -> bool {
        self.lock_info().is_some_and(|lock| self.is_mine(&lock))
    }

    pub fn status_value(&self) -> Value {
        let lock = self.lock_info();
        let owned = lock.as_ref().is_some_and(|lock| self.is_mine(lock));
        json!({
            "owned": owned,
            "user": self.user,
            "lock": lock,
            "dataFile": self.data_file.to_string_lossy()
        })
    }

    pub fn acquire_lock(self: &Arc<Self>) -> Value {
        match self.lock_info() {
            Some(current) if !self.is_mine(&current) => {
                return json!({ "ok": false, "status": self.status_value() });
            }
            // Already ours: just refresh it.
            Some(_) => {
                let _ = write_json_atomic(&self.lock_file, &self.new_lock());
            }
            None => {
                if self.create_lock_exclusive().is_err() {
                    // Someone else created it first (or the share is not
                    // writable): report the current state instead of
                    // overwriting their lock.
                    return json!({ "ok": false, "status": self.status_value() });
                }
            }
        }
        self.start_heartbeat();
        let status = self.status_value();
        self.push("lock-change", status.clone());
        json!({ "ok": true, "status": status })
    }

    pub fn unlock(&self) -> Value {
        self.stop_heartbeat();
        if self.owned() {
            let _ = fs::remove_file(&self.lock_file);
        }
        let status = self.status_value();
        self.push("lock-change", status.clone());
        json!({ "ok": true, "status": status })
    }

    /// Called on real user activity (throttled by the UI). Renews the
    /// heartbeat only while this session owns the lock.
    pub fn activity(&self) -> Value {
        match self.lock_info() {
            Some(mut lock) if self.is_mine(&lock) => {
                if let Some(obj) = lock.as_object_mut() {
                    obj.insert("heartbeat".into(), json!(now_ms()));
                }
                let _ = write_json_atomic(&self.lock_file, &lock);
            }
            _ => {
                let status = self.status_value();
                self.push("lock-change", status);
            }
        }
        Value::Bool(true)
    }

    fn start_heartbeat(self: &Arc<Self>) {
        let generation = self.heartbeat_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let state = Arc::clone(self);
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(10));
            if state.heartbeat_gen.load(Ordering::SeqCst) != generation {
                break;
            }
            if !state.owned() {
                // Somebody else took over our idle lock: let the UI flip back
                // to read-only immediately.
                state.push("lock-change", state.status_value());
                break;
            }
        });
    }

    fn stop_heartbeat(&self) {
        self.heartbeat_gen.fetch_add(1, Ordering::SeqCst);
    }

    // ------------------------------------------------------------ backups --

    pub fn backup(&self) -> Value {
        let default_name = format!(
            "inventory-backup-{}.json",
            chrono::Local::now().format("%Y-%m-%d")
        );
        let mut dialog = rfd::FileDialog::new()
            .set_title("Back up Inventory JSON")
            .set_file_name(&default_name)
            .add_filter("JSON", &["json"]);
        let backup_dir = self.backup_dir();
        if backup_dir.is_dir() {
            dialog = dialog.set_directory(&backup_dir);
        }
        if let Some(parent) = self.parent_window() {
            dialog = dialog.set_parent(&parent);
        }
        let Some(path) = dialog.save_file() else {
            return json!({ "canceled": true });
        };
        match fs::copy(&self.data_file, &path) {
            Ok(_) => json!({ "ok": true, "path": path.to_string_lossy() }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        }
    }

    /// Automatic backups go to `settings.backupDir` when the user picked one,
    /// otherwise to `AutoBackups` beside the data file.
    fn custom_backup_dir(&self) -> Option<PathBuf> {
        backup_dir_setting(&self.load_data())
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.custom_backup_dir()
            .unwrap_or_else(|| self.backups_dir.clone())
    }

    pub fn backup_status(&self) -> Value {
        let custom = self.custom_backup_dir();
        let is_default = custom.is_none();
        let dir = custom.unwrap_or_else(|| self.backups_dir.clone());
        json!({
            "ok": true,
            "path": dir.to_string_lossy(),
            "isDefault": is_default,
            "exists": dir.is_dir()
        })
    }

    pub fn choose_backup_dir(&self) -> Value {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose automatic backup folder");
        let current = self.backup_dir();
        if current.is_dir() {
            dialog = dialog.set_directory(&current);
        }
        if let Some(parent) = self.parent_window() {
            dialog = dialog.set_parent(&parent);
        }
        match dialog.pick_folder() {
            Some(path) => json!({ "ok": true, "path": path.to_string_lossy() }),
            None => json!({ "canceled": true }),
        }
    }

    pub fn open_backup_dir(&self) -> Value {
        let dir = self.backup_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            return json!({ "ok": false, "error": e.to_string() });
        }
        #[cfg(windows)]
        {
            match std::process::Command::new("explorer.exe").arg(&dir).spawn() {
                Ok(_) => json!({ "ok": true, "path": dir.to_string_lossy() }),
                Err(e) => json!({ "ok": false, "error": e.to_string() }),
            }
        }
        #[cfg(not(windows))]
        {
            json!({ "ok": false, "error": "Opening folders is only supported on Windows" })
        }
    }

    pub fn restore(&self) -> Value {
        // Same rule as `save`: only the lock owner may overwrite shared data.
        if !self.owned() {
            return json!({
                "ok": false,
                "error": "Editing lock is not owned by this session",
                "status": self.status_value()
            });
        }
        let mut dialog = rfd::FileDialog::new()
            .set_title("Restore Inventory JSON")
            .add_filter("JSON", &["json"]);
        if let Some(parent) = self.parent_window() {
            dialog = dialog.set_parent(&parent);
        }
        let Some(path) = dialog.pick_file() else {
            return json!({ "canceled": true });
        };
        let data = match fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        {
            Some(value)
                if value.get("items").is_some_and(Value::is_array)
                    && value.get("transactions").is_some_and(Value::is_array) =>
            {
                value
            }
            _ => {
                return json!({ "ok": false, "error": "Invalid inventory JSON" });
            }
        };
        if let Err(e) = write_json_atomic(&self.data_file, &data) {
            return json!({ "ok": false, "error": e.to_string() });
        }
        let meta = self.metadata();
        self.store().set(meta, Arc::new(data));
        // The UI reloads through `load` after a successful restore, so the
        // document is not echoed back here (it can be several megabytes).
        self.push("data", Value::Null);
        json!({ "ok": true, "path": path.to_string_lossy() })
    }

    fn parent_window(&self) -> Option<ParentWindow> {
        let hwnd = self.hwnd.load(Ordering::Relaxed);
        (hwnd != 0).then_some(ParentWindow(hwnd))
    }

    /// Runs before the process exits. A snapshot is only written if this
    /// session actually saved a change (same rule as the Electron app).
    pub fn auto_backup_on_close(&self) {
        if !self.did_save.load(Ordering::SeqCst) || !self.data_file.exists() {
            return;
        }
        let backup_dir = self.backup_dir();
        if fs::create_dir_all(&backup_dir).is_err() {
            return;
        }
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let user = sanitize_user(&self.user);
        let dest = backup_dir.join(format!("inventory-backup_{stamp}_by-{user}.json"));
        if fs::copy(&self.data_file, dest).is_ok() {
            self.prune_auto_backups(&backup_dir, 30);
        }
    }

    fn prune_auto_backups(&self, dir: &Path, keep: usize) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<(PathBuf, SystemTime)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("inventory-backup_") || !name.ends_with(".json") {
                    return None;
                }
                let modified = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(UNIX_EPOCH);
                Some((entry.path(), modified))
            })
            .collect();
        files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
        for (path, _) in files.into_iter().skip(keep) {
            let _ = fs::remove_file(path);
        }
    }

    // ------------------------------------------------------------ updates --

    /// Static facts for the Settings ▸ Updates card.
    pub fn update_info(&self) -> Value {
        let updated_from = self
            .updated_from
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        json!({
            "ok": true,
            "version": crate::update::CURRENT,
            "buildDate": crate::update::BUILD_DATE,
            "repo": crate::update::REPO,
            "channel": "GitHub Releases",
            "devBuild": cfg!(debug_assertions),
            "updatedFrom": updated_from,
            "updating": self.updating.load(Ordering::SeqCst)
        })
    }

    pub fn update_check(&self) -> Value {
        let current = crate::update::Version::current();
        match crate::update::latest_release() {
            Ok(release) => json!({
                "ok": true,
                "current": current.to_string(),
                "latest": release.version.to_string(),
                "available": release.version > current,
                "name": release.name,
                "notes": release.notes,
                "publishedAt": release.published_at,
                "url": release.page_url,
                "size": release.asset_size,
                "devBuild": cfg!(debug_assertions)
            }),
            Err(error) => json!({
                "ok": false,
                "current": current.to_string(),
                "error": error
            }),
        }
    }

    /// Downloads, verifies and installs the latest release, pushing
    /// `update-progress` events, then asks the event loop to restart.
    pub fn update_install(&self) -> Value {
        if self.updating.swap(true, Ordering::SeqCst) {
            return json!({ "ok": false, "error": "An update is already in progress" });
        }
        let result = (|| {
            let release = crate::update::latest_release()?;
            if release.version <= crate::update::Version::current() {
                return Err("This is already the latest version".to_string());
            }
            let latest = release.version.to_string();
            self.push(
                "update-progress",
                json!({ "stage": "download", "done": 0, "total": release.asset_size, "version": latest }),
            );
            crate::update::install(&release, &|done, total| {
                self.push(
                    "update-progress",
                    json!({ "stage": "download", "done": done, "total": total, "version": latest }),
                );
            })?;
            Ok(latest)
        })();
        match result {
            Ok(version) => {
                self.push("update-progress", json!({ "stage": "restart", "version": version }));
                // Let the reply and the "restarting" state reach the UI first.
                let proxy = self
                    .proxy
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(1200));
                    let _ = proxy.send_event(AppEvent::Restart);
                });
                json!({ "ok": true, "version": version })
            }
            Err(error) => {
                self.updating.store(false, Ordering::SeqCst);
                json!({ "ok": false, "error": error })
            }
        }
    }

    pub fn cleanup(&self) {
        self.auto_backup_on_close();
        self.stop_heartbeat();
        if self.owned() {
            let _ = fs::remove_file(&self.lock_file);
        }
    }

    // ------------------------------------------------------------- events --

    /// Watch the data folder rather than the file itself: on Windows the data
    /// file is replaced atomically, which can invalidate a watch on the file.
    pub fn start_watch(self: &Arc<Self>) {
        use notify::{RecursiveMode, Watcher};

        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(_) => return,
        };
        if watcher
            .watch(&self.data_dir, RecursiveMode::NonRecursive)
            .is_err()
        {
            return;
        }

        let state = Arc::clone(self);
        thread::spawn(move || {
            // Keep the watcher alive for the process lifetime.
            let _watcher = watcher;
            let is_data = |result: &notify::Result<notify::Event>| {
                result.as_ref().is_ok_and(|event| {
                    event
                        .paths
                        .iter()
                        .any(|p| p.file_name() == Some(std::ffi::OsStr::new("inventory.json")))
                })
            };
            while let Ok(result) = rx.recv() {
                if !is_data(&result) {
                    continue;
                }
                // One atomic replace produces several events (create, rename,
                // modify...). Let the burst settle, then notify the UI once:
                // each push makes a read-only client reload the whole file.
                thread::sleep(Duration::from_millis(150));
                while rx.try_recv().is_ok() {}
                // Our own save already refreshed the cache; skip the reload.
                if state.store().matches(state.metadata()) {
                    continue;
                }
                state.push("data", Value::Null);
            }
        });
    }
}

/// Small wrapper so native file dialogs are owned by the Inventory window.
struct ParentWindow(isize);

impl raw_window_handle::HasWindowHandle for ParentWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        use raw_window_handle::{RawWindowHandle, Win32WindowHandle, WindowHandle};
        let handle = std::num::NonZeroIsize::new(self.0)
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        let handle = Win32WindowHandle::new(handle);
        // SAFETY: the HWND belongs to the live main window, which outlives the
        // modal dialog call.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

impl raw_window_handle::HasDisplayHandle for ParentWindow {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        use raw_window_handle::{DisplayHandle, RawDisplayHandle, WindowsDisplayHandle};
        // SAFETY: no backend data is stored in the Windows display handle.
        Ok(unsafe {
            DisplayHandle::borrow_raw(RawDisplayHandle::Windows(WindowsDisplayHandle::new()))
        })
    }
}

// ---------------------------------------------------------------- helpers --

/// `dataDir` from `inventory.config.json` next to the executable. The file
/// is never touched by updates (only the exe is replaced).
fn configured_data_dir(exe_dir: &Path) -> Option<PathBuf> {
    let raw = fs::read(exe_dir.join("inventory.config.json")).ok()?;
    let config: Value = serde_json::from_slice(&raw).ok()?;
    config
        .get("dataDir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

/// `settings.backupDir` from an inventory document, if set.
fn backup_dir_setting(data: &Value) -> Option<PathBuf> {
    data.get("settings")
        .and_then(|settings| settings.get("backupDir"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn write_json_atomic(path: &Path, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_bytes_atomic(path, &body)
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // Unique temp name: lock heartbeat writes can race with data saves from
    // separate backend threads.
    static WRITE_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = PathBuf::from(format!(
        "{}.{}.{}.tmp",
        path.display(),
        std::process::id(),
        seq
    ));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all().ok();
    }
    swap_file(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(windows)]
fn swap_file(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let to_wide = |p: &Path| -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let src = to_wide(tmp);
    let dst = to_wide(dest);
    // MoveFileExW is the Windows atomic-replace primitive. std::fs::rename
    // fails when the destination already exists on Windows.
    // SAFETY: `src` and `dst` are NUL-terminated UTF-16 buffers that live
    // until the call returns.
    let ok = unsafe {
        MoveFileExW(
            src.as_ptr(),
            dst.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok != 0 {
        return Ok(());
    }
    // Extremely defensive fallback for exotic filesystems (some SMB servers).
    let err = std::io::Error::last_os_error();
    let _ = fs::remove_file(dest);
    fs::rename(tmp, dest).map_err(|_| err)
}

#[cfg(not(windows))]
fn swap_file(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    fs::rename(tmp, dest)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn iso_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn sanitize_user(user: &str) -> String {
    let cleaned: String = user
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "user".to_string()
    } else {
        cleaned
    }
}
