//! The window commands of W3C WebDriver (Set Window Rect, Maximize, Minimize and
//! Full Screen), independent of Tauri.
//!
//! A call that changes a native window only asks for the change. Tauri queues
//! it for the main thread, and the system applies it later: leaving full
//! screen takes about a second, and a resize made meanwhile is lost. So a
//! command cannot take the window's rectangle right after its own calls as its
//! answer. The functions here change the window, wait until it reports the
//! change (never longer than a fixed time) and answer with the rectangle the
//! window has then, whether or not it is the one asked for: the system moves
//! a window to keep it below the menu bar, and may refuse a size.

use std::time::{Duration, Instant};

use crate::webdriver::{WebDriverError, WindowRect};

/// How long a window may take to change its state (to leave full screen, to
/// be maximized, ...). A window that still has not by then is used as it is.
const STATE_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a window may take to reach a rectangle. A window that has not by
/// then (the system refuses the size) is answered as it is.
const RECT_TIMEOUT: Duration = Duration::from_secs(2);
/// The pause between two looks at the window.
const POLL: Duration = Duration::from_millis(20);
/// How long a window must keep its rectangle to be taken as settled.
const SETTLED: Duration = Duration::from_millis(60);
/// How long a request is given to land before the rectangle can be taken as
/// settled: the main thread applies it on its next turn, not at once.
const LANDING: Duration = Duration::from_millis(100);
/// How long a window may keep a rectangle that is not the asked one before the
/// request is made again: a request made while the window changes state can be
/// lost. The request is made again every such while until [`RECT_TIMEOUT`], for
/// a window may hold its old frame (and say it left a state) while an animation
/// runs that swallows the request.
const RETRY_AFTER: Duration = Duration::from_millis(250);
/// Sizes closer than this are the same: the system rounds to whole pixels.
const TOLERANCE: f64 = 1.0;

/// The time, which a test can replace so that it does not wait for real.
pub(crate) trait Clock {
    /// The time since some fixed start.
    fn now(&self) -> Duration;
    /// Waits for `pause`.
    fn sleep(&self, pause: Duration);
}

/// The wall clock.
pub(crate) struct SystemClock(Instant);

impl SystemClock {
    pub(crate) fn new() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&self, pause: Duration) {
        std::thread::sleep(pause);
    }
}

/// Which of the window's states are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WindowFlags {
    pub(crate) fullscreen: bool,
    pub(crate) maximized: bool,
    pub(crate) minimized: bool,
}

/// What a native window gives these commands. Each change only asks: the
/// window applies it later.
pub(crate) trait NativeWindow {
    /// The window's states now.
    fn flags(&self) -> Result<WindowFlags, WebDriverError>;
    /// The window's outer rectangle now, in CSS pixels.
    fn rect(&self) -> Result<WindowRect, WebDriverError>;
    /// Asks the window to leave full screen.
    fn leave_fullscreen(&self) -> Result<(), WebDriverError>;
    /// Asks the window to enter full screen.
    fn enter_fullscreen(&self) -> Result<(), WebDriverError>;
    /// Asks the window to leave the maximized state.
    fn unmaximize(&self) -> Result<(), WebDriverError>;
    /// Asks the window to be maximized.
    fn maximize(&self) -> Result<(), WebDriverError>;
    /// Asks the window to come back from the dock.
    fn unminimize(&self) -> Result<(), WebDriverError>;
    /// Asks the window to be minimized.
    fn minimize(&self) -> Result<(), WebDriverError>;
    /// Asks for the window's top left corner to be at `x`, `y`.
    fn move_to(&self, x: f64, y: f64) -> Result<(), WebDriverError>;
    /// Asks for the window's outer size to be `width` x `height`.
    fn resize_to(&self, width: f64, height: f64) -> Result<(), WebDriverError>;
    /// Says what the window is, under `label`, when the user asked for a trace.
    fn trace(&self, label: &str) {
        let _ = label;
    }
}

/// What Set Window Rect asks for: a `None` keeps that value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RectRequest {
    pub(crate) x: Option<f64>,
    pub(crate) y: Option<f64>,
    pub(crate) width: Option<f64>,
    pub(crate) height: Option<f64>,
}

impl RectRequest {
    fn moves(&self) -> bool {
        self.x.is_some() || self.y.is_some()
    }

    fn resizes(&self) -> bool {
        self.width.is_some() || self.height.is_some()
    }

    /// Whether `rect` has the size asked for.
    fn is_sized(&self, rect: &WindowRect) -> bool {
        self.width
            .is_none_or(|width| (rect.width - width).abs() <= TOLERANCE)
            && self
                .height
                .is_none_or(|height| (rect.height - height).abs() <= TOLERANCE)
    }

    /// Whether `rect` is where the window was asked to be. The system keeps a
    /// window below the menu bar, so it may be lower than asked (a larger `y`)
    /// and that is taken as placed; a window higher than asked, or at another
    /// `x`, has not had the move yet.
    fn is_placed(&self, rect: &WindowRect) -> bool {
        self.x.is_none_or(|x| (rect.x - x).abs() <= TOLERANCE)
            && self.y.is_none_or(|y| rect.y >= y - TOLERANCE)
    }

    /// Whether `rect` is the one asked for, as far as the system allows.
    fn is_met(&self, rect: &WindowRect) -> bool {
        self.is_sized(rect) && self.is_placed(rect)
    }

    /// Asks `window` for the position and the size.
    fn apply<W: NativeWindow + ?Sized>(&self, window: &W) -> Result<(), WebDriverError> {
        if self.moves() {
            // The corner not given stays where the window is.
            let now = window.rect()?;
            window.move_to(self.x.unwrap_or(now.x), self.y.unwrap_or(now.y))?;
        }
        if self.resizes() {
            let now = window.rect()?;
            window.resize_to(
                self.width.unwrap_or(now.width),
                self.height.unwrap_or(now.height),
            )?;
        }
        Ok(())
    }
}

/// Get Window Rect: the window's outer rectangle as it is now.
///
/// # Errors
///
/// What the window reported when it was asked.
pub(crate) fn current_rect<W: NativeWindow + ?Sized>(
    window: &W,
) -> Result<WindowRect, WebDriverError> {
    window.trace("get rect");
    window.rect()
}

/// Set Window Rect: brings the window back to its normal state (out of full
/// screen, not maximized, not minimized), moves and resizes it, waits until it
/// has, and answers with its rectangle then.
///
/// # Errors
///
/// What the window reported when it was asked.
pub(crate) fn set_rect<W: NativeWindow + ?Sized>(
    window: &W,
    clock: &dyn Clock,
    request: RectRequest,
) -> Result<WindowRect, WebDriverError> {
    window.trace("set rect: asked");
    restore(window, clock)?;
    window.trace("set rect: restored");
    if !request.moves() && !request.resizes() {
        return settle(window, clock, None);
    }
    request.apply(window)?;
    window.trace("set rect: applied");
    let rect = settle(window, clock, Some(&request))?;
    window.trace("set rect: settled");
    Ok(rect)
}

/// Whether `a` and `b` are the same rectangle, to the pixel.
fn same(a: &WindowRect, b: &WindowRect) -> bool {
    (a.x - b.x).abs() <= TOLERANCE
        && (a.y - b.y).abs() <= TOLERANCE
        && (a.width - b.width).abs() <= TOLERANCE
        && (a.height - b.height).abs() <= TOLERANCE
}

/// Waits until `done` holds for the window's states. Gives up after
/// [`STATE_TIMEOUT`] and says so (`false`): a window that does not change
/// state is then used as it is, and the rectangle it answers with tells.
fn wait_for_state<W: NativeWindow + ?Sized>(
    window: &W,
    clock: &dyn Clock,
    done: impl Fn(&WindowFlags) -> bool,
) -> Result<bool, WebDriverError> {
    let started = clock.now();
    loop {
        if done(&window.flags()?) {
            return Ok(true);
        }
        if clock.now().saturating_sub(started) >= STATE_TIMEOUT {
            return Ok(false);
        }
        clock.sleep(POLL);
    }
}

/// Waits until the window's rectangle has stopped changing, and returns it.
/// With a `request`, also until the window has the size and the place it asked
/// for, asking again when the window stays as it is: a request made while the
/// window changes state can be lost. Gives up after [`RECT_TIMEOUT`] and
/// returns the rectangle as it is.
fn settle<W: NativeWindow + ?Sized>(
    window: &W,
    clock: &dyn Clock,
    request: Option<&RectRequest>,
) -> Result<WindowRect, WebDriverError> {
    let started = clock.now();
    let mut seen = window.rect()?;
    // When `seen` was first seen.
    let mut since = started;
    loop {
        clock.sleep(POLL);
        let rect = window.rect()?;
        let now = clock.now();
        if !same(&rect, &seen) {
            seen = rect;
            since = now;
        }
        let kept_for = now.saturating_sub(since);
        let met = request.is_none_or(|request| request.is_met(&rect));
        // A request lands within a turn of the main thread, but not at once:
        // the rectangle is not taken for settled before it had its time.
        let landed = now.saturating_sub(started) >= LANDING;
        if landed && kept_for >= SETTLED && met {
            return Ok(rect);
        }
        if !met && kept_for >= RETRY_AFTER {
            if let Some(request) = request {
                request.apply(window)?;
            }
            since = now;
        }
        if now.saturating_sub(started) >= RECT_TIMEOUT {
            return Ok(rect);
        }
    }
}

/// Brings the window back to its normal state: out of full screen, not
/// maximized, not minimized. A state is left by asking and waiting until the
/// window reports it has, and then until the frame stops moving: leaving full
/// screen is an animation, over which a new size is lost. The window is looked
/// at again after each state, because leaving one can bring another back (a
/// window that was maximized comes back from full screen maximized). A window
/// that does not leave a state in [`STATE_TIMEOUT`] is not asked again: it is
/// used as it is.
fn restore<W: NativeWindow + ?Sized>(window: &W, clock: &dyn Clock) -> Result<(), WebDriverError> {
    for _ in 0..4 {
        let flags = window.flags()?;
        let left = if flags.fullscreen {
            window.leave_fullscreen()?;
            wait_for_state(window, clock, |flags| !flags.fullscreen)?
        } else if flags.maximized {
            window.unmaximize()?;
            wait_for_state(window, clock, |flags| !flags.maximized)?
        } else if flags.minimized {
            window.unminimize()?;
            wait_for_state(window, clock, |flags| !flags.minimized)?
        } else {
            break;
        };
        if !left {
            break;
        }
        settle(window, clock, None)?;
    }
    Ok(())
}

/// Maximize Window: out of full screen and the dock first, then maximized.
///
/// # Errors
///
/// What the window reported when it was asked.
pub(crate) fn maximize<W: NativeWindow + ?Sized>(
    window: &W,
    clock: &dyn Clock,
) -> Result<WindowRect, WebDriverError> {
    window.trace("maximize: asked");
    let flags = window.flags()?;
    if flags.fullscreen {
        window.leave_fullscreen()?;
        wait_for_state(window, clock, |flags| !flags.fullscreen)?;
    }
    if flags.minimized {
        window.unminimize()?;
        wait_for_state(window, clock, |flags| !flags.minimized)?;
    }
    if !flags.maximized {
        if flags.fullscreen || flags.minimized {
            settle(window, clock, None)?;
        }
        window.maximize()?;
        wait_for_state(window, clock, |flags| flags.maximized)?;
    }
    let rect = settle(window, clock, None)?;
    window.trace("maximize: settled");
    Ok(rect)
}

/// Minimize Window.
///
/// # Errors
///
/// What the window reported when it was asked.
pub(crate) fn minimize<W: NativeWindow + ?Sized>(
    window: &W,
    clock: &dyn Clock,
) -> Result<WindowRect, WebDriverError> {
    window.trace("minimize: asked");
    if !window.flags()?.minimized {
        window.minimize()?;
        wait_for_state(window, clock, |flags| flags.minimized)?;
    }
    let rect = settle(window, clock, None)?;
    window.trace("minimize: settled");
    Ok(rect)
}

/// Fullscreen Window.
///
/// # Errors
///
/// What the window reported when it was asked.
pub(crate) fn fullscreen<W: NativeWindow + ?Sized>(
    window: &W,
    clock: &dyn Clock,
) -> Result<WindowRect, WebDriverError> {
    window.trace("fullscreen: asked");
    let flags = window.flags()?;
    if flags.minimized {
        window.unminimize()?;
        wait_for_state(window, clock, |flags| !flags.minimized)?;
    }
    if !flags.fullscreen {
        window.enter_fullscreen()?;
        wait_for_state(window, clock, |flags| flags.fullscreen)?;
    }
    let rect = settle(window, clock, None)?;
    window.trace("fullscreen: settled");
    Ok(rect)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// A clock that does not wait: `sleep` moves it forward.
    #[derive(Default)]
    struct FakeClock(Cell<Duration>);

    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            self.0.get()
        }

        fn sleep(&self, pause: Duration) {
            self.0.set(self.0.get() + pause);
        }
    }

    /// What a call asked of the simulated window.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        LeaveFullscreen,
        EnterFullscreen,
        Unmaximize,
        Maximize,
        Unminimize,
        Minimize,
        MoveTo(f64, f64),
        ResizeTo(f64, f64),
    }

    struct State {
        rect: WindowRect,
        /// The rectangle the window has when it is in its normal state.
        normal: WindowRect,
        flags: WindowFlags,
        queue: Vec<(Duration, Op)>,
        /// Until then a change of state is under way, and a move or resize is lost.
        busy_until: Duration,
        /// The frame moves from one rectangle to another over a while, as it
        /// does when a window leaves full screen: (start, end, from, to).
        animation: Option<(Duration, Duration, WindowRect, WindowRect)>,
        resizes_asked: u32,
        resizes_to_lose: u32,
        moves_asked: u32,
        moves_to_lose: u32,
        leaves_asked: u32,
    }

    /// A window that applies each call some time after it: leaving full screen
    /// takes a while and loses the moves and resizes made meanwhile, a window
    /// that is full screen, maximized or minimized takes no size, and the
    /// system keeps a window below a menu bar of `MENU_BAR`.
    struct Sim {
        clock: Rc<FakeClock>,
        state: RefCell<State>,
        latency: Duration,
        transition: Duration,
        refuses_sizes: bool,
        /// A window that was maximized before full screen comes back maximized.
        returns_maximized: bool,
        /// While it leaves full screen the window says it has, but its frame
        /// stays the full screen one until the animation is over.
        holds_frame: bool,
        /// A window that never leaves full screen.
        ignores_leaving: bool,
    }

    /// The window maximized: the screen below the menu bar and above the dock.
    const MAXIMIZED: WindowRect = WindowRect {
        x: 0.0,
        y: MENU_BAR,
        width: 1680.0,
        height: 929.0,
    };
    const SCREEN: WindowRect = WindowRect {
        x: 0.0,
        y: 0.0,
        width: 1680.0,
        height: 1050.0,
    };
    const MENU_BAR: f64 = 30.0;
    const START: WindowRect = WindowRect {
        x: 440.0,
        y: 113.0,
        width: 800.0,
        height: 600.0,
    };

    impl Sim {
        fn new(clock: &Rc<FakeClock>) -> Self {
            Self {
                clock: Rc::clone(clock),
                state: RefCell::new(State {
                    rect: START,
                    normal: START,
                    flags: WindowFlags::default(),
                    queue: Vec::new(),
                    busy_until: Duration::ZERO,
                    animation: None,
                    resizes_asked: 0,
                    resizes_to_lose: 0,
                    moves_asked: 0,
                    moves_to_lose: 0,
                    leaves_asked: 0,
                }),
                latency: Duration::from_millis(40),
                transition: Duration::from_millis(700),
                refuses_sizes: false,
                returns_maximized: false,
                holds_frame: false,
                ignores_leaving: false,
            }
        }

        fn starting_full_screen(self) -> Self {
            {
                let mut state = self.state.borrow_mut();
                state.flags.fullscreen = true;
                state.rect = SCREEN;
            }
            self
        }

        fn starting_maximized(self) -> Self {
            {
                let mut state = self.state.borrow_mut();
                state.flags.maximized = true;
                state.rect = MAXIMIZED;
            }
            self
        }

        fn starting_minimized(self) -> Self {
            self.state.borrow_mut().flags.minimized = true;
            self
        }

        fn losing_the_first_resizes(self, count: u32) -> Self {
            self.state.borrow_mut().resizes_to_lose = count;
            self
        }

        fn losing_the_first_moves(self, count: u32) -> Self {
            self.state.borrow_mut().moves_to_lose = count;
            self
        }

        fn holding_its_frame_while_it_leaves(mut self) -> Self {
            self.holds_frame = true;
            self
        }

        fn never_leaving_full_screen(mut self) -> Self {
            self.ignores_leaving = true;
            self
        }

        fn refusing_sizes(mut self) -> Self {
            self.refuses_sizes = true;
            self
        }

        fn coming_back_maximized(mut self) -> Self {
            self.returns_maximized = true;
            self
        }

        fn ask(&self, op: Op) {
            let due = self.clock.now() + self.latency;
            self.state.borrow_mut().queue.push((due, op));
        }

        /// Applies every call that is due.
        fn pump(&self) {
            let now = self.clock.now();
            let mut state = self.state.borrow_mut();
            let mut queue = std::mem::take(&mut state.queue);
            queue.sort_by_key(|(due, _)| *due);
            let (ready, later): (Vec<_>, Vec<_>) =
                queue.into_iter().partition(|(due, _)| *due <= now);
            state.queue = later;
            for (due, op) in ready {
                self.apply(&mut state, due, op);
            }
        }

        fn apply(&self, state: &mut State, due: Duration, op: Op) {
            let in_transition = due < state.busy_until;
            let normal_state = state.flags == WindowFlags::default();
            match op {
                Op::LeaveFullscreen => {
                    state.leaves_asked += 1;
                    if state.flags.fullscreen && !self.ignores_leaving {
                        // Like the window of Tauri, it says at once that it is
                        // not full screen, and goes on animating for a while.
                        state.flags.fullscreen = false;
                        let back = if self.returns_maximized {
                            state.flags.maximized = true;
                            MAXIMIZED
                        } else {
                            state.normal
                        };
                        state.animation = Some((due, due + self.transition, SCREEN, back));
                        state.rect = back;
                        state.busy_until = due + self.transition;
                    }
                }
                Op::EnterFullscreen => {
                    if !state.flags.fullscreen {
                        state.flags.fullscreen = true;
                        state.animation = None;
                        state.rect = SCREEN;
                        state.busy_until = due + self.transition;
                    }
                }
                Op::Unmaximize => {
                    if state.flags.maximized {
                        state.flags.maximized = false;
                        state.rect = state.normal;
                    }
                }
                Op::Maximize => {
                    state.flags.maximized = true;
                    state.rect = MAXIMIZED;
                }
                Op::Unminimize => state.flags.minimized = false,
                Op::Minimize => state.flags.minimized = true,
                Op::MoveTo(x, y) => {
                    state.moves_asked += 1;
                    if state.moves_to_lose > 0 {
                        state.moves_to_lose -= 1;
                    } else if !in_transition && !state.flags.fullscreen {
                        state.rect.x = x;
                        state.rect.y = y.max(MENU_BAR);
                        state.normal = state.rect;
                    }
                }
                Op::ResizeTo(width, height) => {
                    state.resizes_asked += 1;
                    if state.resizes_to_lose > 0 {
                        state.resizes_to_lose -= 1;
                    } else if !in_transition && normal_state && !self.refuses_sizes {
                        state.rect.width = width;
                        state.rect.height = height;
                        state.normal = state.rect;
                    }
                }
            }
        }
    }

    impl NativeWindow for Sim {
        fn flags(&self) -> Result<WindowFlags, WebDriverError> {
            self.pump();
            Ok(self.state.borrow().flags)
        }

        fn rect(&self) -> Result<WindowRect, WebDriverError> {
            self.pump();
            let state = self.state.borrow();
            let now = self.clock.now();
            Ok(match state.animation {
                Some((_, end, from, _)) if now < end && self.holds_frame => from,
                Some((start, end, from, to)) if now < end => {
                    let part = (now - start).as_secs_f64() / (end - start).as_secs_f64();
                    let between = |a: f64, b: f64| a + (b - a) * part;
                    WindowRect {
                        x: between(from.x, to.x),
                        y: between(from.y, to.y),
                        width: between(from.width, to.width),
                        height: between(from.height, to.height),
                    }
                }
                _ => state.rect,
            })
        }

        fn leave_fullscreen(&self) -> Result<(), WebDriverError> {
            self.ask(Op::LeaveFullscreen);
            Ok(())
        }

        fn enter_fullscreen(&self) -> Result<(), WebDriverError> {
            self.ask(Op::EnterFullscreen);
            Ok(())
        }

        fn unmaximize(&self) -> Result<(), WebDriverError> {
            self.ask(Op::Unmaximize);
            Ok(())
        }

        fn maximize(&self) -> Result<(), WebDriverError> {
            self.ask(Op::Maximize);
            Ok(())
        }

        fn unminimize(&self) -> Result<(), WebDriverError> {
            self.ask(Op::Unminimize);
            Ok(())
        }

        fn minimize(&self) -> Result<(), WebDriverError> {
            self.ask(Op::Minimize);
            Ok(())
        }

        fn move_to(&self, x: f64, y: f64) -> Result<(), WebDriverError> {
            self.ask(Op::MoveTo(x, y));
            Ok(())
        }

        fn resize_to(&self, width: f64, height: f64) -> Result<(), WebDriverError> {
            self.ask(Op::ResizeTo(width, height));
            Ok(())
        }
    }

    fn all(width: f64, height: f64) -> RectRequest {
        RectRequest {
            x: Some(0.0),
            y: Some(0.0),
            width: Some(width),
            height: Some(height),
        }
    }

    fn setup(build: impl FnOnce(Sim) -> Sim) -> (Rc<FakeClock>, Sim) {
        let clock = Rc::new(FakeClock::default());
        let sim = build(Sim::new(&clock));
        (clock, sim)
    }

    #[test]
    fn the_answer_is_the_window_after_the_change_landed_not_before() {
        // The calls land 40 ms after they are made: a read right after them
        // would still see the old rectangle.
        let (clock, sim) = setup(|sim| sim);
        let rect = set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!((rect.width, rect.height), (900.0, 700.0));
        // The system moved it below the menu bar, and the answer says so.
        assert_eq!((rect.x, rect.y), (0.0, MENU_BAR));
    }

    #[test]
    fn a_window_that_is_full_screen_is_resized_after_it_has_left() {
        // Leaving full screen takes 700 ms, and a resize asked meanwhile is lost.
        let (clock, sim) = setup(Sim::starting_full_screen);
        let rect = set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!((rect.width, rect.height), (900.0, 700.0));
        assert!(!sim.flags().unwrap().fullscreen);
    }

    #[test]
    fn a_maximized_window_is_resized_after_it_has_left_that_state() {
        let (clock, sim) = setup(Sim::starting_maximized);
        let rect = set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!((rect.width, rect.height), (900.0, 700.0));
        assert_eq!(sim.flags().unwrap(), WindowFlags::default());
    }

    #[test]
    fn a_window_that_comes_back_maximized_from_full_screen_is_restored_again() {
        let (clock, sim) = setup(|sim| sim.starting_full_screen().coming_back_maximized());
        let rect = set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!((rect.width, rect.height), (900.0, 700.0));
        assert_eq!(sim.flags().unwrap(), WindowFlags::default());
    }

    #[test]
    fn a_minimized_window_is_brought_back_and_resized() {
        let (clock, sim) = setup(Sim::starting_minimized);
        let rect = set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!((rect.width, rect.height), (900.0, 700.0));
        assert!(!sim.flags().unwrap().minimized);
    }

    #[test]
    fn a_resize_the_window_lost_is_asked_again() {
        let (clock, sim) = setup(|sim| sim.losing_the_first_resizes(1));
        let rect = set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!((rect.width, rect.height), (900.0, 700.0));
        assert_eq!(sim.state.borrow().resizes_asked, 2);
    }

    #[test]
    fn a_size_the_system_refuses_is_answered_as_it_is_and_after_a_bounded_wait() {
        let (clock, sim) = setup(Sim::refusing_sizes);
        let rect = set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!((rect.width, rect.height), (START.width, START.height));
        // Asked again every 250 ms until the wait is over, then given up.
        let asked = sim.state.borrow().resizes_asked;
        assert!((3..=10).contains(&asked), "asked {asked} times");
        assert!(
            clock.now() <= RECT_TIMEOUT + Duration::from_millis(100),
            "waited {:?}",
            clock.now()
        );
    }

    #[test]
    fn a_moved_window_the_system_keeps_in_place_does_not_make_the_command_wait() {
        // Only the position is asked for, to a place above the menu bar: the
        // window ends up below it, and the answer comes as soon as it stays.
        let (clock, sim) = setup(|sim| sim);
        let request = RectRequest {
            x: Some(10.0),
            y: Some(0.0),
            width: None,
            height: None,
        };
        let rect = set_rect(&sim, &*clock, request).unwrap();
        assert_eq!((rect.x, rect.y), (10.0, MENU_BAR));
        assert_eq!((rect.width, rect.height), (START.width, START.height));
        assert!(
            clock.now() < Duration::from_millis(400),
            "{:?}",
            clock.now()
        );
        assert_eq!(sim.state.borrow().resizes_asked, 0);
    }

    #[test]
    fn a_value_that_is_not_given_stays() {
        let (clock, sim) = setup(|sim| sim);
        let request = RectRequest {
            x: None,
            y: None,
            width: Some(900.0),
            height: None,
        };
        let rect = set_rect(&sim, &*clock, request).unwrap();
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (START.x, START.y, 900.0, START.height)
        );
    }

    #[test]
    fn nothing_asked_changes_nothing_but_the_state() {
        let (clock, sim) = setup(Sim::starting_maximized);
        let request = RectRequest {
            x: None,
            y: None,
            width: None,
            height: None,
        };
        let rect = set_rect(&sim, &*clock, request).unwrap();
        assert_eq!(sim.flags().unwrap(), WindowFlags::default());
        assert_eq!((rect.width, rect.height), (START.width, START.height));
    }

    #[test]
    fn maximize_leaves_full_screen_first_and_answers_with_the_maximized_window() {
        let (clock, sim) = setup(Sim::starting_full_screen);
        let rect = maximize(&sim, &*clock).unwrap();
        let flags = sim.flags().unwrap();
        assert!(flags.maximized && !flags.fullscreen);
        assert_eq!((rect.width, rect.height), (1680.0, 929.0));
    }

    #[test]
    fn minimize_answers_once_the_window_is_minimized() {
        let (clock, sim) = setup(|sim| sim);
        minimize(&sim, &*clock).unwrap();
        assert!(sim.flags().unwrap().minimized);
    }

    #[test]
    fn fullscreen_answers_with_the_screen_once_the_window_is_full_screen() {
        let (clock, sim) = setup(|sim| sim);
        let rect = fullscreen(&sim, &*clock).unwrap();
        assert!(sim.flags().unwrap().fullscreen);
        assert_eq!((rect.width, rect.height), (SCREEN.width, SCREEN.height));
    }

    #[test]
    fn a_window_that_keeps_its_old_frame_while_it_leaves_full_screen_gets_the_size_in_the_end() {
        // The window says at once it is not full screen but its frame does not
        // move for 700 ms, so the frame never looks like one in motion, and the
        // first requests are lost: they are made again until one lands.
        let (clock, sim) = setup(|sim| {
            sim.starting_full_screen()
                .holding_its_frame_while_it_leaves()
        });
        let rect = set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!((rect.width, rect.height), (900.0, 700.0));
        assert!(
            clock.now() <= STATE_TIMEOUT + RECT_TIMEOUT,
            "waited {:?}",
            clock.now()
        );
    }

    #[test]
    fn a_move_the_window_lost_is_asked_again() {
        let (clock, sim) = setup(|sim| sim.losing_the_first_moves(1));
        let request = RectRequest {
            x: Some(10.0),
            y: Some(200.0),
            width: None,
            height: None,
        };
        let rect = set_rect(&sim, &*clock, request).unwrap();
        assert_eq!((rect.x, rect.y), (10.0, 200.0));
        assert_eq!(sim.state.borrow().moves_asked, 2);
    }

    #[test]
    fn a_window_that_does_not_leave_full_screen_is_asked_once_and_used_as_it_is() {
        let (clock, sim) = setup(|sim| sim.starting_full_screen().never_leaving_full_screen());
        set_rect(&sim, &*clock, all(900.0, 700.0)).unwrap();
        assert_eq!(sim.state.borrow().leaves_asked, 1);
        assert!(
            clock.now() <= STATE_TIMEOUT + RECT_TIMEOUT + Duration::from_millis(200),
            "waited {:?}",
            clock.now()
        );
    }
}
