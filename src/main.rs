mod lib_input;
mod utils;
mod wii_remote;

use std::{
    ffi::CStr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::SystemTime,
};

use chrono::Local;
use clap::{
    builder::BoolishValueParser, crate_authors, crate_description, crate_name, crate_version, Arg,
    Command,
};
use env_logger::fmt::Formatter;
use env_logger::Builder;
use input_sys::{
    libinput_device_get_udev_device, libinput_dispatch, libinput_event_get_device,
    libinput_get_event,
};
use input_sys::{libinput_udev_assign_seat, libinput_udev_create_context};
use lib_input::INTERFACE;
use libudev_sys::udev_device_get_syspath;
use log::error;
use log::info;
use log::warn;
use log::LevelFilter;
use log::Record;
use std::io::Error;
use std::io::Write;

use log::debug;

use wii_remote::WiiRemote;

static CURRENT_TIME: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(true);

fn main() {
    let matches = Command::new(crate_name!())
        .about(crate_description!())
        .author(crate_authors!(", "))
        .arg_required_else_help(false)
        .args([
            Arg::new("bluetoothctl-path")
                .short('b')
                .long("bluetoothctl-path")
                .help("The filepath to the `bluetoothctl' executable.")
                .required(false),
            Arg::new("xwiishow-path")
                .short('w')
                .long("xwiishow-path")
                .help("The filepath to the `xwiishow' executable.")
                .required(false),
            Arg::new("debug")
                .short('d')
                .long("debug")
                .help("Enables debug mode")
                .default_value(match cfg!(debug_assertions) {
                    true => "true",
                    false => "false",
                })
                .required(false)
                .value_parser(BoolishValueParser::new()),
        ])
        .version(crate_version!())
        .get_matches();

    // Initialize the logger
    Builder::new()
        .format(process_log_buffer)
        .filter(None, LevelFilter::Info)
        .filter_level(match matches.get_one::<bool>("debug") {
            Some(debug) if *debug => LevelFilter::Debug,
            _ => LevelFilter::Info,
        })
        .init();

    info!("Starting Wii Remote manager...");

    let wii_remote = Arc::new(Mutex::new(WiiRemote::new()));
    let wii_remote_connect = Arc::clone(&wii_remote);
    let wii_remote_timeout = Arc::clone(&wii_remote);

    let _connect_and_poll_handle = thread::spawn(move || {
        connect_and_poll(&wii_remote_connect);
    });

    let _timeout_handle = thread::spawn(move || {
        timeout(&wii_remote_timeout);
    });

    while RUNNING.load(Ordering::Relaxed) {
        thread::park();
    }

    info!("Shutting down...");
}

fn connect_and_poll(wii_remote: &Arc<Mutex<WiiRemote>>) {
    info!("Initializing libinput...");

    let libinput;
    unsafe {
        let udev = libudev_sys::udev_new();
        libinput = libinput_udev_create_context(&INTERFACE, std::ptr::null_mut(), udev as *mut _);
        libinput_udev_assign_seat(libinput, c"seat0".as_ptr());
    }

    const MAX_RETRIES: u32 = 10;
    let mut retries = 0;

    loop {
        if retries >= MAX_RETRIES {
            error!(
                "Failed to connect to Wii Remote after {} attempts",
                MAX_RETRIES
            );
            break;
        }

        let mut wii_remote = match wii_remote.try_lock() {
            Ok(lock) => lock,
            Err(_) => {
                debug!("Mutex is locked, retrying...");
                thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
        };

        if !wii_remote.try_connect() {
            retries += 1;
            warn!(
                "Failed to connect to Wii Remote, retrying... (attempt {}/{})",
                retries, MAX_RETRIES
            );
            thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }

        retries = 0;
        info!("Wii Remote connected successfully.");

        let wii_remote_udev_device_path = match wii_remote.get_udev_device_path() {
            Some(path) => path,
            None => {
                warn!("Failed to get udev device path");
                continue;
            }
        };

        unsafe {
            loop {
                let ret = libinput_dispatch(libinput);
                if ret != 0 {
                    error!("Failed to dispatch libinput events: {}", ret);
                    break;
                }

                loop {
                    let event = libinput_get_event(libinput);
                    if event == std::ptr::null_mut() {
                        break;
                    }

                    let device = libinput_event_get_device(event);
                    let udev_device = libinput_device_get_udev_device(device);
                    let udev_device_path = udev_device_get_syspath(udev_device as *mut _);
                    let udev_device_path_cstr = CStr::from_ptr(udev_device_path);
                    if udev_device_path_cstr.to_str().unwrap()
                        != wii_remote_udev_device_path.as_str()
                    {
                        debug!(
                            "Ignoring event from unrelated device: {}",
                            udev_device_path_cstr.to_str().unwrap()
                        );

                        continue;
                    }

                    let current_time =
                        match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                            Ok(duration) => duration.as_secs(),
                            Err(_) => {
                                error!("System time error: clock went backwards");
                                continue;
                            }
                        };

                    CURRENT_TIME.store(current_time, Ordering::Relaxed);
                    debug!("Updated current time: {}", current_time);
                }
            }
        }
    }
}

fn timeout(wii_remote: &Arc<Mutex<WiiRemote>>) {
    loop {
        thread::sleep(std::time::Duration::from_secs(1));

        let mut wii_remote = match wii_remote.try_lock() {
            Ok(lock) => lock,
            Err(_) => {
                debug!("Mutex is locked, skipping timeout check...");
                continue;
            }
        };

        let current_time = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(_) => {
                error!("System time error: clock went backwards");
                continue;
            }
        };

        let elapsed_time = current_time - CURRENT_TIME.load(Ordering::Relaxed);

        if elapsed_time >= (5 * 60) {
            info!("Wii Remote has been idle for 5 minutes, disconnecting...");
            wii_remote.disconnect();
        }
    }
}

fn process_log_buffer(buf: &mut Formatter, record: &Record<'_>) -> Result<(), Error> {
    writeln!(
        buf,
        "[{}] [{}]: {}",
        Local::now().format("%+"),
        record.level(),
        record.args()
    )
}
