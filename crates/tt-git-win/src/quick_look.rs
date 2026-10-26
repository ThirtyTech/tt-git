//! The Quick Look panel (`QLPreviewPanel`), previewing one file.
//!
//! Call these on the main thread, where AppKit runs. The panel is the
//! app's shared one; its data source is made once and kept, since the
//! panel doesn't retain it.

use std::cell::{OnceCell, RefCell};
use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSInteger, NSString, NSURL};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource};

/// What the data source hands the panel: the file shown.
#[derive(Default)]
struct Ivars {
    file: RefCell<Option<Retained<NSURL>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Source`
    // doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TTGitQuickLookSource"]
    #[ivars = Ivars]
    struct Source;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for Source {}

    // SAFETY: the methods match the protocol's signatures.
    unsafe impl QLPreviewPanelDataSource for Source {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_items(&self, _panel: Option<&QLPreviewPanel>) -> NSInteger {
            NSInteger::from(self.ivars().file.borrow().is_some())
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item(
            &self,
            _panel: Option<&QLPreviewPanel>,
            _index: NSInteger,
        ) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
            let file = self.ivars().file.borrow().clone();
            file.map(ProtocolObject::from_retained)
        }
    }
);

impl Source {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars::default());
        // SAFETY: NSObject's `init` takes no arguments and returns the
        // initialized object.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// The panel's data source, kept for the life of the app: the panel
    /// holds it unretained.
    static SOURCE: OnceCell<Retained<Source>> = const { OnceCell::new() };
}

/// Shows `file` in the Quick Look panel, bringing the panel up if it isn't.
/// Off the main thread it does nothing and returns `false`.
pub fn preview(file: &Path) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    SOURCE.with(|cell| {
        let source = cell.get_or_init(|| Source::new(mtm));
        source.ivars().file.replace(Some(url));
        // SAFETY: on the main thread, as the marker shows.
        let Some(panel) = (unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }) else {
            return false;
        };
        // SAFETY: `SOURCE` keeps the data source alive as long as the
        // thread, which is the app's main thread, so as long as the panel.
        unsafe {
            panel.setDataSource(Some(ProtocolObject::from_ref(&**source)));
            panel.reloadData();
            panel.setCurrentPreviewItemIndex(0);
        }
        panel.makeKeyAndOrderFront(None);
        true
    })
}
