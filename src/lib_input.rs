use std::{
    ffi::{c_int, CStr, OsStr},
    fs::{File, OpenOptions},
    os::{
        fd::{FromRawFd, IntoRawFd},
        raw::c_void,
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    },
    path::Path,
};

use input_sys::libinput_interface;

pub static INTERFACE: libinput_interface = libinput_interface {
    open_restricted: Some(open_restricted_func),
    close_restricted: Some(close_restricted_func),
};

extern "C" fn open_restricted_func(path: *const i8, flags: i32, _user_data: *mut c_void) -> c_int {
    let path = unsafe { CStr::from_ptr(path) };
    let path = Path::new(OsStr::from_bytes(path.to_bytes()));

    // Attempt to open the file with the provided flags
    match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(flags)
        .open(path)
    {
        Ok(file) => file.into_raw_fd(), // Return the file descriptor on success
        Err(err) => -err.raw_os_error().unwrap_or(-1), // Return a negative errno on failure
    }
}

extern "C" fn close_restricted_func(fd: i32, _user_data: *mut c_void) {
    // Convert the raw file descriptor to a `File` and drop it to close it
    if fd >= 0 {
        drop(unsafe { File::from_raw_fd(fd) });
    }
}
