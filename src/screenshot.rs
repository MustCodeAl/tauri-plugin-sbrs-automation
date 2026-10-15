//! Screenshots on macOS, taken by the window's `WKWebView` itself
//! (`takeSnapshotWithConfiguration:completionHandler:`) and encoded as PNG.
//!
//! WebKit draws the page out of process, so the snapshot has to come from the
//! web view; it runs on the main thread, as AppKit requires, and answers
//! through a completion handler.

use std::sync::mpsc;
use std::time::Duration;

use base64::Engine as _;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, Message as _};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSImage, NSView,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSDictionary, NSError};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use tauri::{Manager, Runtime, WebviewWindow};

use crate::webdriver::{ViewportRect, WebDriverError};

/// How long WebKit may take to draw a snapshot.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);

/// A PNG, or why there is none.
type Png = Result<Vec<u8>, WebDriverError>;

/// Takes a PNG snapshot of the window's web view, or of `area` of it, and
/// returns it base64-encoded.
pub fn take<R: Runtime>(
    window: &WebviewWindow<R>,
    area: Option<ViewportRect>,
) -> Result<String, WebDriverError> {
    let (sender, answer) = mpsc::channel::<Png>();
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            if let Err(error) = start(&target, area, sender.clone()) {
                // The receiver may have given up already.
                let _ = sender.send(Err(error));
            }
        })
        .map_err(|error| failed(format!("cannot reach the main thread: {error}")))?;
    let png = answer.recv_timeout(SNAPSHOT_TIMEOUT).map_err(|_| {
        WebDriverError::new("timeout", "the web view did not draw a snapshot in time")
    })??;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

/// Asks the web view for a snapshot; the PNG, or the error, goes to
/// `sender`. Runs on the main thread.
fn start<R: Runtime>(
    window: &WebviewWindow<R>,
    area: Option<ViewportRect>,
    sender: mpsc::Sender<Png>,
) -> Result<(), WebDriverError> {
    let main_thread = MainThreadMarker::new().ok_or_else(|| failed("not on the main thread"))?;
    let content = window
        .ns_view()
        .map_err(|error| failed(format!("the window has no content view: {error}")))?;
    // SAFETY: Tauri returns the window's content view, an `NSView` the window
    // keeps alive while it is open, and this runs on the main thread, where
    // AppKit views may be used.
    let content = unsafe { content.cast::<NSView>().as_ref() }
        .ok_or_else(|| failed("the window has no content view"))?;
    let webview = find_webview(content).ok_or_else(|| failed("the window holds no WKWebView"))?;
    // SAFETY: a new configuration, made on the main thread.
    let configuration = unsafe { WKSnapshotConfiguration::new(main_thread) };
    if let Some(area) = area {
        let rect = CGRect::new(
            CGPoint::new(area.x, area.y),
            CGSize::new(area.width, area.height),
        );
        // SAFETY: any rectangle is accepted; WebKit draws the part in view.
        unsafe { configuration.setRect(rect) };
    }
    let handler = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        // SAFETY: WebKit passes an image or an error, each null or valid for
        // the duration of the call.
        let png = match unsafe { image.as_ref() } {
            Some(image) => encode(image),
            None => Err(failed(unsafe { error.as_ref() }.map_or_else(
                || "WebKit gave no snapshot".to_owned(),
                |error| format!("WebKit gave no snapshot: {}", error.localizedDescription()),
            ))),
        };
        // The receiver may have given up already.
        let _ = sender.send(png);
    });
    // SAFETY: the configuration and the handler are valid, and WebKit calls
    // the handler once, on the main thread.
    unsafe {
        webview.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &handler)
    };
    Ok(())
}

/// The first `WKWebView` in `view` or among its subviews.
fn find_webview(view: &NSView) -> Option<Retained<WKWebView>> {
    if let Some(webview) = view.downcast_ref::<WKWebView>() {
        return Some(webview.retain());
    }
    let subviews = view.subviews();
    (0..subviews.count()).find_map(|index| find_webview(&subviews.objectAtIndex(index)))
}

/// The snapshot as PNG bytes.
fn encode(image: &NSImage) -> Png {
    let tiff = image
        .TIFFRepresentation()
        .ok_or_else(|| failed("the snapshot has no bitmap"))?;
    let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)
        .ok_or_else(|| failed("the snapshot's bitmap cannot be read"))?;
    let properties = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new();
    // SAFETY: an empty dictionary is a valid set of PNG properties.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }
    .ok_or_else(|| failed("the snapshot cannot be encoded as PNG"))?;
    Ok(png.to_vec())
}

fn failed(message: impl Into<String>) -> WebDriverError {
    WebDriverError::new("unable to capture screen", message)
}
