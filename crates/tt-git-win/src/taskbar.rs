//! A window's taskbar button (`ITaskbarList3`): progress and the overlay badge.
//!
//! Call these on the thread COM was set up on as a single-threaded
//! apartment: GPUI's main thread, where its `OleInitialize` did so. The
//! taskbar object is made there once and kept.

use std::cell::OnceCell;
use std::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{ITaskbarList3, TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, GetForegroundWindow, HICON, ICONINFO};
use windows::core::{HSTRING, Result};

/// What a taskbar button's progress bar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// No bar.
    None,
    /// A bar that moves without a value.
    Indeterminate,
    /// A full red bar.
    Error,
}

/// A square icon, as 32-bit BGRA pixels with premultiplied alpha, row by
/// row from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub size: u32,
    pub bgra: Vec<u8>,
}

thread_local! {
    static TASKBAR: OnceCell<Result<ITaskbarList3>> = const { OnceCell::new() };
}

/// The taskbar object, made on first use. If that failed, every call fails
/// the same way.
fn with_taskbar<T>(f: impl FnOnce(&ITaskbarList3) -> Result<T>) -> Result<T> {
    TASKBAR.with(|cell| match cell.get_or_init(make) {
        Ok(taskbar) => f(taskbar),
        Err(e) => Err(e.clone()),
    })
}

fn make() -> Result<ITaskbarList3> {
    // SAFETY: CoCreateInstance takes a valid CLSID and no outer object; it
    // fails cleanly if COM isn't set up on this thread.
    let taskbar: ITaskbarList3 = unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) }?;
    // SAFETY: `taskbar` is a live ITaskbarList3, HrInit takes no arguments.
    unsafe { taskbar.HrInit() }?;
    Ok(taskbar)
}

/// Shows `progress` on the taskbar button of the window `hwnd`.
pub fn set_progress(hwnd: isize, progress: Progress) -> Result<()> {
    let hwnd = HWND(hwnd as *mut c_void);
    with_taskbar(|taskbar| {
        // SAFETY: the calls take a window handle by value; a handle that's
        // no longer a window makes them fail, nothing more.
        unsafe {
            match progress {
                Progress::None => taskbar.SetProgressState(hwnd, TBPF_NOPROGRESS),
                Progress::Indeterminate => taskbar.SetProgressState(hwnd, TBPF_INDETERMINATE),
                // The error state colors the bar, so it needs a value.
                Progress::Error => taskbar
                    .SetProgressValue(hwnd, 1, 1)
                    .and_then(|()| taskbar.SetProgressState(hwnd, TBPF_ERROR)),
            }
        }
    })
}

/// Puts `icon` over the taskbar button of the window `hwnd`, with
/// `description` for screen readers, or takes the badge off with `None`.
pub fn set_overlay(hwnd: isize, badge: Option<(&Icon, &str)>) -> Result<()> {
    let hwnd = HWND(hwnd as *mut c_void);
    let Some((icon, description)) = badge else {
        // SAFETY: a null icon takes the overlay off.
        return with_taskbar(|taskbar| unsafe { taskbar.SetOverlayIcon(hwnd, HICON::default(), &HSTRING::new()) });
    };
    let hicon = make_icon(icon)?;
    // SAFETY: `hicon` is the icon just made; the taskbar keeps a copy of
    // it, so it's destroyed right after.
    let shown = with_taskbar(|taskbar| unsafe { taskbar.SetOverlayIcon(hwnd, hicon, &HSTRING::from(description)) });
    // SAFETY: `hicon` came from CreateIconIndirect and isn't used again.
    let _ = unsafe { DestroyIcon(hicon) };
    shown
}

/// Whether the window `hwnd` is the foreground window: the one the user is
/// in, as Windows has it now, whatever the app last heard.
pub fn is_foreground(hwnd: isize) -> bool {
    // SAFETY: GetForegroundWindow takes nothing and returns a handle, null
    // when no window is in the foreground.
    let foreground = unsafe { GetForegroundWindow() };
    foreground.0 as isize == hwnd
}

/// An `HICON` of `icon`'s pixels. The caller destroys it.
fn make_icon(icon: &Icon) -> Result<HICON> {
    let size = icon.size as i32;
    assert_eq!(
        icon.bgra.len(),
        (icon.size * icon.size * 4) as usize,
        "BGRA pixels for a square icon"
    );
    // The mask is ignored where the color has alpha, but must be there: one
    // bit per pixel, each row padded to 16 bits.
    let mask = vec![0u8; (icon.size.div_ceil(16) * 2 * icon.size) as usize];
    // SAFETY: each buffer holds the bits CreateBitmap reads for its size
    // and depth, and it copies them.
    let color = unsafe { CreateBitmap(size, size, 1, 32, Some(icon.bgra.as_ptr().cast())) };
    // SAFETY: as above, at one bit per pixel.
    let mask = unsafe { CreateBitmap(size, size, 1, 1, Some(mask.as_ptr().cast())) };
    let info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    // SAFETY: `info` is a valid ICONINFO; CreateIconIndirect copies the
    // bitmaps, so they're deleted after.
    let hicon = unsafe { CreateIconIndirect(&info) };
    // SAFETY: both bitmaps came from CreateBitmap and aren't used again.
    unsafe {
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
    }
    hicon
}
