//! Publishes the headset battery to the OS through a virtual HID device.
//!
//! UPower has no API for adding batteries, but it lists every kernel
//! power_supply. Creating a HID device via /dev/uhid whose report descriptor
//! contains a Battery Strength input makes the kernel's hid-input register a
//! peripheral power_supply ("hid-a50-headset-battery"), which UPower and the
//! desktop's battery UI then show like a Bluetooth headset.
//!
//! hid-input only keeps the battery if the device also yields an input device,
//! hence the never-pressed Mute key in the descriptor. That input device also
//! decides the battery's type in UPower: a udev rule tags it as a headset
//! sound device (see packaging/70-a50-control.rules).

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;

const UHID_DESTROY: u32 = 1;
const UHID_GET_REPORT: u32 = 9;
const UHID_GET_REPORT_REPLY: u32 = 10;
const UHID_CREATE2: u32 = 11;
const UHID_INPUT2: u32 = 12;
const UHID_SET_REPORT: u32 = 13;
const UHID_SET_REPORT_REPLY: u32 = 14;
const EVENT_SIZE: usize = 4 + 4372;
const BUS_USB: u16 = 0x03;
const EIO: u16 = 5;

/// Product id for the virtual device, distinct from the real base (0x002c).
pub const VIRTUAL_PRODUCT: u32 = 0xa50b;

// Consumer Control application collection, report ID 1:
//   byte 1: Battery Strength 0–100 (Generic Device Controls 0x06 / 0x20)
//   byte 2: bit 0 Charging (Battery System 0x85 / 0x44), bit 1 Mute key
#[rustfmt::skip]
const DESCRIPTOR: &[u8] = &[
	0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01,
	0x85, 0x01,
	0x05, 0x06, 0x09, 0x20,
	0x15, 0x00, 0x26, 0x64, 0x00,
	0x75, 0x08, 0x95, 0x01,
	0x81, 0x02,
	0x05, 0x85, 0x09, 0x44,
	0x15, 0x00, 0x25, 0x01,
	0x75, 0x01, 0x95, 0x01,
	0x81, 0x02,
	0x05, 0x0c, 0x09, 0xe2,
	0x75, 0x01, 0x95, 0x01,
	0x81, 0x02,
	0x75, 0x06, 0x95, 0x01,
	0x81, 0x03,
	0xc0,
];

pub struct VirtualBattery {
	file: File,
	report: [u8; 3],
}

impl VirtualBattery {
	pub fn create(percent: u8, charging: bool) -> std::io::Result<VirtualBattery> {
		let mut file = OpenOptions::new()
			.read(true)
			.write(true)
			.custom_flags(libc::O_NONBLOCK)
			.open("/dev/uhid")?;

		let mut ev = vec![0u8; EVENT_SIZE];
		ev[0..4].copy_from_slice(&UHID_CREATE2.to_le_bytes());
		let mut o = 4;
		put_str(&mut ev[o..o + 128], "Astro A50 Headset");
		o += 128;
		put_str(&mut ev[o..o + 64], "a50-control/battery");
		o += 64;
		put_str(&mut ev[o..o + 64], "a50-headset");
		o += 64;
		ev[o..o + 2].copy_from_slice(&(DESCRIPTOR.len() as u16).to_le_bytes());
		o += 2;
		ev[o..o + 2].copy_from_slice(&BUS_USB.to_le_bytes());
		o += 2;
		ev[o..o + 4].copy_from_slice(&0x9886u32.to_le_bytes());
		o += 4;
		ev[o..o + 4].copy_from_slice(&VIRTUAL_PRODUCT.to_le_bytes());
		o += 4;
		ev[o..o + 4].copy_from_slice(&1u32.to_le_bytes());
		o += 4 + 4; // version, country
		ev[o..o + DESCRIPTOR.len()].copy_from_slice(DESCRIPTOR);
		file.write_all(&ev)?;

		let mut vb = VirtualBattery { file, report: [0x01, 0, 0] };
		vb.update(percent, charging)?;
		Ok(vb)
	}

	pub fn update(&mut self, percent: u8, charging: bool) -> std::io::Result<()> {
		self.report = [0x01, percent.min(100), charging as u8];
		self.send_input()
	}

	fn send_input(&mut self) -> std::io::Result<()> {
		let mut ev = vec![0u8; EVENT_SIZE];
		ev[0..4].copy_from_slice(&UHID_INPUT2.to_le_bytes());
		ev[4..6].copy_from_slice(&(self.report.len() as u16).to_le_bytes());
		ev[6..6 + self.report.len()].copy_from_slice(&self.report);
		self.file.write_all(&ev)
	}

	/// Answers pending kernel requests. Must be called regularly: the kernel
	/// waits on GET/SET_REPORT replies, and unread events fill the queue.
	pub fn service(&mut self) -> std::io::Result<()> {
		let mut buf = vec![0u8; EVENT_SIZE];
		loop {
			let n = match self.file.read(&mut buf) {
				Ok(n) => n,
				Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
				Err(e) => return Err(e),
			};
			if n < 4 {
				return Ok(());
			}
			let kind = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
			let id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
			match kind {
				UHID_GET_REPORT => {
					let mut ev = vec![0u8; EVENT_SIZE];
					ev[0..4].copy_from_slice(&UHID_GET_REPORT_REPLY.to_le_bytes());
					ev[4..8].copy_from_slice(&id.to_le_bytes());
					if buf[8] == self.report[0] {
						ev[10..12].copy_from_slice(&(self.report.len() as u16).to_le_bytes());
						ev[12..12 + self.report.len()].copy_from_slice(&self.report);
					}
					else {
						ev[8..10].copy_from_slice(&EIO.to_le_bytes());
					}
					self.file.write_all(&ev)?;
				}
				UHID_SET_REPORT => {
					let mut ev = vec![0u8; EVENT_SIZE];
					ev[0..4].copy_from_slice(&UHID_SET_REPORT_REPLY.to_le_bytes());
					ev[4..8].copy_from_slice(&id.to_le_bytes());
					ev[8..10].copy_from_slice(&EIO.to_le_bytes());
					self.file.write_all(&ev)?;
				}
				// START/OPEN: (re)send the current level for new listeners.
				2 | 4 => self.send_input()?,
				_ => {}
			}
		}
	}
}

impl Drop for VirtualBattery {
	fn drop(&mut self) {
		let mut ev = vec![0u8; EVENT_SIZE];
		ev[0..4].copy_from_slice(&UHID_DESTROY.to_le_bytes());
		let _ = self.file.write_all(&ev);
	}
}

fn put_str(dst: &mut [u8], s: &str) {
	let n = s.len().min(dst.len() - 1);
	dst[..n].copy_from_slice(&s.as_bytes()[..n]);
}

/// True if this process may open /dev/uhid (udev rule installed).
pub fn accessible() -> bool {
	OpenOptions::new().read(true).write(true).open("/dev/uhid").is_ok()
}
