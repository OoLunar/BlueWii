use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

use anyhow::Context;

use crate::utils::FormattedUnwrap;

pub struct WiiRemote {
    pub bluetooth_address: String,
}

impl WiiRemote {
    pub const fn new() -> WiiRemote {
        WiiRemote {
            bluetooth_address: String::new(),
        }
    }

    pub fn try_connect(&mut self) -> bool {
        if WiiRemote::is_connected(self) {
            return true;
        }

        // If we're not connected to a Wii Remote, try to connect to one
        let bluetoothctl_status = Command::new("bluetoothctl")
            .arg("-t 30")
            .arg("scan on")
            .stdout(Stdio::piped())
            .spawn()
            .context("Failed to execute `bluetoothctl -t 30 scan on'")
            .unwrap_or_fmt();

        // Read the output of the `bluetoothctl -t 30 scan on` command
        let bluetoothctl_status_output = bluetoothctl_status
            .stdout
            .context("Failed to read out of `bluetoothctl -t 30 scan on'")
            .unwrap_or_fmt();

        // Read the output of the `bluetoothctl -t 30 scan on` command as it comes in
        self.bluetooth_address = String::new();
        let stdout_reader = BufReader::new(bluetoothctl_status_output);
        for line in stdout_reader.lines() {
            let line = line
                .context("Failed to read line from `bluetoothctl -t 30 scan on' output")
                .unwrap_or_fmt();

            if !line.contains("RVL") {
                continue;
            }

            self.bluetooth_address = line.split_whitespace().nth(2).unwrap().to_owned();
        }

        // Test to see if we found a Wii Remote
        if self.bluetooth_address.is_empty() {
            return false;
        }

        // Try executing the `bluetoothctl connect` command
        let _bluetoothctl_connect_output = Command::new("bluetoothctl")
            .arg("connect")
            .arg(&self.bluetooth_address)
            .output()
            .context("Failed to execute `bluetoothctl connect'")
            .unwrap_or_fmt();

        // If we've reached this point, we failed to connect to a Wii Remote
        return true;
    }

    pub fn is_connected(&mut self) -> bool {
        // First, check to see if we're connected to any Wii Remotes
        // Normally we'd execute this in Bash: `bluetoothctl devices | grep RVL | cut -d " " -f 2 | bluetoothctl info | grep "Connected: yes"`
        let bluetoothctl_devices_output = Command::new("bluetoothctl")
            .arg("devices")
            .output()
            .context("Failed to execute `bluetoothctl devices'")
            .unwrap_or_fmt();

        let bluetoothctl_devices_str = std::str::from_utf8(&bluetoothctl_devices_output.stdout)
            .context("Failed to convert `bluetoothctl devices' output to a string.")
            .unwrap_or_fmt();

        for line in bluetoothctl_devices_str.lines() {
            if !line.contains("RVL") {
                continue;
            }

            self.bluetooth_address = line.split_whitespace().nth(1).unwrap().to_owned();
            return true;
        }

        return false;
    }

    pub fn disconnect(&mut self) {
        // Execute `bluetoothctl disconnect <bluetooth_address>`
        let _bluetoothctl_disconnect_output = Command::new("bluetoothctl")
            .arg("disconnect")
            .arg(&self.bluetooth_address)
            .output()
            .context("Failed to execute `bluetoothctl disconnect'")
            .unwrap_or_fmt();
    }

    pub fn get_udev_device_path(&self) -> Option<String> {
        // Execute `xwiishow list`
        let xwiishow_output = Command::new("xwiishow")
            .arg("list")
            .output()
            .context("Failed to execute `xwiishow list'")
            .unwrap_or_fmt();

        let xwiishow_str = std::str::from_utf8(&xwiishow_output.stdout)
            .context("Failed to convert `xwiishow list' output to a string.")
            .unwrap_or_fmt();

        /*
        The output will look like this:
        ```
        Listing connected Wii Remote devices:
          Found device #1: /sys/devices/virtual/misc/uhid/0005:057E:0306.0006
        End of device list
        ```
        So we should only parse lines that contain "Found device #1" and splice by the first colon
        */
        for line in xwiishow_str.lines() {
            if !line.contains("Found device #1") {
                continue;
            }

            let udev_device_path = line.split(":").skip(1).collect::<String>();
            return Some(udev_device_path);
        }

        return None;
    }
}
