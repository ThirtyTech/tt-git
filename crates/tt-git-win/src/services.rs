//! The app's macOS service, "Open in tt-git" on folders in Finder.
//!
//! The bundle's `Info.plist` declares it (`NSServices`, message
//! `openFolder`), and macOS sends it to the object the app sets as
//! `NSApp.servicesProvider`: an `openFolder:userData:error:` message with
//! the folders on a pasteboard. GPUI has no provider of its own, so
//! [`install`] sets one that hands the folders to a callback. macOS starts
//! the app first if it isn't running, and sends the message once it has
//! finished launching.

use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSPasteboard, NSPasteboardTypeFileURL};
use objc2_foundation::{NSString, NSURL};

type Callback = Box<dyn Fn(Vec<PathBuf>)>;

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - `ServicesProvider` doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TTGitServicesProvider"]
    #[ivars = Callback]
    struct ServicesProvider;

    impl ServicesProvider {
        /// The `NSMessage` `openFolder` in `Info.plist`.
        #[unsafe(method(openFolder:userData:error:))]
        fn open_folder(&self, pasteboard: &NSPasteboard, _user_data: Option<&NSString>, _error: *mut *mut NSString) {
            let folders = file_paths(pasteboard);
            if !folders.is_empty() {
                (self.ivars())(folders);
            }
        }
    }
);

impl ServicesProvider {
    fn new(callback: Callback, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(callback);
        // SAFETY: NSObject's `init` on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

/// The files and folders on `pasteboard`, each item a file URL.
fn file_paths(pasteboard: &NSPasteboard) -> Vec<PathBuf> {
    let Some(items) = pasteboard.pasteboardItems() else {
        return Vec::new();
    };
    items
        .iter()
        // SAFETY: an AppKit constant, valid for the life of the process.
        .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
        .filter_map(|url| NSURL::URLWithString(&url))
        .filter_map(|url| url.to_file_path())
        .collect()
}

/// Makes the app answer its service: `open` gets the folders picked in
/// Finder. Call it once, on the main thread, from GPUI's
/// `Application::run` callback (the app has launched by then). Returns
/// `false` off the main thread.
pub fn install(open: impl Fn(Vec<PathBuf>) + 'static) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let provider = ServicesProvider::new(Box::new(open), mtm);
    let app = NSApplication::sharedApplication(mtm);
    let object: &AnyObject = &provider;
    // SAFETY: the provider answers the message `Info.plist` names, with
    // the signature AppKit sends it.
    unsafe { app.setServicesProvider(Some(object)) };
    // Needed until the app quits, whether or not AppKit keeps a reference.
    std::mem::forget(provider);
    true
}
