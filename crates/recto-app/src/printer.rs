//! Printing on Windows: the system print dialog picks the printer, the copies and the pages;
//! page images (rendered by the render thread) are drawn on the printer with GDI, from a thread
//! of their own so a slow driver never blocks the window.

use std::sync::mpsc::{Receiver, Sender, channel};

use recto_render::Bitmap;
use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HALFTONE, HDC, HORZRES, LOGPIXELSX, LOGPIXELSY,
    SRCCOPY, SetStretchBltMode, StretchDIBits, VERTRES,
};
use windows::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, PD_NOSELECTION, PD_PAGENUMS, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW, PrintDlgW,
};
use windows::core::PCWSTR;

/// The printer chosen in the dialog.
pub struct Setup {
    /// The printer's device context (an `HDC`, kept as a number to cross threads).
    dc: usize,
    /// Pages to print, 0-based, in order.
    pub pages: std::ops::Range<usize>,
    /// Resolution, in dots per inch.
    pub dpi: f32,
}

/// What the viewer sends to the printing thread.
pub enum Sheet {
    /// A page image rendered at `scale` (pixels per point).
    Page(Bitmap, f32),
    /// Every page was sent.
    Done,
}

/// Shows the print dialog for a document of `count` pages; `None` when it is cancelled.
pub fn choose(count: usize) -> Result<Option<Setup>, String> {
    let last = count.clamp(1, u16::MAX as usize) as u16;
    let mut dialog = PRINTDLGW {
        lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
        Flags: PD_RETURNDC | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE,
        nFromPage: 1,
        nToPage: last,
        nMinPage: 1,
        nMaxPage: last,
        nCopies: 1,
        ..Default::default()
    };
    // SAFETY: `dialog` is initialised with its size; Windows fills the rest.
    let chosen = unsafe { PrintDlgW(&mut dialog) }.as_bool();
    // SAFETY: the handles were allocated by the dialog (or are null) and are not used after.
    unsafe {
        free(dialog.hDevMode);
        free(dialog.hDevNames);
    }
    if !chosen {
        // SAFETY: no arguments.
        let error = unsafe { CommDlgExtendedError() };
        return if error.0 == 0 { Ok(None) } else { Err(format!("print dialog error {:#x}", error.0)) };
    }
    if dialog.hDC.is_invalid() {
        return Err("the printer has no device context".into());
    }
    let pages = if dialog.Flags.contains(PD_PAGENUMS) {
        let from = usize::from(dialog.nFromPage.max(1)) - 1;
        let to = usize::from(dialog.nToPage).clamp(from + 1, count);
        from..to
    } else {
        0..count
    };
    // SAFETY: a valid device context, returned by the dialog.
    let dpi = unsafe { GetDeviceCaps(Some(dialog.hDC), LOGPIXELSX) } as f32;
    Ok(Some(Setup { dc: dialog.hDC.0 as usize, pages, dpi: dpi.max(72.0) }))
}

/// # Safety
/// `handle` is null or a global memory handle nobody else uses.
unsafe fn free(handle: HGLOBAL) {
    if !handle.is_invalid() {
        // SAFETY: as documented.
        let _ = unsafe { GlobalFree(Some(handle)) };
    }
}

/// Starts printing on `setup`'s printer, as a document named `title`. Pages are printed as they
/// arrive; the job ends with `Sheet::Done`, and is cancelled if the sender is dropped before.
pub fn start(setup: Setup, title: String) -> Sender<Sheet> {
    let (sender, receiver) = channel();
    std::thread::Builder::new().name("recto-print".into()).spawn(move || print(setup.dc, &title, receiver)).ok();
    sender
}

fn print(dc: usize, title: &str, sheets: Receiver<Sheet>) {
    let dc = HDC(dc as *mut core::ffi::c_void);
    let name: Vec<u16> = title.encode_utf16().chain([0]).collect();
    let info = DOCINFOW { cbSize: std::mem::size_of::<DOCINFOW>() as i32, lpszDocName: PCWSTR(name.as_ptr()), ..Default::default() };
    // SAFETY: `dc` came from the print dialog and is used by this thread only; `info` and `name`
    // outlive the calls.
    unsafe {
        if StartDocW(dc, &info) <= 0 {
            let _ = DeleteDC(dc);
            return;
        }
        let done = loop {
            match sheets.recv() {
                Ok(Sheet::Page(bitmap, scale)) => page(dc, &bitmap, scale),
                Ok(Sheet::Done) => break true,
                Err(_) => break false,
            }
        };
        if done {
            EndDoc(dc);
        } else {
            AbortDoc(dc);
        }
        let _ = DeleteDC(dc);
    }
}

/// Prints one page: at its real size when it fits the printable area, else shrunk to fit,
/// centred.
///
/// # Safety
/// `dc` is a printer device context inside a started document.
unsafe fn page(dc: HDC, bitmap: &Bitmap, scale: f32) {
    // SAFETY: as documented; the bitmap's pixels match the header below.
    unsafe {
        let (area_w, area_h) = (GetDeviceCaps(Some(dc), HORZRES) as f32, GetDeviceCaps(Some(dc), VERTRES) as f32);
        let (dpi_x, dpi_y) = (GetDeviceCaps(Some(dc), LOGPIXELSX) as f32, GetDeviceCaps(Some(dc), LOGPIXELSY) as f32);
        // The page's size in points, then in printer dots.
        let (points_w, points_h) = (bitmap.width as f32 / scale, bitmap.height as f32 / scale);
        let (mut w, mut h) = (points_w / 72.0 * dpi_x, points_h / 72.0 * dpi_y);
        let shrink = (area_w / w).min(area_h / h).min(1.0);
        (w, h) = (w * shrink, h * shrink);
        let (x, y) = ((area_w - w) / 2.0, (area_h - h) / 2.0);
        let header = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bitmap.width as i32,
            // Negative: rows top to bottom.
            biHeight: -(bitmap.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
        if StartPage(dc) <= 0 {
            return;
        }
        SetStretchBltMode(dc, HALFTONE);
        StretchDIBits(
            dc,
            x as i32,
            y as i32,
            w as i32,
            h as i32,
            0,
            0,
            bitmap.width as i32,
            bitmap.height as i32,
            Some(bitmap.pixels.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        EndPage(dc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::CreateDCW;
    use windows::core::w;

    /// Needs Windows' "Microsoft Print to PDF" printer: `cargo test -p recto printer -- --ignored`.
    #[test]
    #[ignore]
    fn prints_a_page_to_a_file() {
        let out = std::env::temp_dir().join(format!("recto-print-test-{}.pdf", std::process::id()));
        let path: Vec<u16> = out.to_string_lossy().encode_utf16().chain([0]).collect();
        // SAFETY: constant strings; the context is used on this thread only and deleted at the end.
        unsafe {
            let dc = CreateDCW(w!("WINSPOOL"), w!("Microsoft Print to PDF"), PCWSTR::null(), None);
            assert!(!dc.is_invalid(), "no Microsoft Print to PDF printer");
            let info = DOCINFOW {
                cbSize: std::mem::size_of::<DOCINFOW>() as i32,
                lpszDocName: w!("recto test"),
                lpszOutput: PCWSTR(path.as_ptr()),
                ..Default::default()
            };
            assert!(StartDocW(dc, &info) > 0);
            // An A4 page at 1 pixel per point, grey.
            let bitmap = Bitmap { width: 595, height: 842, pixels: vec![128; 595 * 842 * 4] };
            page(dc, &bitmap, 1.0);
            assert!(EndDoc(dc) > 0);
            let _ = DeleteDC(dc);
        }
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "not a PDF");
        let _ = std::fs::remove_file(&out);
    }
}
