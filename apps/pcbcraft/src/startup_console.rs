//! Console attachment belongs to the launcher and runs before any workers start.

/// Attach only when Windows supplied no stderr, then keep its console handle alive.
/// A valid stderr, including a redirected file, pipe or NUL, is left unchanged.
/// Explicitly set stderr because STARTF_USESTDHANDLES can preserve an empty handle
/// after attachment. CONOUT$ is opened only after trying the existing parent;
/// no console is allocated, so ordinary GUI launches remain silent.
/// The returned guard must live until all startup error reporting has finished.
pub(super) fn attach() -> Option<std::fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use win32console::{
            console::{HandleType, WinConsole},
            structs::handle::Handle,
        };

        let stderr = std::io::stderr().as_raw_handle();
        if !stderr.is_null() && stderr as usize != usize::MAX {
            return None;
        }
        let _ = WinConsole::attach_console(u32::MAX);
        // Rust's Unicode console detection needs a readable as well as writable handle.
        let output = std::fs::OpenOptions::new().read(true).write(true).open("CONOUT$").ok()?;
        // Handle::new borrows the handle; the File owns it through the event loop.
        WinConsole::set_std_handle(HandleType::Error, Handle::new(output.as_raw_handle().cast())).ok()?;
        Some(output)
    }
    #[cfg(not(windows))]
    None
}
