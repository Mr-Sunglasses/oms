//! macOS: plays the live wallpaper in a borderless window on every screen,
//! at the desktop picture's level (under the desktop icons), on every Space.
//! One muted, looping player feeds all of them, and it pauses whenever none of
//! them can be seen (a full-screen app, a locked or sleeping screen). An intro
//! plays once instead; then the windows close on its still, which is already
//! the desktop picture underneath.

use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy,
    NSApplicationDidChangeScreenParametersNotification, NSBackingStoreType, NSColor, NSEventMask,
    NSScreen, NSView, NSWindow, NSWindowCollectionBehavior,
    NSWindowDidChangeOcclusionStateNotification, NSWindowOcclusionState, NSWindowStyleMask,
    NSWorkspace, NSWorkspaceScreensDidWakeNotification,
    NSWorkspaceSessionDidBecomeActiveNotification,
};
use objc2_av_foundation::{
    AVLayerVideoGravityResizeAspectFill, AVPlayerActionAtItemEnd, AVPlayerItem,
    AVPlayerItemDidPlayToEndTimeNotification, AVPlayerLayer, AVPlayerLooper, AVQueuePlayer,
};
use objc2_foundation::{
    NSArray, NSDate, NSDefaultRunLoopMode, NSNotification, NSNotificationCenter,
    NSNotificationName, NSPoint, NSRect, NSString, NSURL,
};

use super::log;
use crate::wallpaper::live;

/// `kCGDesktopWindowLevelKey`: the desktop picture's level, under the icons.
const DESKTOP_WINDOW_LEVEL_KEY: i32 = 2;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
}

type Observer = (
    Retained<NSNotificationCenter>,
    Retained<ProtocolObject<dyn NSObjectProtocol>>,
);

struct Playing {
    video: PathBuf,
    player: Retained<AVQueuePlayer>,
    /// Loops it; an intro has none.
    _looper: Option<Retained<AVPlayerLooper>>,
    /// Tells when an intro has played to its end.
    end: Option<Observer>,
    windows: Vec<Retained<NSWindow>>,
    paused: bool,
}

pub struct Live {
    /// `None` off the main thread, where `AppKit` can't be used.
    mtm: Option<MainThreadMarker>,
    playing: Option<Playing>,
    /// Set up the first time a video plays, so the agent stays a plain
    /// background process for everyone else.
    app_ready: bool,
    observers: Vec<Observer>,
    woken: Arc<AtomicBool>,
    screens_changed: Arc<AtomicBool>,
    intro_ended: Arc<AtomicBool>,
}

impl Live {
    pub fn new(woken: &Arc<AtomicBool>) -> Self {
        Self {
            mtm: MainThreadMarker::new(),
            playing: None,
            app_ready: false,
            observers: Vec::new(),
            woken: woken.clone(),
            screens_changed: Arc::new(AtomicBool::new(false)),
            intro_ended: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn active(&self) -> bool {
        self.playing.is_some()
    }

    /// Plays `want` (or stops, for `None`), follows screen changes, and
    /// pauses while nothing of it can be seen.
    pub fn sync(&mut self, want: Option<&Path>) {
        let Some(mtm) = self.mtm else { return };
        if self.intro_ended.swap(false, Ordering::SeqCst) && self.playing.is_some() {
            // Its last frame is the still underneath, so closing is seamless.
            log("the intro finished");
            self.stop();
            if let Err(e) = live::set_playing(None) {
                log(&format!("couldn't clear the intro: {e:#}"));
            }
            return;
        }
        if self.playing.as_ref().map(|p| p.video.as_path()) != want {
            self.stop();
            if let Some(video) = want {
                self.start(mtm, video);
            }
        }
        if self.screens_changed.swap(false, Ordering::SeqCst)
            && let Some(playing) = &mut self.playing
        {
            close(&playing.windows);
            playing.windows = windows(mtm, &playing.player);
        }
        if let Some(playing) = &mut self.playing {
            let visible = playing
                .windows
                .iter()
                .any(|w| w.occlusionState().contains(NSWindowOcclusionState::Visible));
            if visible == playing.paused {
                playing.paused = !visible;
                // SAFETY: plain playback control on a player made on this thread.
                unsafe {
                    if visible {
                        playing.player.play();
                    } else {
                        playing.player.pause();
                    }
                }
            }
        }
    }

    fn start(&mut self, mtm: MainThreadMarker, video: &Path) {
        if !self.app_ready {
            self.set_up_app(mtm);
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&video.to_string_lossy()));
        let intro = live::is_intro(video);
        // SAFETY: AVFoundation objects made and configured on the main thread;
        // the looper needs a template item, which `playerItemWithURL` gives.
        let (player, item, looper) = unsafe {
            let item = AVPlayerItem::playerItemWithURL(&url, mtm);
            let player = if intro {
                let items = NSArray::from_slice(&[&*item]);
                let player = AVQueuePlayer::queuePlayerWithItems(&items, mtm);
                // Hold the last frame until the windows close.
                player.setActionAtItemEnd(AVPlayerActionAtItemEnd::Pause);
                player
            } else {
                AVQueuePlayer::new(mtm)
            };
            player.setMuted(true);
            // Videos keep the display awake by default; a wallpaper mustn't.
            player.setPreventsDisplaySleepDuringVideoPlayback(false);
            player.setAllowsExternalPlayback(false);
            let looper = (!intro)
                .then(|| AVPlayerLooper::playerLooperWithPlayer_templateItem(&player, &item));
            player.play();
            (player, item, looper)
        };
        let end = intro.then(|| {
            self.intro_ended.store(false, Ordering::SeqCst);
            let center = NSNotificationCenter::defaultCenter();
            // SAFETY: AVFoundation exports the name as a constant string;
            // reading the extern static is all that's unsafe.
            let name = unsafe { AVPlayerItemDidPlayToEndTimeNotification };
            let observer = observe_end(&center, name, &item, &self.woken, &self.intro_ended);
            (center, observer)
        });
        let windows = windows(mtm, &player);
        log(&format!("playing {}", video.display()));
        self.playing = Some(Playing {
            video: video.to_path_buf(),
            player,
            _looper: looper,
            end,
            windows,
            paused: false,
        });
    }

    fn stop(&mut self) {
        if let Some(playing) = self.playing.take() {
            // SAFETY: plain playback control on a player made on this thread.
            unsafe { playing.player.pause() };
            close(&playing.windows);
            if let Some((center, observer)) = &playing.end {
                remove(center, observer);
            }
            log("stopped the live wallpaper");
        }
    }

    /// Makes the agent an app that can show windows but has no Dock icon, and
    /// wakes the agent when screens or their visibility change.
    fn set_up_app(&mut self, mtm: MainThreadMarker) {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        let default = NSNotificationCenter::defaultCenter();
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: Foundation and AppKit export these names as constant strings;
        // reading the extern statics is all that's unsafe.
        let names = unsafe {
            [
                (
                    &default,
                    NSApplicationDidChangeScreenParametersNotification,
                    true,
                ),
                (&default, NSWindowDidChangeOcclusionStateNotification, false),
                (&workspace, NSWorkspaceScreensDidWakeNotification, false),
                (
                    &workspace,
                    NSWorkspaceSessionDidBecomeActiveNotification,
                    false,
                ),
            ]
        };
        for (center, name, screens) in names {
            let observer = observe(
                center,
                name,
                &self.woken,
                screens.then_some(&self.screens_changed),
            );
            self.observers.push((center.clone(), observer));
        }
        self.app_ready = true;
    }

    /// Waits up to `timeout` for the agent to be woken, handling the window
    /// server's events meanwhile (an app with windows must, or it's treated
    /// as hung). Only called once a video has played.
    pub fn wait(&self, timeout: Duration, flag: &AtomicBool) {
        let Some(mtm) = self.mtm else {
            std::thread::sleep(timeout);
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let deadline = Instant::now() + timeout;
        loop {
            // Short slices, so a notification (which doesn't end the wait for an
            // event) is acted on within a second.
            let slice = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(1));
            let until = NSDate::dateWithTimeIntervalSinceNow(slice.as_secs_f64());
            // SAFETY: `NSDefaultRunLoopMode` is a constant string exported by
            // Foundation; reading the extern static is all that's unsafe here.
            let mode = unsafe { NSDefaultRunLoopMode };
            if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&until),
                mode,
                true,
            ) {
                app.sendEvent(&event);
            }
            if flag.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return;
            }
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop();
        for (center, observer) in &self.observers {
            remove(center, observer);
        }
    }
}

/// Unregisters an observer from the center it was added to.
fn remove(center: &NSNotificationCenter, observer: &ProtocolObject<dyn NSObjectProtocol>) {
    let observer: &AnyObject = observer.as_ref();
    // SAFETY: each observer was registered with this center, in `observe` or
    // `observe_end`.
    unsafe { center.removeObserver(observer) };
}

/// Registers a block that marks `ended` and wakes the agent when `item`
/// posts `name` (it played to its end).
fn observe_end(
    center: &NSNotificationCenter,
    name: &NSNotificationName,
    item: &AVPlayerItem,
    woken: &Arc<AtomicBool>,
    ended: &Arc<AtomicBool>,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let woken = woken.clone();
    let ended = ended.clone();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        ended.store(true, Ordering::SeqCst);
        woken.store(true, Ordering::SeqCst);
    });
    let item: &AnyObject = item.as_ref();
    // SAFETY: the block only stores to `AtomicBool`s, so it is fine to run on
    // whatever thread posts the notification (the queue is nil).
    unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), Some(item), None, &block)
    }
}

/// Registers a block that wakes the agent (and marks `screens` changed, if given).
fn observe(
    center: &NSNotificationCenter,
    name: &NSNotificationName,
    woken: &Arc<AtomicBool>,
    screens: Option<&Arc<AtomicBool>>,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let woken = woken.clone();
    let screens = screens.cloned();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        if let Some(screens) = &screens {
            screens.store(true, Ordering::SeqCst);
        }
        woken.store(true, Ordering::SeqCst);
    });
    // SAFETY: the block only stores to `AtomicBool`s, so it is fine to run on
    // whatever thread posts the notification (the queue is nil).
    unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) }
}

/// One window per screen, each showing `player`.
fn windows(mtm: MainThreadMarker, player: &AVQueuePlayer) -> Vec<Retained<NSWindow>> {
    // SAFETY: `CGWindowLevelForKey` takes any key and only returns a number.
    let level = unsafe { CGWindowLevelForKey(DESKTOP_WINDOW_LEVEL_KEY) };
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let frame = screen.frame();
            let local = NSRect::new(NSPoint::new(0.0, 0.0), frame.size);
            // SAFETY: a borderless window on a live screen, made on the main
            // thread. It isn't released on close, as Rust owns it.
            let window = unsafe {
                let window = NSWindow::initWithContentRect_styleMask_backing_defer_screen(
                    NSWindow::alloc(mtm),
                    local,
                    NSWindowStyleMask::Borderless,
                    NSBackingStoreType::Buffered,
                    false,
                    Some(&screen),
                );
                window.setReleasedWhenClosed(false);
                window
            };
            window.setFrame_display(frame, false);
            window.setLevel(level as isize);
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            window.setIgnoresMouseEvents(true);
            window.setOpaque(true);
            window.setHasShadow(false);
            window.setBackgroundColor(Some(&NSColor::blackColor()));

            // SAFETY: the layer shows a player made on this thread; the gravity
            // constant is exported by AVFoundation.
            let layer = unsafe {
                let layer = AVPlayerLayer::playerLayerWithPlayer(Some(player));
                if let Some(fill) = AVLayerVideoGravityResizeAspectFill {
                    layer.setVideoGravity(fill);
                }
                layer
            };
            let view = NSView::initWithFrame(NSView::alloc(mtm), local);
            // A layer-hosting view: the player layer is its layer.
            view.setLayer(Some(&layer));
            view.setWantsLayer(true);
            window.setContentView(Some(&view));
            window.orderFrontRegardless();
            window
        })
        .collect()
}

fn close(windows: &[Retained<NSWindow>]) {
    for window in windows {
        window.orderOut(None);
        window.close();
    }
}
