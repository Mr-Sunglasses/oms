//! Wallpaper previews: decoded on a background thread, kept small in a disk
//! cache so they load in milliseconds, and pre-made for every wallpaper while
//! the picker is idle.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Duration;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader};

use crate::settings::data_dir;

/// Previews are downsized to fit this, which is plenty for a terminal pane.
const PREVIEW_SIZE: (u32, u32) = (1600, 1000);
/// Thumbnails for the strip under the preview.
const THUMB_SIZE: (u32, u32) = (480, 300);
const CACHE_SIZE: usize = 12;

pub struct Preview {
    pub image: DynamicImage,
    pub thumb: DynamicImage,
}

type Loaded = (PathBuf, Result<Preview, String>);

pub struct Loader {
    requests: Sender<PathBuf>,
    results: Receiver<Loaded>,
    cache: HashMap<PathBuf, Preview>,
    order: VecDeque<PathBuf>,
    pending: HashSet<PathBuf>,
    pub errors: HashMap<PathBuf, String>,
}

impl Loader {
    /// `prefetch`: wallpapers to put in the disk cache while nothing else is asked for.
    pub fn new(prefetch: Vec<PathBuf>) -> Self {
        let (requests, inbox) = channel::<PathBuf>();
        let (outbox, results) = channel::<Loaded>();
        thread::spawn(move || {
            let mut prefetch: VecDeque<PathBuf> = prefetch
                .into_iter()
                .filter(|p| !cache_path(p).exists())
                .collect();
            let mut queue: Vec<PathBuf> = Vec::new();
            loop {
                if queue.is_empty() {
                    let wait = if prefetch.is_empty() {
                        Duration::from_secs(3600)
                    } else {
                        Duration::from_millis(30)
                    };
                    match inbox.recv_timeout(wait) {
                        Ok(path) => queue.push(path),
                        Err(RecvTimeoutError::Timeout) => {
                            // Idle: make one more cached preview.
                            if let Some(path) = prefetch.pop_front() {
                                let _ = load(&path);
                            }
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                queue.extend(inbox.try_iter());
                // Newest request first, so the picture being looked at wins.
                if let Some(path) = queue.pop() {
                    let result = load(&path);
                    if outbox.send((path, result)).is_err() {
                        return;
                    }
                }
            }
        });
        Loader {
            requests,
            results,
            cache: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
            errors: HashMap::new(),
        }
    }

    pub fn request(&mut self, path: &PathBuf) {
        if !self.cache.contains_key(path)
            && !self.errors.contains_key(path)
            && self.pending.insert(path.clone())
        {
            let _ = self.requests.send(path.clone());
        }
    }

    /// Collects finished images. Returns true if any arrived.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        for (path, result) in self.results.try_iter() {
            any = true;
            self.pending.remove(&path);
            match result {
                Ok(preview) => {
                    self.order.push_back(path.clone());
                    self.cache.insert(path, preview);
                    while self.order.len() > CACHE_SIZE {
                        if let Some(old) = self.order.pop_front() {
                            self.cache.remove(&old);
                        }
                    }
                }
                Err(e) => {
                    self.errors.insert(path, e);
                }
            }
        }
        any
    }

    pub fn get(&self, path: &PathBuf) -> Option<&Preview> {
        self.cache.get(path)
    }
}

/// `<data dir>/previews/<hash>.jpg`, keyed by the file's path, size and
/// modification time, so an edited wallpaper gets a new preview.
fn cache_path(path: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    if let Ok(meta) = fs::metadata(path) {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified() {
            modified.hash(&mut hasher);
        }
    }
    data_dir()
        .join("previews")
        .join(format!("{:016x}.jpg", hasher.finish()))
}

fn decode(path: &Path) -> Result<DynamicImage, String> {
    ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())
}

/// The small cached copy if there is one; otherwise decodes the original,
/// shrinks it and caches that.
fn load(path: &Path) -> Result<Preview, String> {
    let cached = cache_path(path);
    let image = if let Ok(image) = decode(&cached) {
        image
    } else {
        let full = decode(path)?;
        let small = if full.width() > PREVIEW_SIZE.0 || full.height() > PREVIEW_SIZE.1 {
            full.thumbnail(PREVIEW_SIZE.0, PREVIEW_SIZE.1)
        } else {
            full
        };
        let small = DynamicImage::ImageRgb8(small.to_rgb8());
        save(&small, &cached);
        small
    };
    let thumb = image.thumbnail(THUMB_SIZE.0, THUMB_SIZE.1);
    Ok(Preview {
        image: DynamicImage::ImageRgb8(image.to_rgb8()),
        thumb,
    })
}

fn save(image: &DynamicImage, path: &Path) {
    let Some(dir) = path.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    // Write next to it and rename, so a half-written file is never read.
    let tmp = path.with_extension("tmp");
    let written = fs::File::create(&tmp).is_ok_and(|file| {
        let mut writer = std::io::BufWriter::new(file);
        image
            .write_with_encoder(JpegEncoder::new_with_quality(&mut writer, 88))
            .is_ok()
    });
    if written {
        let _ = fs::rename(&tmp, path);
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, ImageFormat, RgbImage};

    /// The trimmed `image` features must still read every format oms shows.
    #[test]
    fn decodes_the_formats_wallpapers_use() {
        let picture = DynamicImage::ImageRgb8(RgbImage::from_fn(32, 20, |x, y| {
            image::Rgb([x as u8 * 8, y as u8 * 12, 90])
        }));
        for format in [
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::WebP,
            ImageFormat::Gif,
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            picture
                .write_to(&mut bytes, format)
                .unwrap_or_else(|e| panic!("can't write {format:?}: {e}"));
            let back = image::load_from_memory_with_format(bytes.get_ref(), format)
                .unwrap_or_else(|e| panic!("can't read {format:?}: {e}"));
            assert_eq!((back.width(), back.height()), (32, 20));
        }
    }
}
