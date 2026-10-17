//! What the window says about its size and state, for finding out why a
//! window command did not do what it was asked.
//!
//! Off unless `SBRS_AUTOMATION_TRACE_WINDOW` is set. Then each window command
//! prints the window's state (what Tauri reports and, on macOS, what the
//! `NSWindow` says: style mask, `isZoomed`, frame, content view, screen) on
//! stderr before it asks for a change, after it asked, and when it answers.

use std::sync::OnceLock;

use tauri::{Runtime, WebviewWindow};

/// Whether the user asked for a trace.
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("SBRS_AUTOMATION_TRACE_WINDOW").is_some_and(|value| !value.is_empty())
    })
}

/// The window's state, in one line.
pub(crate) fn describe<R: Runtime>(window: &WebviewWindow<R>) -> String {
    let scale = window.scale_factor().ok();
    let size = |size: Option<tauri::PhysicalSize<u32>>| size.map(|s| (s.width, s.height));
    let tauri = format!(
        "tauri[fullscreen={:?} maximized={:?} minimized={:?} scale={scale:?} outer={:?} inner={:?}]",
        window.is_fullscreen().ok(),
        window.is_maximized().ok(),
        window.is_minimized().ok(),
        size(window.outer_size().ok()),
        size(window.inner_size().ok()),
    );
    #[cfg(target_os = "macos")]
    {
        format!("{tauri} native[{}]", appkit::describe(window))
    }
    #[cfg(not(target_os = "macos"))]
    {
        tauri
    }
}

#[cfg(target_os = "macos")]
mod appkit {
    use std::sync::mpsc;
    use std::time::Duration;

    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool};
    use objc2_core_foundation::{CGRect, CGSize};
    use tauri::{Manager as _, Runtime, WebviewWindow};

    /// The `NSWindowStyleMaskFullScreen` bit.
    const FULL_SCREEN_MASK: usize = 1 << 14;

    /// What the `NSWindow` says, read on the main thread.
    pub(super) fn describe<R: Runtime>(window: &WebviewWindow<R>) -> String {
        let (sender, answer) = mpsc::channel();
        let target = window.clone();
        if window
            .run_on_main_thread(move || {
                // The receiver may have given up already.
                let _ = sender.send(read(&target));
            })
            .is_err()
        {
            return "the main thread cannot be reached".to_owned();
        }
        answer
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| "the main thread did not answer".to_owned())
    }

    fn rect(rect: CGRect) -> String {
        format!(
            "({:.0},{:.0} {:.0}x{:.0})",
            rect.origin.x, rect.origin.y, rect.size.width, rect.size.height
        )
    }

    fn size(size: CGSize) -> String {
        format!("{:.0}x{:.0}", size.width, size.height)
    }

    /// Reads the `NSWindow`. Runs on the main thread.
    fn read<R: Runtime>(window: &WebviewWindow<R>) -> String {
        let Ok(pointer) = window.ns_window() else {
            return "the window has no NSWindow".to_owned();
        };
        let ns_window: *mut AnyObject = pointer.cast();
        // SAFETY: Tauri returns the window's `NSWindow`, which the window keeps
        // alive while it is open, and this runs on the main thread, where
        // AppKit may be used. Each message is a property getter of `NSWindow`,
        // `NSView` or `NSScreen` with the return type declared here.
        unsafe {
            let style: usize = msg_send![ns_window, styleMask];
            let zoomed: Bool = msg_send![ns_window, isZoomed];
            let miniaturized: Bool = msg_send![ns_window, isMiniaturized];
            let key: Bool = msg_send![ns_window, isKeyWindow];
            let frame: CGRect = msg_send![ns_window, frame];
            let backing: f64 = msg_send![ns_window, backingScaleFactor];
            let minimum: CGSize = msg_send![ns_window, minSize];
            let maximum: CGSize = msg_send![ns_window, maxSize];
            let content: Option<Retained<AnyObject>> = msg_send![ns_window, contentView];
            let content = content.map(|view| {
                let frame: CGRect = msg_send![&*view, frame];
                rect(frame)
            });
            let screen: Option<Retained<AnyObject>> = msg_send![ns_window, screen];
            let screen = screen.map(|screen| {
                let frame: CGRect = msg_send![&*screen, frame];
                let visible: CGRect = msg_send![&*screen, visibleFrame];
                format!("frame={} visible={}", rect(frame), rect(visible))
            });
            format!(
                "style={style:#x} fullscreenBit={} zoomed={} miniaturized={} key={} frame={} content={content:?} backing={backing} min={} max={} screen={screen:?}",
                style & FULL_SCREEN_MASK != 0,
                zoomed.as_bool(),
                miniaturized.as_bool(),
                key.as_bool(),
                rect(frame),
                size(minimum),
                size(maximum),
            )
        }
    }
}
