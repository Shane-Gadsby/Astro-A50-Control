//! Raw transport to the Astro A50 Gen 4 base station over /dev/hidraw.
//!
//! Every request is `02 <cmd> [<len> <payload…>]` padded to 64 bytes, and every
//! reply is `02 <result> <len> <data…>` (result 0x02 = OK, 0x01 = error with a
//! NUL-terminated message after 4 unknown bytes).
//!
//! hidraw hands every input report to *every* process that has the node open,
//! so a reply on our fd may belong to someone else's request (this is how a
//! Wine prefix once made the headset look permanently on). To stay robust we:
//!   - drain anything already queued before sending a request,
//!   - accept a reply only if it matches the request (echoed command byte where
//!     the firmware echoes it, expected shape otherwise),
//!   - poll with a timeout instead of blocking forever on read(),
//!   - after a timeout, wait briefly and drain so a late reply can't be
//!     mistaken for the answer to the next request.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const HID_ID: &str = "HID_ID=0003:00009886:0000002C";
const REPORT_LEN: usize = 64;
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1500);
// SAVE can take over two seconds while the base writes flash.
pub const SLOW_TIMEOUT: Duration = Duration::from_millis(4000);

#[derive(Debug)]
pub enum DevError {
	NotFound,
	Io(std::io::Error),
	Timeout,
	/// Error message reported by the base, e.g. HID_ERROR_SLAVE_NO_SLAVE.
	Device(String),
	BadReply(String),
}

impl DevError {
	/// True when the base itself is gone (unplugged, fd dead), as opposed to a
	/// request that merely failed.
	pub fn is_fatal(&self) -> bool {
		matches!(self, DevError::NotFound | DevError::Io(_))
	}

	/// True when the request needs the headset and it is off and undocked.
	pub fn is_no_headset(&self) -> bool {
		matches!(self, DevError::Device(m) if m.contains("NO_SLAVE") || m.contains("SLAVE_TIMEOUT"))
	}
}

impl fmt::Display for DevError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			DevError::NotFound => write!(f, "A50 base station not found"),
			DevError::Io(e) => write!(f, "I/O error: {e}"),
			DevError::Timeout => write!(f, "no reply from base station"),
			DevError::Device(m) if m.contains("NO_SLAVE") => {
				write!(f, "headset is off — turn it on or dock it")
			}
			DevError::Device(m) => write!(f, "base station error: {m}"),
			DevError::BadReply(m) => write!(f, "unexpected reply: {m}"),
		}
	}
}

impl From<std::io::Error> for DevError {
	fn from(e: std::io::Error) -> Self {
		DevError::Io(e)
	}
}

pub type Result<T> = std::result::Result<T, DevError>;

/// How a reply is recognised as belonging to our request.
#[derive(Clone, Copy)]
pub enum Expect {
	/// data[0] echoes the command byte.
	Echo,
	/// Exactly `n` data bytes, no echo (status, battery, balance, …).
	Len(usize),
}

/// Finds the A50's hidraw node by its HID ID in sysfs.
pub fn find_hidraw() -> Option<PathBuf> {
	let mut nodes: Vec<_> = fs::read_dir("/sys/class/hidraw").ok()?.flatten().collect();
	nodes.sort_by_key(|e| e.file_name());
	nodes.into_iter().find_map(|e| {
		let uevent = fs::read_to_string(e.path().join("device/uevent")).ok()?;
		uevent
			.lines()
			.any(|l| l.eq_ignore_ascii_case(HID_ID))
			.then(|| Path::new("/dev").join(e.file_name()))
	})
}

/// Lists other processes holding `path` open, as (pid, command line).
pub fn other_holders(path: &Path) -> Vec<(u32, String)> {
	let me = std::process::id();
	let Ok(procs) = fs::read_dir("/proc") else {
		return Vec::new();
	};
	procs
		.flatten()
		.filter_map(|p| {
			let pid: u32 = p.file_name().to_str()?.parse().ok()?;
			if pid == me {
				return None;
			}
			let holds = fs::read_dir(p.path().join("fd"))
				.ok()?
				.flatten()
				.any(|fd| fs::read_link(fd.path()).map(|l| l == path).unwrap_or(false));
			if !holds {
				return None;
			}
			let cmd = fs::read(p.path().join("cmdline")).unwrap_or_default();
			let cmd = String::from_utf8_lossy(&cmd).replace('\0', " ").trim().to_string();
			Some((pid, cmd))
		})
		.collect()
}

pub struct Hid {
	file: File,
	pub path: PathBuf,
}

impl Hid {
	pub fn open() -> Result<Hid> {
		let path = find_hidraw().ok_or(DevError::NotFound)?;
		let file = OpenOptions::new()
			.read(true)
			.write(true)
			.custom_flags(libc::O_NONBLOCK)
			.open(&path)?;
		Ok(Hid { file, path })
	}

	fn drain(&mut self) {
		let mut buf = [0u8; REPORT_LEN];
		while matches!(self.file.read(&mut buf), Ok(n) if n > 0) {}
	}

	/// Waits up to `timeout` for one report.
	fn read_report(&mut self, timeout: Duration) -> Result<Option<[u8; REPORT_LEN]>> {
		let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
		let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
		// SAFETY: pfd is a valid pollfd for the duration of the call.
		let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
		if rc < 0 {
			let err = std::io::Error::last_os_error();
			return if err.kind() == std::io::ErrorKind::Interrupted { Ok(None) } else { Err(err.into()) };
		}
		if rc == 0 {
			return Ok(None);
		}
		if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
			return Err(DevError::Io(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "hidraw closed")));
		}
		let mut buf = [0u8; REPORT_LEN];
		match self.file.read(&mut buf) {
			Ok(n) if n >= 3 => Ok(Some(buf)),
			Ok(_) => Ok(None),
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
			Err(e) => Err(e.into()),
		}
	}

	pub fn request(&mut self, cmd: u8, payload: &[u8], expect: Expect) -> Result<Vec<u8>> {
		self.request_with_timeout(cmd, payload, expect, DEFAULT_TIMEOUT)
	}

	pub fn request_with_timeout(
		&mut self,
		cmd: u8,
		payload: &[u8],
		expect: Expect,
		timeout: Duration,
	) -> Result<Vec<u8>> {
		assert!(payload.len() <= REPORT_LEN - 3);
		let mut req = [0u8; REPORT_LEN];
		req[0] = 0x02;
		req[1] = cmd;
		if !payload.is_empty() {
			req[2] = payload.len() as u8;
			req[3..3 + payload.len()].copy_from_slice(payload);
		}

		self.drain();
		self.file.write_all(&req)?;

		let deadline = Instant::now() + timeout;
		loop {
			let left = deadline.saturating_duration_since(Instant::now());
			if left.is_zero() {
				std::thread::sleep(Duration::from_millis(200));
				self.drain();
				return Err(DevError::Timeout);
			}
			let Some(resp) = self.read_report(left)? else {
				continue;
			};
			if resp[0] != 0x02 {
				continue;
			}
			let len = (resp[2] as usize).min(REPORT_LEN - 3);
			let data = &resp[3..3 + len];
			match resp[1] {
				0x01 => {
					let msg: Vec<u8> = data.iter().skip(4).take_while(|&&c| c != 0).copied().collect();
					return Err(DevError::Device(String::from_utf8_lossy(&msg).into_owned()));
				}
				0x00 | 0x02 => {}
				_ => continue,
			}
			let ours = match expect {
				Expect::Echo => data.first() == Some(&cmd),
				Expect::Len(n) => data.len() == n,
			};
			if ours {
				return Ok(data.to_vec());
			}
			// Someone else's reply; keep waiting for ours.
		}
	}
}
