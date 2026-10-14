//! What the web build does in place of a file system, which a browser page
//! does not have. The projects it opens are the examples it carries and
//! the files the user picks, kept by name for the session; a project it
//! saves is downloaded. `read_to_string` and `write` stand in for
//! `std::fs`'s where projects are read and written.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, TryRecvError};
use wasm_bindgen::JsCast;

use crate::ui::file_windows::BUNDLED_EXAMPLES;

thread_local! {
    /// The files the user picked this session, by the name they were
    /// picked as.
    static PICKED: RefCell<HashMap<PathBuf, Vec<u8>>> = RefCell::new(HashMap::new());
}

/// What a feature that needs the file system, other programs or the
/// network says in the web build.
pub fn unavailable(feature: &str) -> String {
    format!("{feature} is not available in the web version of Nerthus.")
}

/// A file picked this session, else the example of that file name.
pub fn read(path: &Path) -> Option<Vec<u8>> {
    let picked = PICKED.with(|files| files.borrow().get(path).cloned());
    if picked.is_some() {
        return picked;
    }
    let name = path.file_name()?.to_str()?;
    let example = BUNDLED_EXAMPLES.iter().find(|(example, _)| *example == name);
    example.map(|(_, bytes)| bytes.to_vec())
}

/// Whether `read` has the file.
pub fn exists(path: &Path) -> bool {
    let picked = PICKED.with(|files| files.borrow().contains_key(path));
    let name = path.file_name().and_then(|name| name.to_str());
    picked || name.is_some_and(|name| BUNDLED_EXAMPLES.iter().any(|(example, _)| *example == name))
}

/// `std::fs::read_to_string` for the files `read` has.
pub fn read_to_string(path: &Path) -> io::Result<String> {
    let bytes = read(path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file in this session"))?;
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// `std::fs::write`: the contents downloaded under the path's file name,
/// and kept for reading back this session.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let contents = contents.as_ref();
    let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled.terrain".to_string());
    download(&name, contents).map_err(io::Error::other)?;
    PICKED.with(|files| files.borrow_mut().insert(path.to_path_buf(), contents.to_vec()));
    Ok(())
}

/// Hands `contents` to the browser as a download named `name`.
fn download(name: &str, contents: &[u8]) -> Result<(), String> {
    let window = web_sys::window().ok_or("no browser window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(contents));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(describe)?;
    let address = web_sys::Url::create_object_url_with_blob(&blob).map_err(describe)?;
    let link: web_sys::HtmlAnchorElement = document.create_element("a").map_err(describe)?.unchecked_into();
    link.set_href(&address);
    link.set_download(name);
    link.click();
    // Let go of the contents once the download has surely started (at
    // once, some browsers drop it).
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || web_sys::Url::revoke_object_url(&address));
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 60_000).map_err(describe)?;
    Ok(())
}

fn describe(value: wasm_bindgen::JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

/// A file the user is picking: its path once picked and kept (none if
/// they cancelled).
pub struct Picking {
    answer: Receiver<Option<PathBuf>>,
}

impl Picking {
    /// The browser's file picker, offering files with these extensions.
    pub fn start(filter: &str, extensions: &[&str]) -> Self {
        let (sender, answer) = crossbeam_channel::bounded(1);
        let dialog = rfd::AsyncFileDialog::new().add_filter(filter, extensions);
        wasm_bindgen_futures::spawn_local(async move {
            let picked = match dialog.pick_file().await {
                Some(file) => {
                    let path = PathBuf::from(file.file_name());
                    let contents = file.read().await;
                    PICKED.with(|files| files.borrow_mut().insert(path.clone(), contents));
                    Some(path)
                }
                None => None,
            };
            let _ = sender.send(picked);
        });
        Self { answer }
    }

    /// The answer, once given: `Some(None)` when cancelled.
    pub fn poll(&self) -> Option<Option<PathBuf>> {
        match self.answer.try_recv() {
            Ok(picked) => Some(picked),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(None),
        }
    }
}
