mod udev_device_extensions;
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
use log::error;
use log::info;
use log::warn;
use log::LevelFilter;
use log::Record;
use std::io::Error;
use std::io::Write;

use colpetto::{event::AsRawEvent, Libinput};
use devil::sys::udev_device_get_devpath;
use log::debug;
use rustix::{
    fd::{FromRawFd, IntoRawFd, OwnedFd},
    fs::{open, Mode, OFlags},
    io::Errno,
};

use udev_device_extensions::PubDevice;
use wii_remote::WiiRemote;

static CURRENT_TIME: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(true);

fn main() {
    let matches = Command::new(crate_name!())
        .about(crate_description!())
        .author(crate_authors!(", "))
        .arg_required_else_help(true)
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

    let mut libinput = Libinput::new(
        |path, flags| {
            open(path, OFlags::from_bits_retain(flags as u32), Mode::empty())
                .map(IntoRawFd::into_raw_fd)
                .map_err(Errno::raw_os_error)
        },
        |fd| drop(unsafe { OwnedFd::from_raw_fd(fd) }),
    )
    .expect("Failed to initialize libinput.");

    libinput
        .udev_assign_seat(c"seat0")
        .expect("Failed to assign seat");

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

        loop {
            let Some(event) = libinput.get_event() else {
                if let Err(err) = libinput.dispatch() {
                    error!("libinput dispatch error: {:?}", err);
                    break;
                }
                continue;
            };

            let udev_device = event
                .device()
                .udev_device()
                .expect("Failed to get udev device");

            let udev_device_path = match unsafe {
                let udev_device_pub = PubDevice::new(udev_device);
                CStr::from_ptr(udev_device_get_devpath(udev_device_pub.raw)).to_str()
            } {
                Ok(path) => path,
                Err(_) => {
                    error!("Invalid UTF-8 in udev device path");
                    continue;
                }
            };

            if udev_device_path != wii_remote_udev_device_path.as_str() {
                debug!("Ignoring event from unrelated device: {}", udev_device_path);
                continue;
            }

            let current_time = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
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
