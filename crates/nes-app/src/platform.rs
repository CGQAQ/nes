//! Platform glue: where saves live, and loading files without blocking (the
//! browser can only read files asynchronously).

use std::sync::{Arc, Mutex};

/// A file picked or dropped by the user, delivered asynchronously.
pub struct PickedFile {
    pub name: String,
    /// Native path, for the recent-files list.
    pub path: Option<String>,
    pub data: Vec<u8>,
}

/// Slot that async file loads complete into; the app polls it every frame.
#[derive(Clone, Default)]
pub struct FileInbox(Arc<Mutex<Vec<PickedFile>>>);

impl FileInbox {
    pub fn deliver(&self, file: PickedFile) {
        self.0.lock().unwrap().push(file);
    }

    pub fn take(&self) -> Vec<PickedFile> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// Opens a file picker for ROMs; the result arrives in `inbox`.
pub fn pick_rom(inbox: &FileInbox) {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Open NES ROM")
        .add_filter("NES ROM", &["nes", "zip", "NES", "ZIP"])
        .add_filter("All files", &["*"]);
    let inbox = inbox.clone();
    spawn_task(async move {
        if let Some(handle) = dialog.pick_file().await {
            let data = handle.read().await;
            #[cfg(not(target_arch = "wasm32"))]
            let path = Some(handle.path().display().to_string());
            #[cfg(target_arch = "wasm32")]
            let path = None;
            inbox.deliver(PickedFile {
                name: handle.file_name(),
                path,
                data,
            });
        }
    });
}

/// Reads a file the user dropped on the window.
pub fn read_dropped(file: egui::DroppedFileHandle, inbox: &FileInbox) {
    let name = file
        .path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "dropped.nes".into());
    #[cfg(not(target_arch = "wasm32"))]
    match file.bytes() {
        Ok(data) => inbox.deliver(PickedFile {
            name,
            path: Some(file.path().display().to_string()),
            data,
        }),
        Err(e) => log::error!("reading dropped file: {e}"),
    }
    #[cfg(target_arch = "wasm32")]
    {
        let inbox = inbox.clone();
        spawn_task(async move {
            match file.bytes_async().await {
                Ok(data) => inbox.deliver(PickedFile {
                    name,
                    path: None,
                    data,
                }),
                Err(e) => log::error!("reading dropped file: {e}"),
            }
        });
    }
}

/// Query-string parameters of the page (`?rom=...&debug`), web only.
#[cfg(target_arch = "wasm32")]
pub fn page_params() -> Vec<(String, String)> {
    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let decode = |s: &str| {
                js_sys::decode_uri_component(&s.replace('+', " "))
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_else(|| s.to_string())
            };
            (decode(k), decode(v))
        })
        .collect()
}

/// Downloads a ROM from `url` into the inbox (web only: `?rom=<url>`).
#[cfg(target_arch = "wasm32")]
pub fn fetch_rom(url: String, inbox: &FileInbox) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let inbox = inbox.clone();
    spawn_task(async move {
        let result: Result<Vec<u8>, String> = async {
            let window = web_sys::window().ok_or("no window")?;
            let resp = JsFuture::from(window.fetch_with_str(&url))
                .await
                .map_err(|e| format!("{e:?}"))?;
            let resp: web_sys::Response = resp.dyn_into().map_err(|e| format!("{e:?}"))?;
            if !resp.ok() {
                return Err(format!("HTTP {}", resp.status()));
            }
            let buf = JsFuture::from(resp.array_buffer().map_err(|e| format!("{e:?}"))?)
                .await
                .map_err(|e| format!("{e:?}"))?;
            Ok(js_sys::Uint8Array::new(&buf).to_vec())
        }
        .await;
        match result {
            Ok(data) => {
                let name = url.rsplit('/').next().unwrap_or("rom.nes").to_string();
                inbox.deliver(PickedFile {
                    name,
                    path: None,
                    data,
                });
            }
            Err(e) => log::error!("fetching {url}: {e}"),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_task(fut: impl std::future::Future<Output = ()> + Send + 'static) {
    std::thread::spawn(move || pollster_block_on(fut));
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_task(fut: impl std::future::Future<Output = ()> + 'static) {
    wasm_bindgen_futures::spawn_local(fut);
}

/// Minimal executor for rfd's native futures (which complete on their own
/// thread and just need polling to completion).
#[cfg(not(target_arch = "wasm32"))]
fn pollster_block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, Wake, Waker};
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// Persistent key/value blob storage for battery saves and save states:
/// files in the app's data directory on desktop, local storage on the web.
pub struct Store {
    #[cfg(not(target_arch = "wasm32"))]
    dir: Option<std::path::PathBuf>,
    #[cfg(target_arch = "wasm32")]
    blobs: std::collections::BTreeMap<String, String>,
    #[cfg(target_arch = "wasm32")]
    dirty: bool,
}

#[cfg(target_arch = "wasm32")]
const BLOB_INDEX_KEY: &str = "nes_blob_keys";

impl Store {
    pub fn new(storage: Option<&dyn eframe::Storage>) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = storage;
            let dir = eframe::storage_dir("nes");
            if let Some(d) = &dir {
                let _ = std::fs::create_dir_all(d);
            }
            Self { dir }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let mut blobs = std::collections::BTreeMap::new();
            if let Some(s) = storage {
                let keys: Vec<String> = eframe::get_value(s, BLOB_INDEX_KEY).unwrap_or_default();
                for k in keys {
                    if let Some(v) = s.get_string(&format!("blob:{k}")) {
                        blobs.insert(k, v);
                    }
                }
            }
            Self {
                blobs,
                dirty: false,
            }
        }
    }

    /// Where files are kept, for display.
    pub fn location(&self) -> String {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.dir
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "(unavailable)".into())
        }
        #[cfg(target_arch = "wasm32")]
        {
            "browser local storage".into()
        }
    }

    pub fn read(&self, key: &str) -> Option<Vec<u8>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::read(self.dir.as_ref()?.join(key)).ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.blobs.get(key).and_then(|h| from_hex(h))
        }
    }

    pub fn write(&mut self, key: &str, data: &[u8]) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = &self.dir {
            let path = dir.join(key);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, data) {
                log::error!("writing {}: {e}", path.display());
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.blobs.insert(key.to_string(), to_hex(data));
            self.dirty = true;
        }
    }

    pub fn exists(&self, key: &str) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.dir.as_ref().is_some_and(|d| d.join(key).exists())
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.blobs.contains_key(key)
        }
    }

    /// Flushes to eframe storage (web only; native writes immediately).
    pub fn persist(&mut self, storage: &mut dyn eframe::Storage) {
        #[cfg(target_arch = "wasm32")]
        if self.dirty {
            let keys: Vec<String> = self.blobs.keys().cloned().collect();
            for (k, v) in &self.blobs {
                storage.set_string(&format!("blob:{k}"), v.clone());
            }
            eframe::set_value(storage, BLOB_INDEX_KEY, &keys);
            self.dirty = false;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = storage;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }
}

#[cfg(target_arch = "wasm32")]
fn to_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(target_arch = "wasm32")]
fn from_hex(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
