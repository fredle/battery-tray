//! Plain Win32 HID: finding HID collections (SetupDi), opening them, and
//! overlapped reads and writes with timeouts, so a device that does not
//! answer never holds the poll thread for long. The protocols on top are in
//! `hidpp.rs` (Logitech) and `sony.rs` (PlayStation controllers).
//!
//! Only the collections those modules ask for are opened, and only with
//! the access they need.

use std::time::{Duration, Instant};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_IDW, CM_Get_Parent, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS, HidD_FreePreparsedData, HidD_GetAttributes,
    HidD_GetHidGuid, HidD_GetPreparsedData, HidP_GetCaps, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
    WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
use windows::core::PCWSTR;

use crate::devices::{self, Reading};

/// The whole HID part of a poll may take at most this long.
pub const BUDGET: Duration = Duration::from_millis(2000);

/// One HID collection Windows lists.
#[derive(Clone, Debug)]
pub struct Info {
    /// NUL-terminated device interface path.
    pub path: Vec<u16>,
    pub vendor: u16,
    pub product: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: usize,
    pub output_len: usize,
    /// The device node one level up: collections of the same USB interface
    /// or Bluetooth device share it.
    pub parent: u32,
    /// The Bluetooth address if the device is reached over Bluetooth.
    pub address: Option<String>,
}

/// Every present HID collection whose vendor is one of `vendors`.
pub fn list(vendors: &[u16]) -> Vec<Info> {
    let mut found = Vec::new();
    unsafe {
        let guid = HidD_GetHidGuid();
        let Ok(set) = SetupDiGetClassDevsW(Some(&guid), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)
        else {
            return found;
        };
        let mut index = 0;
        loop {
            let mut interface = SP_DEVICE_INTERFACE_DATA {
                cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInterfaces(set, None, &guid, index, &mut interface).is_err() {
                break;
            }
            index += 1;
            let mut required = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &interface, None, 0, Some(&mut required), None);
            if required == 0 {
                continue;
            }
            // A u32 buffer keeps the structure aligned.
            let mut buffer = vec![0u32; (required as usize).div_ceil(4)];
            let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let mut node = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiGetDeviceInterfaceDetailW(set, &interface, Some(detail), required, None, Some(&mut node)).is_err() {
                continue;
            }
            let path_ptr = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
            let mut path = Vec::new();
            for i in 0.. {
                let unit = *path_ptr.add(i);
                path.push(unit);
                if unit == 0 {
                    break;
                }
            }
            if let Some(info) = describe(path, node.DevInst, vendors) {
                found.push(info);
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    found
}

/// Attributes and capabilities, read with no access rights (which works
/// even for keyboards and mice Windows holds exclusively).
fn describe(path: Vec<u16>, node: u32, vendors: &[u16]) -> Option<Info> {
    let handle = open_raw(&path, 0)?;
    let result = unsafe {
        let mut attributes = HIDD_ATTRIBUTES {
            Size: size_of::<HIDD_ATTRIBUTES>() as u32,
            ..Default::default()
        };
        let ok = HidD_GetAttributes(handle.0, &mut attributes);
        if !ok || !vendors.contains(&attributes.VendorID) {
            None
        } else {
            let mut preparsed = PHIDP_PREPARSED_DATA::default();
            if !HidD_GetPreparsedData(handle.0, &mut preparsed) {
                None
            } else {
                let mut caps = HIDP_CAPS::default();
                let status = HidP_GetCaps(preparsed, &mut caps);
                let _ = HidD_FreePreparsedData(preparsed);
                (status == HIDP_STATUS_SUCCESS).then(|| Info {
                    path: path.clone(),
                    vendor: attributes.VendorID,
                    product: attributes.ProductID,
                    usage_page: caps.UsagePage,
                    usage: caps.Usage,
                    input_len: caps.InputReportByteLength as usize,
                    output_len: caps.OutputReportByteLength as usize,
                    parent: parent(node).unwrap_or(0),
                    address: bluetooth_address(node),
                })
            }
        }
    };
    drop(handle);
    result
}

fn parent(node: u32) -> Option<u32> {
    let mut parent = 0u32;
    (unsafe { CM_Get_Parent(&mut parent, node, 0) } == CR_SUCCESS).then_some(parent)
}

fn device_id(node: u32) -> Option<String> {
    let mut buffer = [0u16; 512];
    if unsafe { CM_Get_Device_IDW(node, &mut buffer, 0) } != CR_SUCCESS {
        return None;
    }
    let len = buffer.iter().position(|u| *u == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}

/// Walks up from a HID collection to the Bluetooth device it belongs to,
/// if any, and returns that device's address.
fn bluetooth_address(node: u32) -> Option<String> {
    let mut current = node;
    for _ in 0..4 {
        if let Some(id) = device_id(current)
            && is_bluetooth_id(&id)
            && let Some(address) = devices::address_in_instance_id(&id)
        {
            return Some(address);
        }
        current = parent(current)?;
    }
    None
}

/// `BTHENUM\…`, `BTHLE\…`, `BTHLEDevice\…`: device IDs under Windows'
/// Bluetooth enumerators.
pub fn is_bluetooth_id(id: &str) -> bool {
    id.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("BTH"))
}

/// An owned handle, closed on drop.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

fn open_raw(path: &[u16], access: u32) -> Option<Owned> {
    unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    }
    .ok()
    .map(Owned)
}

/// An open HID collection with one read kept outstanding.
pub struct Device {
    pub info: Info,
    handle: Owned,
    event: Owned,
    overlapped: Box<OVERLAPPED>,
    buffer: Box<[u8]>,
    pending: bool,
}

impl Device {
    /// Opens for reading, and for writing too when `write` is set.
    pub fn open(info: &Info, write: bool) -> Option<Device> {
        let access = if write { GENERIC_READ.0 | GENERIC_WRITE.0 } else { GENERIC_READ.0 };
        let handle = open_raw(&info.path, access)?;
        let event = Owned(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?);
        Some(Device {
            info: info.clone(),
            handle,
            event,
            overlapped: Box::default(),
            buffer: vec![0u8; info.input_len.max(1)].into_boxed_slice(),
            pending: false,
        })
    }

    /// Sends one output report, padded to the collection's report length.
    pub fn write(&mut self, report: &[u8]) -> bool {
        let mut data = report.to_vec();
        data.resize(self.info.output_len.max(report.len()), 0);
        unsafe {
            let Ok(event) = CreateEventW(None, true, false, PCWSTR::null()) else {
                return false;
            };
            let event = Owned(event);
            let mut overlapped = OVERLAPPED { hEvent: event.0, ..Default::default() };
            let started = WriteFile(self.handle.0, Some(&data), None, Some(&mut overlapped));
            if let Err(e) = started
                && e.code() != ERROR_IO_PENDING.to_hresult()
            {
                return false;
            }
            let handles = [event.0];
            if WaitForMultipleObjects(&handles, false, 500) != WAIT_OBJECT_0 {
                let _ = CancelIoEx(self.handle.0, Some(&overlapped));
            }
            let mut written = 0u32;
            // Waits for the cancel, if there was one, so `data` and
            // `overlapped` outlive the write.
            GetOverlappedResult(self.handle.0, &overlapped, &mut written, true).is_ok()
        }
    }

    /// Makes sure a read is outstanding. False if reading failed.
    fn arm(&mut self) -> bool {
        if self.pending {
            return true;
        }
        *self.overlapped = OVERLAPPED { hEvent: self.event.0, ..Default::default() };
        let started = unsafe { ReadFile(self.handle.0, Some(&mut self.buffer), None, Some(&mut *self.overlapped)) };
        match started {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
            Err(_) => return false,
        }
        self.pending = true;
        true
    }

    /// The report the finished read got.
    fn take(&mut self) -> Option<Vec<u8>> {
        let mut read = 0u32;
        let result = unsafe { GetOverlappedResult(self.handle.0, &*self.overlapped, &mut read, false) };
        self.pending = false;
        result.ok()?;
        Some(self.buffer[..read as usize].to_vec())
    }

    /// The next input report, or `None` by the deadline.
    pub fn read(&mut self, deadline: Instant) -> Option<Vec<u8>> {
        read_any(&mut [self], deadline).map(|(_, report)| report)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if self.pending {
            unsafe {
                let _ = CancelIoEx(self.handle.0, Some(&*self.overlapped));
                let mut read = 0u32;
                let _ = GetOverlappedResult(self.handle.0, &*self.overlapped, &mut read, true);
            }
        }
    }
}

/// The next input report from any of `devices`, with its index, or `None`
/// by the deadline.
pub fn read_any(devices: &mut [&mut Device], deadline: Instant) -> Option<(usize, Vec<u8>)> {
    loop {
        for device in devices.iter_mut() {
            if !device.arm() {
                return None;
            }
        }
        let handles: Vec<HANDLE> = devices.iter().map(|d| d.event.0).collect();
        let left = deadline.saturating_duration_since(Instant::now());
        let wait = unsafe { WaitForMultipleObjects(&handles, false, left.as_millis() as u32) };
        if wait == WAIT_TIMEOUT {
            return None;
        }
        let index = wait.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        let device = devices.get_mut(index)?;
        if let Some(report) = device.take() {
            return Some((index, report));
        }
        if Instant::now() >= deadline {
            return None;
        }
    }
}

/// The HID sources together: one pass over the HID devices per poll, with
/// one time budget shared by all of them.
#[derive(Default)]
pub struct Hid {
    logitech: crate::hidpp::Logitech,
}

impl Hid {
    pub fn read(&mut self) -> Option<Vec<Reading>> {
        let deadline = Instant::now() + BUDGET;
        let infos = list(&[crate::hidpp::VENDOR, crate::sony::VENDOR]);
        let mut readings = crate::sony::read(&infos, deadline);
        readings.extend(self.logitech.read(&infos, deadline));
        Some(readings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_device_ids_are_recognised() {
        assert!(is_bluetooth_id(r"BTHENUM\{00001124-0000-1000-8000-00805F9B34FB}_VID&0002054C_PID&0CE6\8&1&0&AABBCCDDEEFF_C00000000"));
        assert!(is_bluetooth_id(r"BTHLEDevice\{00001812-0000-1000-8000-00805f9b34fb}_Dev_VID&02046d_PID&b023_REV&0010_e1a2b3c4d5e6\9&1&0&0020"));
        assert!(!is_bluetooth_id(r"USB\VID_046D&PID_C52B&MI_02\7&1&0&2"));
        assert!(!is_bluetooth_id("BT"));
    }

    /// Lists the HID collections of the supported vendors on this PC.
    /// `cargo test list_for_real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn list_for_real() {
        let all = list(&[crate::hidpp::VENDOR, crate::sony::VENDOR, 0x045e, 0x1532, 0x056d, 0x047f]);
        for info in &all {
            println!(
                "{:04x}:{:04x} page {:04x} usage {:04x} in {} out {} parent {} bt {:?} {}",
                info.vendor,
                info.product,
                info.usage_page,
                info.usage,
                info.input_len,
                info.output_len,
                info.parent,
                info.address,
                String::from_utf16_lossy(&info.path[..info.path.len() - 1])
            );
        }
        let mut hid = Hid::default();
        let started = Instant::now();
        println!("{:?} in {:?}", hid.read(), started.elapsed());
    }
}
