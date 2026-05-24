//! Small AppKit utilities that don't fit elsewhere — clipboard writes and
//! "open this in the user's default app" via NSWorkspace. Both are common
//! UI plumbing; isolating them here keeps `crates/anvil/` free of objc2
//! boilerplate.

use objc2::msg_send;
use objc2::runtime::AnyClass;

/// Copy `text` to the system clipboard (NSPasteboard general pasteboard).
///
/// Silently no-ops on any failure (class lookup, message-send) — this is a
/// best-effort UI nicety, never a fatal path.
pub fn set_clipboard(text: &str) {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: standard NSPasteboard.generalPasteboard / clearContents /
        // setString:forType: chain on the main thread.
        unsafe {
            let Some(pb_class) = AnyClass::get(c"NSPasteboard") else {
                return;
            };
            let pb: *mut objc2::runtime::AnyObject = msg_send![pb_class, generalPasteboard];
            if pb.is_null() {
                return;
            }
            let _: i64 = msg_send![pb, clearContents];

            let Some(nsstring_class) = AnyClass::get(c"NSString") else {
                return;
            };
            let cstr = match std::ffi::CString::new(text) {
                Ok(c) => c,
                Err(_) => return,
            };
            let s: *mut objc2::runtime::AnyObject =
                msg_send![nsstring_class, stringWithUTF8String: cstr.as_ptr()];
            if s.is_null() {
                return;
            }
            let type_str: *mut objc2::runtime::AnyObject =
                msg_send![nsstring_class, stringWithUTF8String: c"public.utf8-plain-text".as_ptr()];
            let _: bool = msg_send![pb, setString: s, forType: type_str];
        }
    });
}

/// Read the system clipboard's current text contents.
///
/// Returns `None` when the pasteboard has no string for the public UTF-8
/// type, or any AppKit call fails. Best-effort like `set_clipboard`.
pub fn get_clipboard() -> Option<String> {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: NSPasteboard.generalPasteboard.stringForType: chain on the
        // main thread.
        unsafe {
            let pb_class = AnyClass::get(c"NSPasteboard")?;
            let pb: *mut objc2::runtime::AnyObject = msg_send![pb_class, generalPasteboard];
            if pb.is_null() {
                return None;
            }
            let nsstring_class = AnyClass::get(c"NSString")?;
            let type_str: *mut objc2::runtime::AnyObject = msg_send![
                nsstring_class,
                stringWithUTF8String: c"public.utf8-plain-text".as_ptr()
            ];
            let s: *mut objc2::runtime::AnyObject = msg_send![pb, stringForType: type_str];
            if s.is_null() {
                return None;
            }
            let cstr: *const std::ffi::c_char = msg_send![s, UTF8String];
            if cstr.is_null() {
                return None;
            }
            Some(
                std::ffi::CStr::from_ptr(cstr)
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    })
}

/// Open `path_or_url` in the user's default app (Finder for directories,
/// $EDITOR-equivalent for files, browser for `https://…`). Best-effort.
pub fn open_with_default_app(path_or_url: &str) {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: NSWorkspace.sharedWorkspace + openURL: / openFile: on the
        // main thread.
        unsafe {
            let Some(ws_class) = AnyClass::get(c"NSWorkspace") else {
                return;
            };
            let ws: *mut objc2::runtime::AnyObject = msg_send![ws_class, sharedWorkspace];
            if ws.is_null() {
                return;
            }
            let Some(nsstring_class) = AnyClass::get(c"NSString") else {
                return;
            };
            let cstr = match std::ffi::CString::new(path_or_url) {
                Ok(c) => c,
                Err(_) => return,
            };
            let s: *mut objc2::runtime::AnyObject =
                msg_send![nsstring_class, stringWithUTF8String: cstr.as_ptr()];
            if s.is_null() {
                return;
            }
            // URL? Try NSURL.URLWithString: first; otherwise treat as path.
            let is_url = path_or_url.starts_with("http://")
                || path_or_url.starts_with("https://")
                || path_or_url.starts_with("file://");
            if is_url {
                let Some(url_class) = AnyClass::get(c"NSURL") else {
                    return;
                };
                let url: *mut objc2::runtime::AnyObject = msg_send![url_class, URLWithString: s];
                if !url.is_null() {
                    let _: bool = msg_send![ws, openURL: url];
                }
            } else {
                let _: bool = msg_send![ws, openFile: s];
            }
        }
    });
}
