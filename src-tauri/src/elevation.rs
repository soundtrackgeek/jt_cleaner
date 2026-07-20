use std::ffi::OsString;

pub(crate) const ELEVATED_SCAN_ARGUMENT: &str = "--elevated-scan-root";

pub(crate) fn startup_scan_root() -> Option<String> {
    startup_scan_root_from(std::env::args_os())
}

fn startup_scan_root_from(arguments: impl IntoIterator<Item = OsString>) -> Option<String> {
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == ELEVATED_SCAN_ARGUMENT {
            return arguments
                .next()
                .and_then(|path| path.into_string().ok())
                .filter(|path| !path.trim().is_empty());
        }
    }
    None
}

#[cfg(windows)]
pub(crate) fn is_process_elevated() -> Result<bool, String> {
    use std::{mem::size_of, ptr::null_mut};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let mut token: HANDLE = null_mut();
    // SAFETY: Windows owns the pseudo process handle. `token` is a valid output
    // pointer and is closed below on every path after OpenProcessToken succeeds.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(format!(
            "Windows could not inspect Luna's administrator status: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned_size = 0_u32;
    // SAFETY: `elevation` points to writable storage of the exact size passed to
    // GetTokenInformation, and `token` is valid until CloseHandle below.
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_size,
        )
    };
    let information_error = (result == 0).then(std::io::Error::last_os_error);
    // SAFETY: `token` was returned by OpenProcessToken and is closed once here.
    unsafe { CloseHandle(token) };

    if let Some(error) = information_error {
        return Err(format!(
            "Windows could not read Luna's administrator status: {error}"
        ));
    }
    Ok(elevation.TokenIsElevated != 0)
}

#[cfg(not(windows))]
pub(crate) fn is_process_elevated() -> Result<bool, String> {
    Ok(false)
}

#[cfg(windows)]
pub(crate) fn relaunch_for_scan(path: &str) -> Result<(), String> {
    use std::{
        os::windows::ffi::OsStrExt,
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

    let executable = std::env::current_exe()
        .map_err(|error| format!("Luna could not locate its executable: {error}"))?;
    let parameters = format!("{ELEVATED_SCAN_ARGUMENT} {}", quote_windows_argument(path));
    let operation = wide_null("runas");
    let executable: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let parameters = wide_null(&parameters);

    // SAFETY: every string pointer is null-terminated and remains alive for the
    // duration of ShellExecuteW. No window or working-directory handle is used.
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            executable.as_ptr(),
            parameters.as_ptr(),
            null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        return Err(if result == 5 {
            "Administrator approval was cancelled. Luna kept the current window open.".to_string()
        } else {
            format!(
                "Windows could not restart Luna with administrator access (ShellExecute code {result})."
            )
        });
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn relaunch_for_scan(_path: &str) -> Result<(), String> {
    Err("Administrator relaunch is available only on Windows.".to_string())
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

fn quote_windows_argument(argument: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0_usize;
    for character in argument.chars() {
        if character == '\\' {
            backslashes += 1;
        } else if character == '"' {
            quoted.push_str(&"\\".repeat(backslashes.saturating_mul(2).saturating_add(1)));
            quoted.push('"');
            backslashes = 0;
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            quoted.push(character);
        }
    }
    quoted.push_str(&"\\".repeat(backslashes.saturating_mul(2)));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_scan_root_consumes_the_value_after_its_flag() {
        let arguments = [
            OsString::from("luna-clean.exe"),
            OsString::from("--hidden"),
            OsString::from(ELEVATED_SCAN_ARGUMENT),
            OsString::from(r"C:\Users\Luna Clean"),
        ];
        assert_eq!(
            startup_scan_root_from(arguments),
            Some(r"C:\Users\Luna Clean".to_string())
        );
    }

    #[test]
    fn windows_argument_quoting_preserves_spaces_quotes_and_trailing_slashes() {
        assert_eq!(quote_windows_argument(r"C:\"), r#""C:\\""#);
        assert_eq!(
            quote_windows_argument(r#"C:\A folder\say "hello""#),
            r#""C:\A folder\say \"hello\"""#
        );
    }

    #[cfg(windows)]
    #[test]
    fn current_windows_token_reports_an_elevation_state() {
        assert!(is_process_elevated().is_ok());
    }
}
