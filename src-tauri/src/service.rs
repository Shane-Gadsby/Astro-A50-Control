//! The background service: the only code that talks to the base station.
//!
//! One thread owns the hidraw fd and does everything that needs it, in order:
//! polling headset status, polling the battery, running settings requests from
//! the UI, and saving to flash. Serialising all traffic through one owner is
//! what keeps replies matched to requests (see device.rs).
//!
//! Headset state follows the a50-watcher logic: audio goes to the A50 only when
//! it is switched on and off the dock, and a new state must hold for STABLE
//! before it counts (lifting a switched-off headset reports "on" for ~2 s).

use crate::audio;
use crate::config::Config;
use crate::device::{self, DevError, Hid};
use crate::protocol::{self, Battery, Status};
use crate::uhid::{self, VirtualBattery};
use serde::Serialize;
use serde_json::Value;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(250);
const STABLE: Duration = Duration::from_secs(3);
const BATTERY_EVERY: Duration = Duration::from_secs(30);
const REOPEN_EVERY: Duration = Duration::from_secs(2);
const SAVE_DELAY: Duration = Duration::from_secs(3);
const SINK_DEBOUNCE: Duration = Duration::from_millis(300);
const LOW_BATTERY: [u8; 2] = [15, 5];

pub type CallFn = Box<dyn FnOnce(&mut Hid) -> device::Result<Value> + Send>;

pub enum Msg {
	/// Run a request on the device. `mutates` schedules a save to flash.
	Call { f: CallFn, mutates: bool, reply: Sender<Result<Value, String>> },
	ConfigChanged,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct State {
	pub connected: bool,
	pub device_path: Option<String>,
	pub error: Option<String>,
	/// Other processes holding the hidraw node (their traffic is filtered out).
	pub other_holders: Vec<String>,
	pub status: Option<Status>,
	/// Debounced: switched on and off the dock.
	pub wearing: bool,
	pub battery: Option<Battery>,
	pub battery_published: bool,
	/// Label of the output audio should be on.
	pub audio_target: Option<String>,
	pub audio_sink: Option<String>,
	/// Label of the highest-priority connected Bluetooth headset.
	pub bluetooth_connected: Option<String>,
}

pub struct Hooks {
	pub on_state: Box<dyn Fn(&State) + Send>,
	pub notify: Box<dyn Fn(&str, &str) + Send>,
}

pub fn spawn(state: Arc<Mutex<State>>, config: Arc<Mutex<Config>>, hooks: Hooks) -> Sender<Msg> {
	let (tx, rx) = mpsc::channel();
	let (sink_tx, sink_rx) = mpsc::channel();
	audio::spawn_watcher(sink_tx);
	std::thread::Builder::new()
		.name("a50-service".into())
		.spawn(move || Service::new(state, config, hooks).run(rx, sink_rx))
		.expect("spawn service thread");
	tx
}

struct Service {
	shared: Arc<Mutex<State>>,
	config: Arc<Mutex<Config>>,
	hooks: Hooks,
	st: State,
	hid: Option<Hid>,
	next_open: Instant,
	last_error: Option<String>,
	baselined: bool,
	pending: bool,
	pending_since: Instant,
	next_battery: Instant,
	save_due: Option<Instant>,
	sink_due: Option<Instant>,
	vbat: Option<VirtualBattery>,
	vbat_failed: bool,
	alerted: Vec<u8>,
}

impl Service {
	fn new(shared: Arc<Mutex<State>>, config: Arc<Mutex<Config>>, hooks: Hooks) -> Self {
		let now = Instant::now();
		Service {
			shared,
			config,
			hooks,
			st: State::default(),
			hid: None,
			next_open: now,
			last_error: None,
			baselined: false,
			pending: false,
			pending_since: now,
			next_battery: now,
			save_due: None,
			sink_due: Some(now),
			vbat: None,
			vbat_failed: false,
			alerted: Vec::new(),
		}
	}

	fn cfg(&self) -> Config {
		self.config.lock().unwrap().clone()
	}

	fn run(mut self, rx: Receiver<Msg>, sink_rx: Receiver<()>) {
		crate::log("service started");
		loop {
			match rx.recv_timeout(TICK) {
				Ok(Msg::Call { f, mutates, reply }) => {
					let res = self.call(f, mutates);
					let _ = reply.send(res);
				}
				Ok(Msg::ConfigChanged) => {
					self.vbat_failed = false;
					self.sink_due = Some(Instant::now());
				}
				Err(RecvTimeoutError::Timeout) => {}
				Err(RecvTimeoutError::Disconnected) => return,
			}
			while sink_rx.try_recv().is_ok() {
				self.sink_due = Some(Instant::now() + SINK_DEBOUNCE);
			}
			self.tick();
			self.publish();
		}
	}

	fn tick(&mut self) {
		let now = Instant::now();
		self.ensure_open(now);

		if self.hid.is_some() {
			match self.with_hid(protocol::status) {
				Ok(s) => self.on_status(s, now),
				Err(e) if !e.is_fatal() => self.note_error(&e),
				Err(_) => {}
			}
		}

		if self.hid.is_some() && now >= self.next_battery {
			self.poll_battery(now);
		}

		if let Some(due) = self.save_due {
			if now >= due && self.hid.is_some() {
				self.save_due = None;
				match self.with_hid(protocol::save) {
					Ok(()) => crate::log("settings saved to flash"),
					Err(e) => crate::log(&format!("save failed: {e}")),
				}
			}
		}

		if let Some(due) = self.sink_due {
			if now >= due {
				self.sink_due = None;
				self.apply_audio();
			}
		}

		self.update_vbat();
	}

	fn ensure_open(&mut self, now: Instant) {
		if self.hid.is_some() || now < self.next_open {
			return;
		}
		self.next_open = now + REOPEN_EVERY;
		match Hid::open() {
			Ok(h) => {
				crate::log(&format!("found A50 at {}", h.path.display()));
				self.st.other_holders = device::other_holders(&h.path)
					.into_iter()
					.map(|(pid, cmd)| format!("{cmd} (PID {pid})"))
					.collect();
				for holder in &self.st.other_holders {
					crate::log(&format!("WARNING: {} is also open in {holder}", h.path.display()));
				}
				self.st.connected = true;
				self.st.device_path = Some(h.path.display().to_string());
				self.st.error = None;
				self.last_error = None;
				self.hid = Some(h);
				self.next_battery = now;
			}
			Err(e) => {
				self.st.connected = false;
				self.st.device_path = None;
				self.note_error(&e);
			}
		}
	}

	/// Runs `f` on the device; drops the device on a fatal error so the next
	/// tick re-discovers it (a replug can renumber the hidraw node).
	fn with_hid<T>(&mut self, f: impl FnOnce(&mut Hid) -> device::Result<T>) -> device::Result<T> {
		let hid = self.hid.as_mut().ok_or(DevError::NotFound)?;
		let res = f(hid);
		if let Err(e) = &res {
			if e.is_fatal() {
				crate::log(&format!("lost A50: {e}"));
				self.hid = None;
				self.st.connected = false;
				self.st.device_path = None;
				self.st.status = None;
				self.st.battery = None;
				self.note_error(e);
			}
		}
		res
	}

	fn note_error(&mut self, e: &DevError) {
		let msg = e.to_string();
		if self.last_error.as_deref() != Some(&msg) {
			crate::log(&format!("WARNING: {msg}"));
			self.last_error = Some(msg.clone());
		}
		self.st.error = Some(msg);
	}

	fn on_status(&mut self, s: Status, now: Instant) {
		let reachable_changed = self.st.status.map(|o| o.on || o.docked) != Some(s.on || s.docked);
		self.st.status = Some(s);
		self.st.error = None;
		self.last_error = None;
		if reachable_changed {
			// Headset just became reachable/unreachable: refresh battery soon.
			self.next_battery = now + Duration::from_millis(500);
		}

		let wearing = s.on && !s.docked;
		if !self.baselined {
			self.baselined = true;
			self.pending = wearing;
			self.st.wearing = wearing;
			crate::log(&format!("initial state: {}", describe(s)));
			self.sink_due = Some(now);
			return;
		}
		if wearing != self.pending {
			self.pending = wearing;
			self.pending_since = now;
		}
		if self.pending != self.st.wearing && now.duration_since(self.pending_since) >= STABLE {
			self.st.wearing = self.pending;
			crate::log(&format!("headset {}", describe(s)));
			self.sink_due = Some(now);
		}
	}

	fn poll_battery(&mut self, now: Instant) {
		self.next_battery = now + BATTERY_EVERY;
		let reachable = self.st.status.map(|s| s.on || s.docked).unwrap_or(false);
		if !reachable {
			self.st.battery = None;
			return;
		}
		match self.with_hid(protocol::battery) {
			Ok(b) => {
				self.check_low_battery(b);
				self.st.battery = Some(b);
			}
			Err(e) if e.is_no_headset() => self.st.battery = None,
			Err(e) => crate::log(&format!("battery read failed: {e}")),
		}
	}

	fn check_low_battery(&mut self, b: Battery) {
		if b.charging {
			self.alerted.clear();
			return;
		}
		let cfg = self.cfg();
		for level in LOW_BATTERY {
			if b.percent <= level && !self.alerted.contains(&level) {
				self.alerted.push(level);
				if cfg.low_battery_alerts {
					(self.hooks.notify)("A50 battery low", &format!("Headset battery at {}%", b.percent));
				}
			}
		}
	}

	fn update_vbat(&mut self) {
		let want = self.cfg().publish_battery;
		match (want, self.st.battery) {
			(true, Some(b)) => {
				if let Some(v) = &mut self.vbat {
					if v.update(b.percent, b.charging).and_then(|_| v.service()).is_err() {
						self.vbat = None;
					}
				}
				else if !self.vbat_failed {
					match VirtualBattery::create(b.percent, b.charging) {
						Ok(v) => {
							crate::log("publishing headset battery to UPower");
							self.vbat = Some(v);
						}
						Err(e) => {
							crate::log(&format!("cannot publish battery (/dev/uhid: {e}) — install the udev rules"));
							self.vbat_failed = true;
						}
					}
				}
			}
			_ => self.vbat = None,
		}
		self.st.battery_published = self.vbat.is_some();
	}

	fn apply_audio(&mut self) {
		let cfg = self.cfg();
		let sinks = audio::list_sinks();
		let bluetooth = audio::first_connected(&cfg.bluetooth_devices, &sinks);
		self.st.bluetooth_connected = bluetooth.as_ref().map(|(device, _)| device.label().to_string());
		if !cfg.audio_switching || !self.baselined {
			return;
		}
		let (label, sink) = if self.st.wearing {
			("Astro A50", audio::find_a50(&sinks))
		}
		else if let Some((device, sink)) = bluetooth {
			(device.label(), Some(sink))
		}
		else {
			("Speakers", audio::find_speakers(&sinks))
		};
		self.st.audio_target = Some(label.to_string());
		let Some(sink) = sink else {
			if self.st.audio_sink.is_some() {
				crate::log(&format!("no sink found for {label} — waiting for it to appear"));
			}
			self.st.audio_sink = None;
			return;
		};
		if audio::default_sink().as_deref() != Some(sink.as_str()) {
			crate::log(&format!("switching to {label} → {sink}"));
			if let Err(e) = audio::switch_to(&sink) {
				crate::log(&format!("WARNING: failed to switch to {sink}: {e}"));
			}
		}
		self.st.audio_sink = Some(sink);
	}

	fn call(&mut self, f: CallFn, mutates: bool) -> Result<Value, String> {
		if self.hid.is_none() {
			return Err(DevError::NotFound.to_string());
		}
		let res = self.with_hid(f).map_err(|e| e.to_string());
		if mutates && res.is_ok() {
			self.save_due = Some(Instant::now() + SAVE_DELAY);
		}
		res
	}

	fn publish(&mut self) {
		let mut shared = self.shared.lock().unwrap();
		if *shared != self.st {
			*shared = self.st.clone();
			drop(shared);
			(self.hooks.on_state)(&self.st);
		}
	}
}

fn describe(s: Status) -> String {
	format!(
		"{}{}",
		if s.on { "ON" } else { "OFF" },
		if s.docked { ", DOCKED" } else { "" }
	)
}

/// Whether the udev permissions are in place.
#[derive(Serialize)]
pub struct SystemStatus {
	pub hidraw_ok: bool,
	pub uhid_ok: bool,
	/// Our rules file is installed (it also tags the battery as a headset).
	pub rules_installed: bool,
	pub in_plugdev: bool,
	pub autostart: bool,
}

pub fn system_status() -> SystemStatus {
	let hidraw_ok = device::find_hidraw()
		.map(|p| std::fs::OpenOptions::new().read(true).write(true).open(p).is_ok())
		.unwrap_or(false);
	let in_plugdev = std::process::Command::new("id")
		.arg("-nG")
		.output()
		.map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().any(|g| g == "plugdev"))
		.unwrap_or(false);
	let rules_installed = std::fs::read_to_string("/etc/udev/rules.d/70-a50-control.rules")
		.map(|r| r.contains("SOUND_FORM_FACTOR"))
		.unwrap_or(false);
	SystemStatus {
		hidraw_ok,
		uhid_ok: uhid::accessible(),
		rules_installed,
		in_plugdev,
		autostart: crate::config::autostart_enabled(),
	}
}
