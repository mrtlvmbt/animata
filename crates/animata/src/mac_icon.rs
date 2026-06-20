//! macOS Dock-icon theming. macOS uses ONE static `.icns` from the `.app` bundle and never switches
//! it by light/dark appearance, so to honour the system theme we set `NSApp.applicationIconImage`
//! at runtime to the matching variant (and re-set it when the appearance changes). For a bare
//! `cargo run` binary (no bundle) this is also what puts any icon in the Dock at all.
//!
//! The two icon variants are embedded so the binary is self-contained; the `.app` bundle's static
//! default icon is built separately by `scripts/bundle-macos.sh` (the light variant).

use objc2::rc::Retained;
use objc2::ClassType;
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::{MainThreadMarker, NSData};

const ICON_LIGHT: &[u8] = include_bytes!("../assets/icons/light/animata-light-1024.png");
const ICON_DARK: &[u8] = include_bytes!("../assets/icons/dark/animata-dark-1024.png");

fn make_image(bytes: &[u8]) -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(bytes);
    NSImage::initWithData(NSImage::alloc(), &data)
}

/// Sync the Dock icon to the current system appearance. Tracks the last applied state in `last_dark`
/// and only rebuilds/sets the image when the appearance actually changes — cheap to call every frame.
pub fn sync(last_dark: &mut Option<bool>) {
    // The macroquad event loop drives this from the main thread on macOS (miniquad requires it), so
    // the unchecked marker is sound here.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let app = NSApplication::sharedApplication(mtm);
    // The effective appearance name contains "DarkAqua" in dark mode, "Aqua" in light — a robust,
    // version-stable check without juggling the appearance-name constant arrays.
    let appearance = app.effectiveAppearance();
    // SAFETY: `name` just reads the appearance's name string — no aliasing/lifetime contract.
    let is_dark = unsafe { appearance.name() }.to_string().contains("Dark");
    if *last_dark == Some(is_dark) {
        return;
    }
    let bytes = if is_dark { ICON_DARK } else { ICON_LIGHT };
    if let Some(img) = make_image(bytes) {
        // SAFETY: setting the Dock icon to a valid `NSImage` we own; called on the main thread.
        unsafe { app.setApplicationIconImage(Some(&img)) };
        *last_dark = Some(is_dark);
    }
}
