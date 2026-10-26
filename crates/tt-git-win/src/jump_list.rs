//! The taskbar's jump list (`ICustomDestinationList`): each entry an
//! `IShellLinkW` that starts this executable with its arguments, titled
//! through its `IPropertyStore`.
//!
//! Call it on a thread that has COM set up; GPUI's main thread is one
//! (`OleInitialize`, a single-threaded apartment). No `SetAppID`: the list
//! goes with the process's AppUserModelID.

use windows::Win32::Foundation::{E_FAIL, PROPERTYKEY};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, SHStrDupW, ShellLink,
};
use windows::core::{GUID, HSTRING, Interface};

pub use windows::core::{Error, Result};

/// `PKEY_Title` (in `Win32_Storage_EnhancedStorage`, which nothing else
/// needs): the text an entry shows.
const PKEY_TITLE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9),
    pid: 2,
};

/// The longest command line read back from a removed entry.
const MAX_ARGUMENTS: usize = 4096;

/// An entry: this executable started with `arguments`.
#[derive(Debug, Clone, Copy)]
pub struct Link<'a> {
    pub arguments: &'a str,
    pub title: &'a str,
    /// Shown when the pointer rests on the entry.
    pub tooltip: &'a str,
}

/// A list being built: begun, then committed (or abandoned when dropped).
pub struct Session {
    list: ICustomDestinationList,
    slots: usize,
    removed: Vec<String>,
    exe: HSTRING,
    done: bool,
}

impl Session {
    /// Starts a new list. Windows says how many entries fit and which ones
    /// the user removed since the last list.
    pub fn begin() -> Result<Self> {
        let exe = std::env::current_exe().map_err(|e| Error::new(E_FAIL, e.to_string()))?;
        // SAFETY: a plain COM object creation with a valid CLSID; the
        // caller's thread has COM set up.
        let list: ICustomDestinationList = unsafe { CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER) }?;
        let mut slots = 0;
        // SAFETY: `slots` is a valid u32 for the call to write.
        let removed: IObjectArray = unsafe { list.BeginList(&mut slots) }?;
        Ok(Self {
            list,
            slots: slots as usize,
            removed: removed_arguments(&removed),
            exe: HSTRING::from(exe.as_os_str()),
            done: false,
        })
    }

    /// How many entries fit (the user's setting for recent items).
    pub fn slots(&self) -> usize {
        self.slots
    }

    /// The command lines of the entries the user removed.
    pub fn removed(&self) -> &[String] {
        &self.removed
    }

    /// Puts `entries` under the heading `category` and `tasks` in Tasks,
    /// and replaces the list with them. Windows refuses `category` when
    /// the user turned off "Show recently opened items" in Start's
    /// settings: it's left out, the tasks still go in, and that error comes
    /// back in `Ok(Some(..))`.
    pub fn commit(mut self, category: &str, entries: &[Link], tasks: &[Link]) -> Result<Option<Error>> {
        let mut refused = None;
        if !entries.is_empty() {
            let recent = self.collection(entries)?;
            // SAFETY: `recent` is a live object array of shell links.
            refused = unsafe { self.list.AppendCategory(&HSTRING::from(category), &recent) }.err();
        }
        let tasks = self.collection(tasks)?;
        // SAFETY: `tasks` is a live object array of shell links.
        unsafe { self.list.AddUserTasks(&tasks) }?;
        self.done = true;
        // SAFETY: the list was begun in `begin` and not committed yet.
        unsafe { self.list.CommitList() }?;
        Ok(refused)
    }

    /// `links` as an object array.
    fn collection(&self, links: &[Link]) -> Result<IObjectArray> {
        // SAFETY: a plain COM object creation with a valid CLSID.
        let collection: IObjectCollection =
            unsafe { CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER) }?;
        for link in links {
            // SAFETY: adds a live COM object, which the collection AddRefs.
            unsafe { collection.AddObject(&self.link(link)?) }?;
        }
        collection.cast()
    }

    /// An entry that runs this executable with `link.arguments`.
    fn link(&self, link: &Link) -> Result<IShellLinkW> {
        // SAFETY: a plain COM object creation with a valid CLSID.
        let shell_link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
        // SAFETY: each string is a NUL-terminated HSTRING that outlives
        // its call; the link copies it.
        unsafe {
            shell_link.SetPath(&self.exe)?;
            shell_link.SetArguments(&HSTRING::from(link.arguments))?;
            shell_link.SetDescription(&HSTRING::from(link.tooltip))?;
            shell_link.SetIconLocation(&self.exe, 0)?;
        }
        let store: IPropertyStore = shell_link.cast()?;
        let mut value = string_value(link.title)?;
        // SAFETY: `value` is a valid VT_LPWSTR the store copies.
        let set = unsafe { store.SetValue(&PKEY_TITLE, &value) };
        // SAFETY: `value` owns its string (from SHStrDupW), freed here once.
        unsafe { PropVariantClear(&mut value) }?;
        set?;
        // SAFETY: a plain call on a live property store.
        unsafe { store.Commit() }?;
        Ok(shell_link)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.done {
            // SAFETY: the list was begun and not committed; aborting it
            // leaves the old one in place.
            let _ = unsafe { self.list.AbortList() };
        }
    }
}

/// A `VT_LPWSTR` holding `text` (free it with `PropVariantClear`).
fn string_value(text: &str) -> Result<PROPVARIANT> {
    // SAFETY: copies a NUL-terminated string into memory from
    // CoTaskMemAlloc, which PropVariantClear frees.
    let text = unsafe { SHStrDupW(&HSTRING::from(text)) }?;
    let mut value = PROPVARIANT::default();
    // SAFETY: writes the type and the matching union field of a zeroed
    // PROPVARIANT, which is how InitPropVariantFromString fills one.
    unsafe {
        let inner = &mut *value.Anonymous.Anonymous;
        inner.vt = VT_LPWSTR;
        inner.Anonymous.pwszVal = text;
    }
    Ok(value)
}

/// The command lines of the removed entries (all ours, so shell links).
fn removed_arguments(removed: &IObjectArray) -> Vec<String> {
    // SAFETY: a plain call on a live object array.
    let count = unsafe { removed.GetCount() }.unwrap_or(0);
    (0..count)
        .filter_map(|i| {
            // SAFETY: `i` is below the array's count.
            let link: IShellLinkW = unsafe { removed.GetAt(i) }.ok()?;
            let mut buf = vec![0u16; MAX_ARGUMENTS];
            // SAFETY: the call writes at most `buf.len()` characters,
            // NUL-terminated, into `buf`.
            unsafe { link.GetArguments(&mut buf) }.ok()?;
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..len]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    use super::*;

    /// Builds a real list for the test executable and takes it away again.
    /// Ignored: it changes the shell's state for this user.
    #[test]
    #[ignore]
    fn a_list_is_built_and_committed() {
        // SAFETY: sets this test thread up for COM, as GPUI's main thread is.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.unwrap();
        let session = Session::begin().unwrap();
        assert!(session.slots() > 0);
        let entry = Link {
            arguments: r#""C:\src\a""#,
            title: "a",
            tooltip: r"C:\src\a",
        };
        let task = Link {
            arguments: "--open-repository",
            title: "Open repository…",
            tooltip: "Open repository…",
        };
        session.commit("Recent repositories", &[entry], &[task]).unwrap();
        // SAFETY: a plain COM object creation with a valid CLSID.
        let list: ICustomDestinationList =
            unsafe { CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER) }.unwrap();
        // SAFETY: deletes the list of this process's app ID.
        unsafe { list.DeleteList(None) }.unwrap();
    }
}
