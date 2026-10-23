//! Default-sink switching, ported from the a50-watcher Node daemon.
//!
//! Priority: A50 (on and off the dock) > Sony WH-CH720N (connected) > speakers.
//! Sinks are identified by stable device properties from `pactl -f json`
//! (USB vendor/product, Bluetooth address, card profile), never by node name:
//! PipeWire rewrites names whenever a device is re-created (e.g.
//! `...A50-00.stereo-chat` → `...A50-00.stereo-chat.2`).
//!
//! Decisions compare against PipeWire's *actual* default sink, so if a sink is
//! re-created and WirePlumber moves the default, the next sink event puts it back.

use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Target {
	A50,
	Sony,
	Speakers,
}

impl Target {
	pub fn label(self) -> &'static str {
		match self {
			Target::A50 => "Astro A50",
			Target::Sony => "Sony WH-CH720N",
			Target::Speakers => "Speakers",
		}
	}
}

pub struct Sink {
	pub name: String,
	props: Value,
}

impl Sink {
	fn prop(&self, key: &str) -> &str {
		self.props.get(key).and_then(Value::as_str).unwrap_or("")
	}
}

fn matches(target: Target, sink: &Sink, sony_address: &str) -> bool {
	match target {
		// Chat profile only; the base also exposes a stereo-game sink.
		Target::A50 => {
			sink.prop("device.vendor.id") == "0x9886"
				&& sink.prop("device.product.id") == "0x002c"
				&& sink.prop("device.profile.name").contains("chat")
		}
		Target::Sony => {
			let addr = sony_address.to_ascii_uppercase();
			!addr.is_empty()
				&& (sink.prop("api.bluez5.address").eq_ignore_ascii_case(&addr)
					|| sink.name.starts_with(&format!("bluez_output.{}.", addr.replace(':', "_"))))
		}
		// Onboard Family 17h/19h HD Audio, analog (line-out) profiles only.
		Target::Speakers => {
			sink.prop("device.vendor.id") == "0x1022"
				&& sink.prop("device.product.id") == "0x15e3"
				&& sink.prop("device.profile.name").starts_with("analog-")
		}
	}
}

pub fn list_sinks() -> Vec<Sink> {
	let Ok(out) = Command::new("pactl").args(["-f", "json", "list", "sinks"]).output() else {
		return Vec::new();
	};
	if !out.status.success() {
		return Vec::new();
	}
	let Ok(Value::Array(sinks)) = serde_json::from_slice::<Value>(&out.stdout) else {
		return Vec::new();
	};
	sinks
		.into_iter()
		.filter_map(|s| {
			let name = s.get("name")?.as_str()?.to_string();
			let props = s.get("properties").cloned().unwrap_or(Value::Null);
			Some(Sink { name, props })
		})
		.collect()
}

pub fn resolve(target: Target, sony_address: &str) -> Option<String> {
	list_sinks().into_iter().find(|s| matches(target, s, sony_address)).map(|s| s.name)
}

pub fn default_sink() -> Option<String> {
	let out = Command::new("pactl").arg("get-default-sink").output().ok()?;
	out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Makes `sink` the default and moves every playing stream onto it.
pub fn switch_to(sink: &str) -> Result<(), String> {
	let run = |args: &[&str]| -> Result<(), String> {
		let out = Command::new("pactl").args(args).output().map_err(|e| e.to_string())?;
		if out.status.success() {
			Ok(())
		}
		else {
			Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
		}
	};
	run(&["set-default-sink", sink])?;
	if let Ok(out) = Command::new("pactl").args(["list", "short", "sink-inputs"]).output() {
		for id in String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.split_whitespace().next()) {
			// A stream may vanish between listing and moving; that's fine.
			let _ = run(&["move-sink-input", id, sink]);
		}
	}
	Ok(())
}

/// Spawns `pactl subscribe` and sends `()` whenever a sink appears/disappears
/// or a card changes. Plain sink 'change' events (volume, our own stream
/// moves) are ignored. Restarts itself if pactl exits.
pub fn spawn_watcher(notify: Sender<()>) {
	std::thread::spawn(move || loop {
		let child = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
		if let Ok(mut child) = child {
			if let Some(out) = child.stdout.take() {
				for line in BufReader::new(out).lines().map_while(Result::ok) {
					let sink_added_removed =
						line.contains("'new' on sink #") || line.contains("'remove' on sink #");
					if (sink_added_removed || line.contains("on card")) && notify.send(()).is_err() {
						return;
					}
				}
			}
			let _ = child.wait();
		}
		crate::log("pactl subscribe exited — restarting in 2s");
		std::thread::sleep(Duration::from_secs(2));
	});
}
