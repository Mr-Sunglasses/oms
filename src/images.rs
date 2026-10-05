//! Decodes and downsizes wallpapers on a background thread for the preview.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use image::{DynamicImage, ImageReader};

/// Previews are downsized to fit this, which is plenty for a terminal pane.
const MAX_SIZE: (u32, u32) = (1600, 1000);
const CACHE_SIZE: usize = 24;

type Loaded = (PathBuf, Result<DynamicImage, String>);

pub struct Loader {
    requests: Sender<PathBuf>,
    results: Receiver<Loaded>,
    cache: HashMap<PathBuf, DynamicImage>,
    order: VecDeque<PathBuf>,
    pending: HashSet<PathBuf>,
    pub errors: HashMap<PathBuf, String>,
}

impl Loader {
    pub fn new() -> Self {
        let (requests, inbox) = channel::<PathBuf>();
        let (outbox, results) = channel::<Loaded>();
        thread::spawn(move || {
            while let Ok(first) = inbox.recv() {
                // Newest request first, so the picture being looked at wins.
                let mut queue = vec![first];
                queue.extend(inbox.try_iter());
                while let Some(path) = queue.pop() {
                    let result = decode(&path);
                    if outbox.send((path, result)).is_err() {
                        return;
                    }
                    queue.extend(inbox.try_iter());
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
                Ok(image) => {
                    self.order.push_back(path.clone());
                    self.cache.insert(path, image);
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

    pub fn get(&self, path: &PathBuf) -> Option<&DynamicImage> {
        self.cache.get(path)
    }
}

fn decode(path: &PathBuf) -> Result<DynamicImage, String> {
    let image = ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let image = if image.width() > MAX_SIZE.0 || image.height() > MAX_SIZE.1 {
        image.thumbnail(MAX_SIZE.0, MAX_SIZE.1)
    } else {
        image
    };
    Ok(DynamicImage::ImageRgb8(image.to_rgb8()))
}
