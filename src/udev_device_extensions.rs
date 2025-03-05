use devil::{sys, Device};

/// Makes the [`Device.raw`](devil::Device) field public.
pub struct PubDevice {
    pub raw: *mut sys::udev_device,
}

impl PubDevice {
    pub unsafe fn new(device: Device) -> Self {
        std::mem::transmute(&device)
    }
}
