//! Toast notifications (`Windows.UI.Notifications`), sent as an app id.
//!
//! The caller builds the toast's XML; [`Toaster`] shows it under a tag, so
//! a newer toast with the same tag replaces the last, and reports a click
//! on it (or on one of its buttons) with the button's `arguments`.
//!
//! The WinRT calls here are safe ones; the module is here with the other
//! Windows calls. Use a [`Toaster`] on the thread that made it.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager, ToastNotifier};
use windows::core::{HSTRING, IInspectable, Interface as _, Result};

/// Shows toasts as one app id, and remembers the last one under each tag.
pub struct Toaster {
    notifier: ToastNotifier,
    shown: HashMap<String, ToastNotification>,
}

impl Toaster {
    /// A toaster for the AppUserModelID `app_id`, which needs a
    /// `DisplayName` under `HKCU\Software\Classes\AppUserModelId\<app_id>`
    /// (or a Start menu shortcut carrying it).
    pub fn new(app_id: &str) -> Result<Self> {
        Ok(Self {
            notifier: ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?,
            shown: HashMap::new(),
        })
    }

    /// Shows the toast `xml` under `tag`, hiding the last one under it.
    /// `activated` gets the `arguments` of the button clicked, or `None` for
    /// a click on the toast itself. It's called on a thread of Windows'.
    pub fn show(&mut self, tag: &str, xml: &str, activated: impl Fn(Option<String>) + Send + 'static) -> Result<()> {
        let document = XmlDocument::new()?;
        document.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&document)?;
        toast.SetTag(&HSTRING::from(short_tag(tag)))?;
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, arguments| {
                let action = arguments
                    .as_ref()
                    .and_then(|a| a.cast::<ToastActivatedEventArgs>().ok())
                    .and_then(|a| a.Arguments().ok())
                    .filter(|a| !a.is_empty())
                    .map(|a| a.to_string());
                activated(action);
                Ok(())
            },
        ))?;
        if let Some(last) = self.shown.remove(tag) {
            // Gone already when the user dismissed it.
            let _ = self.notifier.Hide(&last);
        }
        self.notifier.Show(&toast)?;
        self.shown.insert(tag.to_string(), toast);
        Ok(())
    }
}

/// `tag` hashed down to fit: Windows takes tags of up to 64 characters.
fn short_tag(tag: &str) -> String {
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
