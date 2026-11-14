//! Battery levels of paired Bluetooth devices, as Windows knows them.
//!
//! - Paired devices, their names, kinds and whether they are connected come
//!   from WinRT (`Windows.Devices.Bluetooth`), for classic Bluetooth and
//!   Bluetooth LE separately.
//! - A Bluetooth LE device's level is read from its GATT Battery Service
//!   (`GattServiceUuids.Battery`, 0x180F) and Battery Level characteristic
//!   (`GattCharacteristicUuids.BatteryLevel`, 0x2A19): one byte, percent.
//! - A classic device's level is the device property
//!   `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}` 2 that Windows' Bluetooth stack
//!   sets on the device's nodes from the Hands-Free profile (the number
//!   Windows Settings shows). Microsoft does not document this property; it
//!   is widely used, a single byte, and simply absent on devices that do not
//!   report a level. Windows sets the same property for LE devices it reads
//!   itself, so it is also the fallback when the GATT read is refused.
//!
//! Only connected devices are asked: a GATT request to a device that is off
//! would make Windows try to connect and block for seconds. Nothing here
//! pairs, connects or changes a device.
//!
//! This module could not be tested against real hardware where it was
//! written; `source.rs` has a simulated source for everything above it.

use std::collections::HashMap;

use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristicUuids, GattCommunicationStatus, GattServiceUuids,
};
use windows::Devices::Bluetooth::{
    BluetoothCacheMode, BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice,
};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Storage::Streams::DataReader;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GETIDLIST_FILTER_ENUMERATOR, CM_GETIDLIST_FILTER_PRESENT, CM_Get_DevNode_PropertyW,
    CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW, CM_LOCATE_DEVNODE_NORMAL,
    CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPROP_TYPE_BYTE, DEVPROPTYPE};
use windows::Win32::Foundation::DEVPROPKEY;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::{GUID, HSTRING, PCWSTR, Result};

use crate::devices::{self, Battery, Kind, Reading};
use crate::source::Source;

/// The battery percentage Windows' Bluetooth stack keeps on a device node.
const BATTERY_PROPERTY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

pub struct Windows;

impl Source for Windows {
    fn read(&mut self) -> Option<Vec<Reading>> {
        // WinRT needs COM on this thread; a repeat call is harmless.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let node_levels = node_levels();
        let mut readings = low_energy(&node_levels).ok()?;
        // A dual-mode device is paired both ways; keep the one with a level.
        for classic in classic(&node_levels).ok()? {
            match readings.iter_mut().find(|r| r.id == classic.id) {
                Some(existing) => {
                    if !matches!(existing.battery, Battery::Level(_)) {
                        *existing = classic;
                    }
                }
                None => readings.push(classic),
            }
        }
        Some(readings)
    }
}

fn paired(selector: &HSTRING) -> Result<Vec<HSTRING>> {
    let found = DeviceInformation::FindAllAsyncAqsFilter(selector)?.join()?;
    (0..found.Size()?).map(|i| found.GetAt(i)?.Id()).collect()
}

fn classic(node_levels: &HashMap<String, u8>) -> Result<Vec<Reading>> {
    let mut readings = Vec::new();
    for id in paired(&BluetoothDevice::GetDeviceSelectorFromPairingState(true)?)? {
        // One device that cannot be opened must not hide the others.
        let Ok(device) = BluetoothDevice::FromIdAsync(&id).and_then(|op| op.join()) else {
            continue;
        };
        let address = devices::address_id(device.BluetoothAddress()?);
        let connected = device.ConnectionStatus()? == BluetoothConnectionStatus::Connected;
        readings.push(Reading {
            name: device.Name()?.to_string(),
            kind: Kind::from_class_of_device(device.ClassOfDevice()?.RawValue()?),
            le: false,
            connected,
            battery: match node_levels.get(&address) {
                Some(level) => Battery::Level(*level),
                None => Battery::NotReported,
            },
            id: address,
        });
    }
    Ok(readings)
}

fn low_energy(node_levels: &HashMap<String, u8>) -> Result<Vec<Reading>> {
    let mut readings = Vec::new();
    for id in paired(&BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?)? {
        let Ok(device) = BluetoothLEDevice::FromIdAsync(&id).and_then(|op| op.join()) else {
            continue;
        };
        let address = devices::address_id(device.BluetoothAddress()?);
        let connected = device.ConnectionStatus()? == BluetoothConnectionStatus::Connected;
        let battery = if connected {
            match gatt_level(&device) {
                Ok(Some(level)) => Battery::Level(level),
                other => match (node_levels.get(&address), other) {
                    (Some(level), _) => Battery::Level(*level),
                    (None, Ok(None)) => Battery::NotReported,
                    (None, _) => Battery::Unreadable,
                },
            }
        } else {
            Battery::Unreadable
        };
        readings.push(Reading {
            name: device.Name()?.to_string(),
            kind: Kind::from_appearance(device.Appearance()?.RawValue()?),
            le: true,
            connected,
            battery,
            id: address,
        });
    }
    Ok(readings)
}

/// `Ok(None)` means the device has no Battery Service; an error means it has
/// not answered.
fn gatt_level(device: &BluetoothLEDevice) -> Result<Option<u8>> {
    let unreachable = || windows::core::Error::from(windows::Win32::Foundation::E_FAIL);
    let services = device
        .GetGattServicesForUuidWithCacheModeAsync(
            GattServiceUuids::Battery()?,
            BluetoothCacheMode::Uncached,
        )?
        .join()?;
    if services.Status()? != GattCommunicationStatus::Success {
        return Err(unreachable());
    }
    let services = services.Services()?;
    if services.Size()? == 0 {
        return Ok(None);
    }
    let service = services.GetAt(0)?;
    let characteristics = service
        .GetCharacteristicsForUuidWithCacheModeAsync(
            GattCharacteristicUuids::BatteryLevel()?,
            BluetoothCacheMode::Uncached,
        )?
        .join()?;
    if characteristics.Status()? != GattCommunicationStatus::Success {
        return Err(unreachable());
    }
    let characteristics = characteristics.Characteristics()?;
    if characteristics.Size()? == 0 {
        return Ok(None);
    }
    let read = characteristics
        .GetAt(0)?
        .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?
        .join()?;
    if read.Status()? != GattCommunicationStatus::Success {
        return Err(unreachable());
    }
    let value = read.Value()?;
    if value.Length()? == 0 {
        return Err(unreachable());
    }
    let level = DataReader::FromBuffer(&value)?.ReadByte()?;
    let _ = service.Close();
    Ok(Some(level.min(100)))
}

/// Battery levels Windows holds on present Bluetooth device nodes, by
/// Bluetooth address.
fn node_levels() -> HashMap<String, u8> {
    let mut levels = HashMap::new();
    for enumerator in ["BTHENUM", "BTHLE"] {
        for instance_id in instance_ids(enumerator) {
            let Some(address) = devices::address_in_instance_id(&instance_id) else {
                continue;
            };
            if let Some(level) = node_level(&instance_id) {
                levels.entry(address).or_insert(level);
            }
        }
    }
    levels
}

/// Instance IDs of the devices currently present under one enumerator.
fn instance_ids(enumerator: &str) -> Vec<String> {
    let filter = HSTRING::from(enumerator);
    let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
    let mut length = 0u32;
    unsafe {
        if CM_Get_Device_ID_List_SizeW(&mut length, &filter, flags) != CR_SUCCESS || length == 0 {
            return Vec::new();
        }
        let mut buffer = vec![0u16; length as usize];
        if CM_Get_Device_ID_ListW(&filter, &mut buffer, flags) != CR_SUCCESS {
            return Vec::new();
        }
        // A list of NUL-terminated strings, ended by an empty one.
        buffer
            .split(|unit| *unit == 0)
            .filter(|id| !id.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
}

fn node_level(instance_id: &str) -> Option<u8> {
    let wide: Vec<u16> = instance_id.encode_utf16().chain([0]).collect();
    let mut node = 0u32;
    let mut kind = DEVPROPTYPE(0);
    let mut value = [0u8; 4];
    let mut size = value.len() as u32;
    unsafe {
        if CM_Locate_DevNodeW(&mut node, PCWSTR(wide.as_ptr()), CM_LOCATE_DEVNODE_NORMAL)
            != CR_SUCCESS
        {
            return None;
        }
        if CM_Get_DevNode_PropertyW(
            node,
            &BATTERY_PROPERTY,
            &mut kind,
            Some(value.as_mut_ptr()),
            &mut size,
            0,
        ) != CR_SUCCESS
        {
            return None;
        }
    }
    (kind == DEVPROP_TYPE_BYTE && size == 1 && value[0] <= 100).then_some(value[0])
}
