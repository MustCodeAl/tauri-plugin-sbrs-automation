//! The app's window, as the window commands (`window.rs`) use it.

use tauri::{LogicalPosition, LogicalSize, Runtime, WebviewWindow};

use crate::trace;
use crate::webdriver::{WebDriverError, WindowRect};
use crate::window::{NativeWindow, WindowFlags};

/// A window of the app, driven through Tauri. Each change only asks: Tauri
/// queues it for the main thread, which applies it on one of its next turns.
pub(crate) struct Native<'a, R: Runtime>(&'a WebviewWindow<R>);

impl<'a, R: Runtime> Native<'a, R> {
    pub(crate) fn new(window: &'a WebviewWindow<R>) -> Self {
        Self(window)
    }
}

/// A window operation that failed, as a WebDriver error.
fn refused(error: tauri::Error) -> WebDriverError {
    WebDriverError::new("unknown error", format!("the window refused: {error}"))
}

impl<R: Runtime> NativeWindow for Native<'_, R> {
    fn flags(&self) -> Result<WindowFlags, WebDriverError> {
        Ok(WindowFlags {
            fullscreen: self.0.is_fullscreen().map_err(refused)?,
            maximized: self.0.is_maximized().map_err(refused)?,
            minimized: self.0.is_minimized().map_err(refused)?,
        })
    }

    fn rect(&self) -> Result<WindowRect, WebDriverError> {
        let scale = self.0.scale_factor().map_err(refused)?;
        let position = self
            .0
            .outer_position()
            .map_err(refused)?
            .to_logical::<f64>(scale);
        let size = self
            .0
            .outer_size()
            .map_err(refused)?
            .to_logical::<f64>(scale);
        Ok(WindowRect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        })
    }

    fn leave_fullscreen(&self) -> Result<(), WebDriverError> {
        self.0.set_fullscreen(false).map_err(refused)
    }

    fn enter_fullscreen(&self) -> Result<(), WebDriverError> {
        self.0.set_fullscreen(true).map_err(refused)
    }

    fn unmaximize(&self) -> Result<(), WebDriverError> {
        self.0.unmaximize().map_err(refused)
    }

    fn maximize(&self) -> Result<(), WebDriverError> {
        self.0.maximize().map_err(refused)
    }

    fn unminimize(&self) -> Result<(), WebDriverError> {
        self.0.unminimize().map_err(refused)
    }

    fn minimize(&self) -> Result<(), WebDriverError> {
        self.0.minimize().map_err(refused)
    }

    fn move_to(&self, x: f64, y: f64) -> Result<(), WebDriverError> {
        self.0
            .set_position(LogicalPosition::new(x, y))
            .map_err(refused)
    }

    fn resize_to(&self, width: f64, height: f64) -> Result<(), WebDriverError> {
        // Tauri sizes the inside of the window, WebDriver the outside: take
        // the frame off, as it is now.
        let outer = self.rect()?;
        let scale = self.0.scale_factor().map_err(refused)?;
        let inner = self
            .0
            .inner_size()
            .map_err(refused)?
            .to_logical::<f64>(scale);
        let size = LogicalSize::new(
            (width - (outer.width - inner.width)).max(1.0),
            (height - (outer.height - inner.height)).max(1.0),
        );
        self.0.set_size(size).map_err(refused)
    }

    fn trace(&self, label: &str) {
        if trace::enabled() {
            eprintln!(
                "tauri-plugin-sbrs-automation window: {label}: {}",
                trace::describe(self.0)
            );
        }
    }
}
