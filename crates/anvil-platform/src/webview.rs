//! WKWebView host: embeds one transparent WKWebView above the Metal terminal
//! view, registers the web→native script-message handler, and exposes
//! `show` / `hide` / `set_frame` / `eval_js`.
//!
//! Port of `src/webview/webview.zig`.  Uses `objc2-web-kit` 0.3 typed bindings.
//!
//! Script messages (JSON strings posted by
//! `window.webkit.messageHandlers.anvil.postMessage(…)`) are forwarded to the
//! [`AppHandler::webview_message`] callback.
//!
//! # Safety
//!
//! All ObjC calls happen on the main thread.  The handler Rc pointer in the
//! script-message handler ivar is written once at startup and read from the
//! main-thread callback, so no data race occurs.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSResponder, NSView, NSWindow};
use objc2_foundation::{MainThreadMarker, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::{
    WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKWebView,
    WKWebViewConfiguration,
};

use crate::appkit::AppHandler;

// ── Script-message handler ────────────────────────────────────────────────────

struct ScriptHandlerIvars {
    /// Raw pointer to the handler Rc, valid for app lifetime.
    handler: *mut Rc<RefCell<dyn AppHandler>>,
}

// SAFETY: the pointer is only dereferenced on the main thread.
unsafe impl Send for ScriptHandlerIvars {}
unsafe impl Sync for ScriptHandlerIvars {}

define_class!(
    // SAFETY:
    // - NSObject has no special subclassing requirements.
    // - ScriptHandlerIvars is only accessed on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ScriptHandlerIvars]
    #[name = "AnvilScriptHandler"]
    struct AnvilScriptHandler;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for AnvilScriptHandler {}

    // SAFETY: WKScriptMessageHandler has no safety requirements.
    unsafe impl WKScriptMessageHandler for AnvilScriptHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive_script_message(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
        ) {
            // SAFETY: `body` returns the JS value (always an NSString here).
            let body_obj: Retained<AnyObject> = unsafe { message.body() };

            // Downcast AnyObject → NSString via the UTF8String message.
            // The web surface always posts a string, so this is safe per contract.
            let json: String = objc2::rc::autoreleasepool(|_pool| {
                // SAFETY: we send UTF8String to an object we know is NSString.
                let cstr: *const std::ffi::c_char = unsafe { msg_send![&*body_obj, UTF8String] };
                if cstr.is_null() {
                    return String::new();
                }
                // SAFETY: cstr is a valid null-terminated UTF-8 C string.
                let s = unsafe { std::ffi::CStr::from_ptr(cstr) };
                s.to_string_lossy().into_owned()
            });

            if json.is_empty() {
                return;
            }

            // SAFETY: handler pointer lives for app lifetime; we're on the MT.
            let rc: &Rc<RefCell<dyn AppHandler>> = unsafe { &*self.ivars().handler };
            rc.borrow_mut().webview_message(json);
        }
    }
);

// ── Webview ───────────────────────────────────────────────────────────────────

/// Configuration for [`Webview::init`].
pub struct WebviewConfig<'a> {
    pub window: Retained<NSWindow>,
    pub container: &'a NSView,
    pub terminal_view: Retained<NSView>,
    pub width: f64,
    pub height: f64,
    pub html: &'a str,
    /// Raw pointer to the handler Rc; must remain valid for the Webview lifetime.
    pub handler_ptr: *mut Rc<RefCell<dyn AppHandler>>,
    pub mtm: MainThreadMarker,
}

/// A hosted WKWebView sitting above the Metal terminal view.
///
/// Create via [`Webview::init`]; then call `show` / `hide` / `eval_js` from
/// the `anvil` binary as needed.
pub struct Webview {
    /// The underlying WKWebView object.
    obj: Retained<WKWebView>,
    /// The NSWindow (needed to adjust first-responder on show/hide).
    window: Retained<NSWindow>,
    /// The terminal view (needed to return focus on hide).
    terminal_view: Retained<NSView>,
}

impl Webview {
    /// Create the WKWebView, add it as a hidden subview of `config.container`,
    /// and load `config.html`.
    pub fn init(config: WebviewConfig<'_>) -> Self {
        let WebviewConfig {
            window,
            container,
            terminal_view,
            width,
            height,
            html,
            handler_ptr,
            mtm,
        } = config;
        // ── Script-message handler ────────────────────────────────────────────
        let sh_ivars = ScriptHandlerIvars {
            handler: handler_ptr,
        };
        let sh_alloc = AnvilScriptHandler::alloc(mtm);
        let sh_with_ivars = sh_alloc.set_ivars(sh_ivars);
        // SAFETY: NSObject's init signature is correct.
        let script_handler: Retained<AnvilScriptHandler> =
            unsafe { msg_send![super(sh_with_ivars), init] };

        // ── WKUserContentController ───────────────────────────────────────────
        // SAFETY: init is the WKUserContentController designated initialiser.
        let ucc = unsafe { WKUserContentController::init(WKUserContentController::alloc(mtm)) };

        let handler_proto: &ProtocolObject<dyn WKScriptMessageHandler> =
            ProtocolObject::from_ref(&*script_handler);
        let message_name = NSString::from_str("anvil");
        // SAFETY: addScriptMessageHandler_name is safe when handler and name are valid.
        unsafe { ucc.addScriptMessageHandler_name(handler_proto, &message_name) };

        // ── WKWebViewConfiguration ────────────────────────────────────────────
        // SAFETY: init is the WKWebViewConfiguration designated initialiser.
        let config = unsafe { WKWebViewConfiguration::init(WKWebViewConfiguration::alloc(mtm)) };
        // SAFETY: setUserContentController is safe on a freshly-allocated config.
        unsafe { config.setUserContentController(&ucc) };

        // ── WKWebView ─────────────────────────────────────────────────────────
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
        // SAFETY: initWithFrame_configuration is the WKWebView designated init.
        let webview = unsafe {
            WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config)
        };

        // Make the webview transparent (the dim backdrop shows the terminal).
        // `drawsBackground` is a private/non-standard property set via KVC.
        // setValue:forKey: takes an object, so the bool is boxed in an NSNumber.
        // SAFETY: setValue:forKey: is an NSObject method; both args are valid.
        unsafe {
            let no = NSNumber::numberWithBool(false);
            let key = NSString::from_str("drawsBackground");
            let _: () = msg_send![&*webview, setValue: &*no, forKey: &*key];
        }

        // Start hidden; the app calls `show()` when needed.
        webview.setHidden(true);

        // Add as a subview of the Metal terminal view.
        // SAFETY: addSubview is safe when both views are in the same window.
        // WKWebView is a subclass of NSView, so the cast is valid.
        let wv_as_view: &NSView = unsafe { &*(Retained::as_ptr(&webview) as *const NSView) };
        container.addSubview(wv_as_view);

        // Load the HTML.
        let html_str = NSString::from_str(html);
        // SAFETY: loadHTMLString_baseURL is safe; nil baseURL is permitted.
        unsafe {
            let _: Retained<AnyObject> = msg_send![&*webview, loadHTMLString: &*html_str, baseURL: std::ptr::null::<NSObject>()];
        }

        Webview {
            obj: webview,
            window,
            terminal_view,
        }
    }

    /// Make the webview visible and give it keyboard focus.
    pub fn show(&self) {
        self.obj.setHidden(false);
        // SAFETY: makeFirstResponder is safe on the MT.
        unsafe {
            let wv_as_responder: &NSResponder =
                &*(Retained::as_ptr(&self.obj) as *const NSResponder);
            self.window.makeFirstResponder(Some(wv_as_responder));
        }
    }

    /// Hide the webview and return keyboard focus to the terminal view.
    pub fn hide(&self) {
        self.obj.setHidden(true);
        // SAFETY: makeFirstResponder is safe on the MT.
        unsafe {
            let tv_as_responder: &NSResponder =
                &*(self.terminal_view.as_ref() as *const NSView as *const NSResponder);
            self.window.makeFirstResponder(Some(tv_as_responder));
        }
    }

    /// Resize the webview frame to fill the window content area (points).
    pub fn set_frame(&self, width: f64, height: f64) {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
        self.obj.setFrame(frame);
    }

    /// Evaluate JavaScript in the WKWebView.
    pub fn eval_js(&self, js: &str) {
        let js_str = NSString::from_str(js);
        // SAFETY: evaluateJavaScript:completionHandler: is safe; nil handler is valid.
        unsafe {
            let _: () = msg_send![
                &*self.obj,
                evaluateJavaScript: &*js_str,
                completionHandler: std::ptr::null::<NSObject>()
            ];
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // WKWebView requires a running NSApplication and a real window — not
    // headless-testable.  The script-message decoding path is tested via the
    // AppHandler trait in appkit.rs tests.
    //
    // This test confirms the module compiles cleanly.
    #[test]
    fn webview_module_compiles() {
        // No-op: successful compilation is the test.
    }
}
