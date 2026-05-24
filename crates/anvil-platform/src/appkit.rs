//! AppKit platform shell: NSApplication bootstrap, NSWindow creation, custom
//! NSView subclass for input, app/window delegates, and the NSTimer run loop.
//!
//! Port of the AppKit glue in `src/main.zig` (the Objective-C method
//! implementations, class registration, and `main()` setup).  The application
//! *logic* (terminal, renderer, tab management) lives in the `anvil` binary
//! and is reached through the [`AppHandler`] callback trait.
//!
//! # Architecture
//!
//! `anvil-platform` must not depend on the `anvil` binary (that would be a
//! cycle).  Instead, the `anvil` binary implements [`AppHandler`] on its `App`
//! type and passes a `Rc<RefCell<dyn AppHandler>>` into [`AppKitApp::new`].
//!
//! The NSView subclass and the delegate subclass each store a raw pointer to
//! the `Rc<RefCell<dyn AppHandler>>` in their ivars.  The pointer is valid
//! for the lifetime of the app because [`AppKitApp`] owns the `Rc` and
//! `run()` blocks until the app exits.
//!
//! # Safety
//!
//! All `unsafe` blocks are annotated with a SAFETY comment.  The main
//! invariants are:
//! - All ObjC methods are invoked on the main thread (enforced by
//!   `MainThreadMarker` / `MainThreadOnly`).
//! - The raw handler pointer lives as long as the run loop.
//! - `define_class!` bodies mirror the signatures in the AppKit headers.

use std::cell::RefCell;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSBackingStoreType, NSEvent, NSEventModifierFlags, NSImage, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSNotification, NSPoint, NSRect, NSSize, NSString, NSTimer,
};

// ── event types delivered to the AppHandler ──────────────────────────────────

/// Keyboard modifier flags decoded from an [`NSEvent`], objc2-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub option: bool,
    pub command: bool,
}

/// A decoded key event: either a named key or a Unicode codepoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    F(u8), // F1..F12
}

/// A decoded key-down event.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub key: KeyInput,
    pub mods: Modifiers,
}

/// A decoded mouse event location in AppKit view-points (not device pixels).
#[derive(Debug, Clone, Copy)]
pub struct MouseLocation {
    /// x in view-point coordinates (origin = view bottom-left).
    pub x: f64,
    /// y in view-point coordinates (origin = view bottom-left).
    pub y: f64,
}

// ── AppHandler trait ──────────────────────────────────────────────────────────

/// Callback trait that the `anvil` binary implements on its `App` type.
///
/// The platform layer calls these methods on the main thread.  Implementations
/// must not call back into any AppKit API (those live in the platform layer).
pub trait AppHandler {
    /// Called ~60 times per second by the NSTimer.
    fn tick(&mut self);

    /// A key-down event from the NSView.  Modifier-only events are filtered;
    /// ⌘-combos with no codepoint are delivered with `key = None`.
    fn key_down(&mut self, event: KeyEvent);

    /// A `performKeyEquivalent:` event (runs before `keyDown:`, catches Tab).
    /// Returns `true` if the event was consumed.
    fn perform_key_equivalent(&mut self, event: KeyEvent) -> bool;

    /// Left mouse button pressed. `click_count` is `NSEvent.clickCount` —
    /// 1 for a single click, 2 for a double-click, 3 for triple, etc.
    fn mouse_down(
        &mut self,
        loc: MouseLocation,
        mods: Modifiers,
        click_count: u32,
        view_bounds: (f64, f64),
    );

    /// Left mouse button released.
    fn mouse_up(&mut self, loc: MouseLocation, mods: Modifiers);

    /// Mouse dragged (button held).
    fn mouse_dragged(&mut self, loc: MouseLocation);

    /// Scroll-wheel event. `dy` is the AppKit delta along Y. `pixel_precise`
    /// is true for trackpad / Magic Mouse (delta is in points), false for a
    /// traditional mouse wheel (delta is in "lines" — ~1.0 per detent).
    fn scroll(&mut self, dy: f64, pixel_precise: bool, loc: MouseLocation);

    /// The window was resized (called on every resize notification; also during
    /// live resize).  `in_live_resize` matches `NSView.inLiveResize`.
    fn resize(&mut self, width: f64, height: f64, in_live_resize: bool);

    /// A live-resize drag finished (`viewDidEndLiveResize`).
    fn live_resize_ended(&mut self);

    /// The window became the key window.
    fn focus_gained(&mut self);

    /// The window resigned key status.
    fn focus_lost(&mut self);

    /// The app/last-window is about to terminate.  Return `false` to cancel.
    fn should_terminate(&mut self) -> bool;

    /// A script message arrived from the WKWebView JS bridge.
    /// `json` is the raw UTF-8 JSON string posted by
    /// `window.webkit.messageHandlers.anvil.postMessage(…)`.
    fn webview_message(&mut self, json: String);
}

// ── NSEvent decoding ──────────────────────────────────────────────────────────

/// Decode `NSEvent.modifierFlags` into [`Modifiers`].
///
/// This is public so it can be called from tests without AppKit running.
pub fn decode_mods(flags: NSEventModifierFlags) -> Modifiers {
    Modifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        control: flags.contains(NSEventModifierFlags::Control),
        option: flags.contains(NSEventModifierFlags::Option),
        command: flags.contains(NSEventModifierFlags::Command),
    }
}

/// Map a virtual keyCode to a named [`KeyInput`], or `None` for character keys.
///
/// This is public so it can be tested without AppKit running.
pub fn decode_keycode(keycode: u16) -> Option<KeyInput> {
    Some(match keycode {
        36 | 76 => KeyInput::Enter,
        48 => KeyInput::Tab,
        51 => KeyInput::Backspace,
        53 => KeyInput::Escape,
        123 => KeyInput::Left,
        124 => KeyInput::Right,
        125 => KeyInput::Down,
        126 => KeyInput::Up,
        115 => KeyInput::Home,
        119 => KeyInput::End,
        116 => KeyInput::PageUp,
        121 => KeyInput::PageDown,
        117 => KeyInput::Delete,
        // F-keys
        122 => KeyInput::F(1),
        120 => KeyInput::F(2),
        99 => KeyInput::F(3),
        118 => KeyInput::F(4),
        96 => KeyInput::F(5),
        97 => KeyInput::F(6),
        98 => KeyInput::F(7),
        100 => KeyInput::F(8),
        101 => KeyInput::F(9),
        109 => KeyInput::F(10),
        103 => KeyInput::F(11),
        111 => KeyInput::F(12),
        _ => return None,
    })
}

/// Extract the first `char` from an `NSString` via an autoreleasepool.
///
/// Public for use in tests.
pub fn first_char_of_nsstring(s: &NSString) -> Option<char> {
    objc2::rc::autoreleasepool(|pool| {
        // SAFETY: pool is live for this closure; to_str borrows it.
        let utf8 = unsafe { s.to_str(pool) };
        utf8.chars().next()
    })
}

/// Decode an NSEvent into a [`KeyEvent`], using the same logic as the Zig
/// `extractKey` function.  Returns `None` when no meaningful key can be
/// extracted (e.g. dead-key, null string).
pub(crate) fn decode_key_event(event: &NSEvent) -> Option<KeyEvent> {
    let flags = event.modifierFlags();
    let mods = decode_mods(flags);

    let keycode = event.keyCode();
    if let Some(named) = decode_keycode(keycode) {
        return Some(KeyEvent { key: named, mods });
    }

    // For control/option combos use charactersIgnoringModifiers so the raw
    // letter is delivered, not the control character.
    let nsstr = if mods.control || mods.option {
        event.charactersIgnoringModifiers()
    } else {
        event.characters()
    };
    let nsstr = nsstr?;
    let ch = first_char_of_nsstring(&nsstr)?;
    Some(KeyEvent {
        key: KeyInput::Char(ch),
        mods,
    })
}

/// Decode an NSEvent into a key event for `performKeyEquivalent:` — always
/// uses `charactersIgnoringModifiers` so the base letter is delivered.
pub(crate) fn decode_key_equivalent(event: &NSEvent) -> Option<KeyEvent> {
    let flags = event.modifierFlags();
    let mods = decode_mods(flags);
    let keycode = event.keyCode();
    let nsstr = event.charactersIgnoringModifiers()?;
    let ch = first_char_of_nsstring(&nsstr)?;
    // Also check named keys (e.g. Tab = keycode 48).
    let key = decode_keycode(keycode).unwrap_or(KeyInput::Char(ch));
    Some(KeyEvent { key, mods })
}

// ── Handler pointer wrapper ───────────────────────────────────────────────────

/// A thin wrapper that stores a raw pointer to `Rc<RefCell<dyn AppHandler>>`.
/// Used as the ivar type in both the view and delegate subclasses.
///
/// # Safety
///
/// The pointer is only ever accessed on the main thread while the
/// `AppKitApp` (which owns the `Rc`) is live.
struct HandlerPtr(*mut Rc<RefCell<dyn AppHandler>>);

// SAFETY: ObjC ivar types must be Send; the pointer is only dereferenced on
// the main thread so no actual cross-thread access occurs.
unsafe impl Send for HandlerPtr {}
unsafe impl Sync for HandlerPtr {}

impl HandlerPtr {
    /// Borrow the handler mutably.
    ///
    /// # Safety
    ///
    /// Caller must ensure no other borrow is active — we never call
    /// `borrow_mut` recursively from within a handler method.
    unsafe fn borrow_mut(&self) -> std::cell::RefMut<'_, dyn AppHandler> {
        // SAFETY: the Rc lives for the duration of the app run loop.
        unsafe { (*self.0).borrow_mut() }
    }
}

// ── NSView subclass (AnvilTerminalView) ───────────────────────────────────────

struct ViewIvars {
    handler: HandlerPtr,
}

define_class!(
    // SAFETY:
    // - NSView does not have special subclassing requirements.
    // - ViewIvars is only accessed on the main thread.
    #[unsafe(super(NSView, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewIvars]
    #[name = "AnvilTerminalView"]
    struct AnvilView;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for AnvilView {}

    impl AnvilView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let Some(ev) = decode_key_event(event) else { return };
            // SAFETY: handler pointer is valid for the app lifetime.
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.key_down(ev);
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let Some(ev) = decode_key_equivalent(event) else {
                return false.into();
            };
            // SAFETY: handler pointer is valid for the app lifetime.
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.perform_key_equivalent(ev)
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let dy = event.scrollingDeltaY();
            // Use the TYPED binding — the previous `msg_send![event,
            // hasPreciseScrollingDeltas]` returned garbage for the bool
            // (BOOL is a signed char, not a Rust bool), so every event
            // was misclassified as line-mode regardless of input source.
            let pixel_precise = event.hasPreciseScrollingDeltas();
            let loc = location_in_view(self, event);
            // SAFETY: handler pointer is valid for the app lifetime.
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.scroll(dy, pixel_precise, loc);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let mods = decode_mods(event.modifierFlags());
            let loc = location_in_view(self, event);
            let bounds = self.bounds();
            let cc = event.clickCount() as u32;
            // SAFETY: handler pointer is valid for the app lifetime.
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.mouse_down(loc, mods, cc, (bounds.size.width, bounds.size.height));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let loc = location_in_view(self, event);
            // SAFETY: handler pointer is valid for the app lifetime.
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.mouse_dragged(loc);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let mods = decode_mods(event.modifierFlags());
            let loc = location_in_view(self, event);
            // SAFETY: handler pointer is valid for the app lifetime.
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.mouse_up(loc, mods);
        }

        #[unsafe(method(viewDidEndLiveResize))]
        fn view_did_end_live_resize(&self) {
            // SAFETY: handler pointer is valid for the app lifetime.
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.live_resize_ended();
        }

        // AppKit dispatches Esc through `-cancelOperation:` first (it's
        // the "cancel" responder action). Without an explicit override
        // the key sometimes never reaches `-keyDown:`, so dismissals like
        // "Esc closes the cheatsheet/HUD" feel broken. Synthesize a key
        // event and route it through the handler's normal key path.
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: *mut objc2::runtime::AnyObject) {
            let ev = KeyEvent {
                key: KeyInput::Escape,
                mods: Modifiers::default(),
            };
            // SAFETY: handler pointer is valid for the app lifetime.
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.key_down(ev);
        }
    }
);

/// Extract the event location converted to the view's coordinate space.
fn location_in_view(view: &NSView, event: &NSEvent) -> MouseLocation {
    let win_pt = event.locationInWindow();
    let view_pt = view.convertPoint_fromView(win_pt, None);
    MouseLocation {
        x: view_pt.x,
        y: view_pt.y,
    }
}

// ── Delegate subclass (AnvilDelegate) ────────────────────────────────────────

struct DelegateIvars {
    handler: HandlerPtr,
    /// Retained reference to the view for resize callbacks.
    view: std::cell::OnceCell<Retained<NSView>>,
}

define_class!(
    // SAFETY:
    // - NSObject does not have special subclassing requirements.
    // - DelegateIvars is only accessed on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    #[name = "AnvilDelegate"]
    struct AnvilDelegate;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for AnvilDelegate {}

    // SAFETY: NSApplicationDelegate has no safety requirements.
    unsafe impl NSApplicationDelegate for AnvilDelegate {
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
        }
    }

    // SAFETY: NSWindowDelegate has no safety requirements.
    unsafe impl NSWindowDelegate for AnvilDelegate {
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            // Deliver the current content-view bounds.
            if let Some(view) = self.ivars().view.get() {
                let b = view.bounds();
                let in_live = view.inLiveResize();
                let mut h = unsafe { self.ivars().handler.borrow_mut() };
                h.resize(b.size.width, b.size.height, in_live);
            }
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.focus_gained();
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            let mut h = unsafe { self.ivars().handler.borrow_mut() };
            h.focus_lost();
        }

        // When the user clicks the red traffic-light button to close the
        // window we want the process to actually exit. The default AppKit
        // path (applicationShouldTerminateAfterLastWindowClosed → true)
        // doesn't always finalize cleanly when long-lived background
        // threads (PTY readers, git/recent-files workers, caldera poller)
        // hold non-detached handles. Force the issue.
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            std::process::exit(0);
        }
    }
);

// ── Public AppKitApp ──────────────────────────────────────────────────────────

/// The AppKit shell.  Create with [`AppKitApp::new`], then call [`run`].
///
/// `handler` is called back from the main thread on every platform event.
pub struct AppKitApp {
    /// The shared NSApplication.
    pub nsapp: Retained<NSApplication>,
    /// The terminal content view (Metal-backed, handles input).
    pub view: Retained<NSView>,
    /// The window.
    pub window: Retained<NSWindow>,
    /// The owning `Rc` for the handler — kept alive for the run loop.
    _handler_rc: Rc<RefCell<dyn AppHandler>>,
}

/// Bytes of the bundled application icon (PNG).
const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon.png");

impl AppKitApp {
    /// Bootstrap the AppKit shell.
    ///
    /// `handler` is the application logic object; it will be called back from
    /// the main thread for every event during `run()`.
    ///
    /// `width` / `height` are the initial window content rect in points.
    /// `title` is the window title.
    ///
    /// # Panics
    ///
    /// Panics if called from any thread other than the main thread, or if
    /// AppKit fails to create the window.
    pub fn new(handler: Rc<RefCell<dyn AppHandler>>, width: f64, height: f64, title: &str) -> Self {
        let mtm =
            MainThreadMarker::new().expect("AppKitApp::new must be called on the main thread");

        // ── NSApplication ────────────────────────────────────────────────────
        let nsapp = NSApplication::sharedApplication(mtm);
        nsapp.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        set_application_icon(&nsapp, mtm);

        // ── NSWindow ─────────────────────────────────────────────────────────
        // FullSizeContentView + transparent title bar: our raster spans the
        // whole window, traffic lights overlay the top-left of our chrome
        // row. Window appearance is forced to dark (below) so the title-bar
        // material renders dark regardless of the user's system mode —
        // matching Warp / Zed's approach.
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable
            | NSWindowStyleMask::FullSizeContentView;

        let content_rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));

        // SAFETY: initWithContentRect_styleMask_backing_defer is the standard
        // NSWindow designated initialiser; all arguments are valid.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content_rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // Disable auto-release-when-closed so we control the lifetime.
        // SAFETY: setReleasedWhenClosed is safe to call on a retained window.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        window.setTitlebarAppearsTransparent(true);
        // Force dark appearance: title-bar material renders dark even if the
        // user's macOS appearance is Light. Matches Warp / Zed.
        let dark = NSAppearance::appearanceNamed(&NSString::from_str("NSAppearanceNameDarkAqua"));
        if let Some(dark) = dark {
            // setAppearance: lives on NSAppearanceCustomization, not NSWindow
            // directly in objc2's bindings — go through msg_send.
            let _: () = unsafe { msg_send![&*window, setAppearance: &*dark] };
        }

        // ── Handler Rc → raw pointer for ivars ───────────────────────────────
        // We box each clone of the Rc and take a raw pointer.  The boxes are
        // freed when `AppKitApp` is dropped (after the run loop exits).
        let view_box: Box<Rc<RefCell<dyn AppHandler>>> = Box::new(Rc::clone(&handler));
        let view_ptr = Box::into_raw(view_box);

        let delegate_box: Box<Rc<RefCell<dyn AppHandler>>> = Box::new(Rc::clone(&handler));
        let delegate_ptr = Box::into_raw(delegate_box);

        // ── AnvilTerminalView ─────────────────────────────────────────────────
        let view_ivars = ViewIvars {
            handler: HandlerPtr(view_ptr),
        };
        let view_alloc = AnvilView::alloc(mtm);
        let view_with_ivars = view_alloc.set_ivars(view_ivars);
        // SAFETY: initWithFrame is the NSView designated initialiser.
        let anvil_view: Retained<AnvilView> =
            unsafe { msg_send![super(view_with_ivars), initWithFrame: content_rect] };

        // Give the view a Metal-backing CAMetalLayer.
        // SAFETY: calling +layer on CAMetalLayer and -setLayer:/-setWantsLayer:
        // are all safe on the main thread.
        unsafe {
            let cls_name = c"CAMetalLayer";
            let cls =
                objc2::runtime::AnyClass::get(cls_name).expect("CAMetalLayer must be available");
            let layer: Retained<NSObject> = msg_send![cls, layer];
            let _: () = msg_send![&*anvil_view, setLayer: &*layer];
            let _: () = msg_send![&*anvil_view, setWantsLayer: true];
        }

        // Cast to NSView for window / delegate APIs.
        // SAFETY: AnvilView IS an NSView (defined as such in define_class!).
        let view: Retained<NSView> = unsafe { Retained::cast_unchecked(anvil_view) };

        // ── AnvilDelegate ─────────────────────────────────────────────────────
        let delegate_ivars = DelegateIvars {
            handler: HandlerPtr(delegate_ptr),
            view: std::cell::OnceCell::new(),
        };
        let delegate_alloc = AnvilDelegate::alloc(mtm);
        let delegate_with_ivars = delegate_alloc.set_ivars(delegate_ivars);
        // SAFETY: NSObject's init signature is correct.
        let anvil_delegate: Retained<AnvilDelegate> =
            unsafe { msg_send![super(delegate_with_ivars), init] };

        // Give the delegate a retained reference to the view for resize callbacks.
        anvil_delegate
            .ivars()
            .view
            .set(view.clone())
            .expect("view OnceCell must be set exactly once");

        // ── Wire window, view, delegate ───────────────────────────────────────
        window.setContentView(Some(&view));

        // SAFETY: ProtocolObject::from_ref coerces AnvilDelegate (which
        // implements NSWindowDelegate) to the protocol object — safe.
        let delegate_as_win: &ProtocolObject<dyn NSWindowDelegate> =
            ProtocolObject::from_ref(&*anvil_delegate);
        window.setDelegate(Some(delegate_as_win));

        let delegate_as_app: &ProtocolObject<dyn NSApplicationDelegate> =
            ProtocolObject::from_ref(&*anvil_delegate);
        nsapp.setDelegate(Some(delegate_as_app));

        window.center();
        window.makeKeyAndOrderFront(None);

        // Give the terminal view first-responder status for keyboard input.
        // SAFETY: makeFirstResponder is safe on the MT when the view is in the window.
        unsafe {
            use objc2_app_kit::NSResponder;
            window.makeFirstResponder(Some(
                &*(view.as_ref() as *const NSView as *const NSResponder),
            ))
        };

        // ── NSTimer at ~60 fps ────────────────────────────────────────────────
        // `scheduledTimerWithTimeInterval:repeats:block:` fires on the main
        // run loop; we capture a clone of the handler Rc in an RcBlock.
        let timer_box: Box<Rc<RefCell<dyn AppHandler>>> = Box::new(Rc::clone(&handler));
        let timer_ptr = Box::into_raw(timer_box);

        let tick_block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| {
            let rc: &Rc<RefCell<dyn AppHandler>> = unsafe { &*timer_ptr };
            rc.borrow_mut().tick();
        });

        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0 / 60.0, true, &tick_block)
        };

        // ── Activate ─────────────────────────────────────────────────────────
        // Required when launching unbundled (e.g. via cargo run).
        #[allow(deprecated)]
        nsapp.activateIgnoringOtherApps(true);

        Self {
            nsapp,
            view,
            window,
            _handler_rc: handler,
        }
    }

    /// Enter the NSApplication run loop.  This function does not return until
    /// the application exits.
    pub fn run(self) {
        self.nsapp.run();
    }

    /// Return the Metal backing-scale factor of the window.
    pub fn backing_scale_factor(&self) -> f64 {
        self.window.backingScaleFactor()
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Set the application icon from the bundled PNG bytes.
fn set_application_icon(app: &NSApplication, _mtm: MainThreadMarker) {
    // SAFETY: dataWithBytes_length takes a non-null const pointer + length.
    let data = unsafe {
        NSData::dataWithBytes_length(
            APP_ICON_PNG.as_ptr() as *const std::ffi::c_void,
            APP_ICON_PNG.len(),
        )
    };
    // SAFETY: initWithData is the NSImage designated initialiser for raw data.
    // NSImage implements AnyThread (not MainThreadOnly), so use AnyThread::alloc.
    let image = NSImage::initWithData(NSImage::alloc(), &data);
    if let Some(img) = image {
        // SAFETY: setApplicationIconImage is safe on the MT.
        unsafe { app.setApplicationIconImage(Some(&*img)) };
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── modifier decoding ────────────────────────────────────────────────────

    #[test]
    fn decode_mods_none() {
        let flags = NSEventModifierFlags(0);
        let mods = decode_mods(flags);
        assert!(!mods.shift && !mods.control && !mods.option && !mods.command);
    }

    #[test]
    fn decode_mods_shift() {
        let flags = NSEventModifierFlags::Shift;
        let mods = decode_mods(flags);
        assert!(mods.shift);
        assert!(!mods.control && !mods.option && !mods.command);
    }

    #[test]
    fn decode_mods_control() {
        let flags = NSEventModifierFlags::Control;
        let mods = decode_mods(flags);
        assert!(mods.control);
        assert!(!mods.shift && !mods.option && !mods.command);
    }

    #[test]
    fn decode_mods_option() {
        let flags = NSEventModifierFlags::Option;
        let mods = decode_mods(flags);
        assert!(mods.option);
        assert!(!mods.shift && !mods.control && !mods.command);
    }

    #[test]
    fn decode_mods_command() {
        let flags = NSEventModifierFlags::Command;
        let mods = decode_mods(flags);
        assert!(mods.command);
        assert!(!mods.shift && !mods.control && !mods.option);
    }

    #[test]
    fn decode_mods_all() {
        let flags = NSEventModifierFlags::Shift
            | NSEventModifierFlags::Control
            | NSEventModifierFlags::Option
            | NSEventModifierFlags::Command;
        let mods = decode_mods(flags);
        assert!(mods.shift && mods.control && mods.option && mods.command);
    }

    // ── keycode mapping ──────────────────────────────────────────────────────

    #[test]
    fn decode_keycode_enter() {
        assert_eq!(decode_keycode(36), Some(KeyInput::Enter));
        assert_eq!(decode_keycode(76), Some(KeyInput::Enter)); // numpad enter
    }

    #[test]
    fn decode_keycode_tab() {
        assert_eq!(decode_keycode(48), Some(KeyInput::Tab));
    }

    #[test]
    fn decode_keycode_backspace() {
        assert_eq!(decode_keycode(51), Some(KeyInput::Backspace));
    }

    #[test]
    fn decode_keycode_escape() {
        assert_eq!(decode_keycode(53), Some(KeyInput::Escape));
    }

    #[test]
    fn decode_keycode_arrows() {
        assert_eq!(decode_keycode(123), Some(KeyInput::Left));
        assert_eq!(decode_keycode(124), Some(KeyInput::Right));
        assert_eq!(decode_keycode(125), Some(KeyInput::Down));
        assert_eq!(decode_keycode(126), Some(KeyInput::Up));
    }

    #[test]
    fn decode_keycode_navigation() {
        assert_eq!(decode_keycode(115), Some(KeyInput::Home));
        assert_eq!(decode_keycode(119), Some(KeyInput::End));
        assert_eq!(decode_keycode(116), Some(KeyInput::PageUp));
        assert_eq!(decode_keycode(121), Some(KeyInput::PageDown));
        assert_eq!(decode_keycode(117), Some(KeyInput::Delete));
    }

    #[test]
    fn decode_keycode_f_keys() {
        assert_eq!(decode_keycode(122), Some(KeyInput::F(1)));
        assert_eq!(decode_keycode(120), Some(KeyInput::F(2)));
        assert_eq!(decode_keycode(99), Some(KeyInput::F(3)));
        assert_eq!(decode_keycode(118), Some(KeyInput::F(4)));
        assert_eq!(decode_keycode(96), Some(KeyInput::F(5)));
        assert_eq!(decode_keycode(97), Some(KeyInput::F(6)));
        assert_eq!(decode_keycode(98), Some(KeyInput::F(7)));
        assert_eq!(decode_keycode(100), Some(KeyInput::F(8)));
        assert_eq!(decode_keycode(101), Some(KeyInput::F(9)));
        assert_eq!(decode_keycode(109), Some(KeyInput::F(10)));
        assert_eq!(decode_keycode(103), Some(KeyInput::F(11)));
        assert_eq!(decode_keycode(111), Some(KeyInput::F(12)));
    }

    #[test]
    fn decode_keycode_unknown_returns_none() {
        // Character key codes (e.g. 0 = 'a', 11 = 'b') are not named keys.
        assert_eq!(decode_keycode(0), None);
        assert_eq!(decode_keycode(11), None);
        assert_eq!(decode_keycode(49), None); // space
    }

    // ── Modifiers struct ─────────────────────────────────────────────────────

    #[test]
    fn modifiers_default_all_false() {
        let m = Modifiers::default();
        assert!(!m.shift && !m.control && !m.option && !m.command);
    }
}
