//! Default-sink switching, ported from the a50-watcher Node daemon.
//!
//! Priority: A50 (on and off the dock) > connected Bluetooth headsets, in the
//! user's order > speakers (chosen by the user, or detected). Sinks are identified by stable device properties from
//! `pactl -f json` (USB vendor/product, Bluetooth address, card profile), never by
//! node name: PipeWire rewrites names whenever a device is re-created (e.g.
//! `...A50-00.stereo-chat` → `...A50-00.stereo-chat.2`).
//!
//! Decisions compare against PipeWire's *actual* default sink, so if a sink is
//! re-created and WirePlumber moves the default, the next sink event puts it back.

use crate::config::{BluetoothDevice, SpeakerChoice};
use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;

pub struct Sink {
	pub name: String,
	description: String,
	props: Value,
}

impl Sink {
	fn prop(&self, key: &str) -> &str {
		self.props.get(key).and_then(Value::as_str).unwrap_or("")
	}
}

fn is_a50_base(sink: &Sink) -> bool {
	sink.prop("device.vendor.id") == "0x9886" && sink.prop("device.product.id") == "0x002c"
}

/// Chat profile only; the base also exposes a stereo-game sink.
fn is_a50(sink: &Sink) -> bool {
	is_a50_base(sink) && sink.prop("device.profile.name").contains("chat")
}

fn is_any_bluetooth(sink: &Sink) -> bool {
	!sink.prop("api.bluez5.address").is_empty() || sink.name.starts_with("bluez_output.")
}

/// "analog-surround-21" → "analog": a card's stereo and surround profiles are the
/// same physical output.
fn profile_family(profile: &str) -> &str {
	profile.split(['-', '+', ':']).next().unwrap_or(profile)
}

fn is_speaker_candidate(sink: &Sink) -> bool {
	!is_a50_base(sink) && !is_any_bluetooth(sink)
}

fn is_choice(sink: &Sink, choice: &SpeakerChoice) -> bool {
	if choice.vendor_id.is_empty() {
		// Virtual sinks have no device ids; their name is all there is.
		return sink.name == choice.sink_name;
	}
	sink.prop("device.vendor.id") == choice.vendor_id
		&& sink.prop("device.product.id") == choice.product_id
		&& profile_family(sink.prop("device.profile.name")) == profile_family(&choice.profile)
}

fn speaker_score(sink: &Sink, default: Option<&str>) -> u32 {
	let family = profile_family(sink.prop("device.profile.name"));
	let kind = match family {
		"analog" if sink.prop("device.bus") == "pci" => 4,
		"analog" => 3,
		"hdmi" | "iec958" => 1,
		_ => 2,
	};
	// An output already in use is the strongest hint: the user picked it.
	kind + if default == Some(sink.name.as_str()) { 10 } else { 0 }
}

/// Best guess at the speakers. Earlier sinks win ties.
fn detect_speakers<'a>(sinks: &'a [Sink], default: Option<&str>) -> Option<&'a Sink> {
	sinks.iter().filter(|s| is_speaker_candidate(s)).rev().max_by_key(|s| speaker_score(s, default))
}

fn is_bluetooth(sink: &Sink, address: &str) -> bool {
	!address.is_empty()
		&& (sink.prop("api.bluez5.address").eq_ignore_ascii_case(address)
			|| sink.name.starts_with(&format!("bluez_output.{}.", address.to_ascii_uppercase().replace(':', "_"))))
}

pub fn find_a50(sinks: &[Sink]) -> Option<String> {
	sinks.iter().find(|s| is_a50(s)).map(|s| s.name.clone())
}

/// The user's chosen speakers if present, otherwise the best guess, so unplugging
/// a chosen output doesn't leave audio stuck on a device that's gone.
pub fn find_speakers(sinks: &[Sink], choice: Option<&SpeakerChoice>, default: Option<&str>) -> Option<String> {
	choice
		.and_then(|c| sinks.iter().find(|s| is_choice(s, c)))
		.or_else(|| detect_speakers(sinks, default))
		.map(|s| s.name.clone())
}

/// What the UI needs to offer a choice of speakers.
#[derive(Serialize)]
pub struct SpeakerOptions {
	pub outputs: Vec<SpeakerChoice>,
	/// Sink name of the best guess.
	pub detected: Option<String>,
	/// Sink name matching the saved choice, if it's present.
	pub selected: Option<String>,
}

pub fn speaker_options(choice: Option<&SpeakerChoice>) -> SpeakerOptions {
	let sinks = list_sinks();
	let default = default_sink();
	SpeakerOptions {
		outputs: sinks
			.iter()
			.filter(|s| is_speaker_candidate(s))
			.map(|s| SpeakerChoice {
				vendor_id: s.prop("device.vendor.id").to_string(),
				product_id: s.prop("device.product.id").to_string(),
				profile: s.prop("device.profile.name").to_string(),
				sink_name: s.name.clone(),
				label: if s.description.is_empty() { s.name.clone() } else { s.description.clone() },
			})
			.collect(),
		detected: detect_speakers(&sinks, default.as_deref()).map(|s| s.name.clone()),
		selected: choice.and_then(|c| sinks.iter().find(|s| is_choice(s, c))).map(|s| s.name.clone()),
	}
}

/// The highest-priority device in `devices` that currently has a sink.
pub fn first_connected<'a>(devices: &'a [BluetoothDevice], sinks: &[Sink]) -> Option<(&'a BluetoothDevice, String)> {
	devices
		.iter()
		.find_map(|d| sinks.iter().find(|s| is_bluetooth(s, &d.address)).map(|s| (d, s.name.clone())))
}

/// A paired audio device, as the system Bluetooth panel lists it.
#[derive(Serialize, Debug, PartialEq)]
pub struct PairedDevice {
	pub address: String,
	pub name: String,
	pub connected: bool,
}

const AUDIO_SINK_UUID: &str = "0000110b-";

/// Paired audio devices from BlueZ. Empty if Bluetooth isn't available.
pub fn paired_devices() -> Vec<PairedDevice> {
	let out = Command::new("busctl")
		.args(["--system", "--json=short", "call", "org.bluez", "/"])
		.args(["org.freedesktop.DBus.ObjectManager", "GetManagedObjects"])
		.output();
	match out {
		Ok(o) if o.status.success() => {
			serde_json::from_slice(&o.stdout).map(|v| parse_paired(&v)).unwrap_or_default()
		}
		_ => Vec::new(),
	}
}

fn parse_paired(reply: &Value) -> Vec<PairedDevice> {
	let Some(objects) = reply.pointer("/data/0").and_then(Value::as_object) else {
		return Vec::new();
	};
	let mut found: Vec<PairedDevice> = objects
		.values()
		.filter_map(|object| {
			let dev = object.get("org.bluez.Device1")?;
			let prop = |key: &str| dev.get(key).and_then(|p| p.get("data"));
			let text = |key: &str| prop(key).and_then(Value::as_str);
			let flag = |key: &str| prop(key).and_then(Value::as_bool).unwrap_or(false);
			// UUIDs can be missing until first connection, so the icon counts too.
			let has_sink = prop("UUIDs")
				.and_then(Value::as_array)
				.is_some_and(|ids| ids.iter().filter_map(Value::as_str).any(|id| id.starts_with(AUDIO_SINK_UUID)));
			if !flag("Paired") || !(has_sink || text("Icon").is_some_and(|i| i.starts_with("audio-"))) {
				return None;
			}
			let address = text("Address")?.to_string();
			Some(PairedDevice {
				name: text("Alias").unwrap_or(&address).to_string(),
				connected: flag("Connected"),
				address,
			})
		})
		.collect();
	found.sort_by_key(|d| d.name.to_lowercase());
	found
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
			let description = s.get("description").and_then(Value::as_str).unwrap_or("").to_string();
			let props = s.get("properties").cloned().unwrap_or(Value::Null);
			Some(Sink { name, description, props })
		})
		.collect()
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

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn sink(name: &str, props: Value) -> Sink {
		Sink { name: name.into(), description: String::new(), props }
	}

	fn output(name: &str, bus: &str, vendor: &str, product: &str, profile: &str) -> Sink {
		sink(
			name,
			json!({
				"device.bus": bus,
				"device.vendor.id": vendor,
				"device.product.id": product,
				"device.profile.name": profile,
			}),
		)
	}

	fn a50() -> Sink {
		output("a50.chat", "usb", "0x9886", "0x002c", "stereo-chat")
	}

	fn onboard() -> Sink {
		output("onboard.analog-stereo", "pci", "0x1022", "0x15e3", "analog-stereo")
	}

	fn hdmi() -> Sink {
		output("gpu.hdmi", "pci", "0x10de", "0x228b", "hdmi-stereo")
	}

	fn usb_dac() -> Sink {
		output("dac.analog-stereo", "usb", "0x1234", "0x0001", "analog-stereo")
	}

	fn device(address: &str, nickname: &str) -> BluetoothDevice {
		BluetoothDevice { address: address.into(), name: "Headset".into(), nickname: nickname.into() }
	}

	fn bt_sink(address: &str) -> Sink {
		sink(&format!("bluez_output.{}.1", address.replace(':', "_")), json!({ "api.bluez5.address": address }))
	}

	const FIRST: &str = "AA:AA:AA:AA:AA:01";
	const SECOND: &str = "AA:AA:AA:AA:AA:02";

	#[test]
	fn list_order_decides_not_sink_order() {
		let devices = [device(FIRST, ""), device(SECOND, "")];
		let sinks = [bt_sink(SECOND), bt_sink(FIRST)];
		assert_eq!(first_connected(&devices, &sinks).unwrap().0.address, FIRST);
	}

	#[test]
	fn falls_through_to_next_connected_device() {
		let devices = [device(FIRST, ""), device(SECOND, "")];
		let (found, sink) = first_connected(&devices, &[bt_sink(SECOND)]).unwrap();
		assert_eq!(found.address, SECOND);
		assert_eq!(sink, "bluez_output.AA_AA_AA_AA_AA_02.1");
	}

	#[test]
	fn unlisted_and_blank_addresses_never_match() {
		// A sink with no Bluetooth address must not match a blank config entry.
		let usb = sink("alsa_output.usb", json!({}));
		assert!(first_connected(&[device("", "")], &[usb]).is_none());
		assert!(first_connected(&[device(FIRST, "")], &[bt_sink(SECOND)]).is_none());
	}

	#[test]
	fn matches_lowercase_address_by_sink_name() {
		assert!(is_bluetooth(&sink("bluez_output.AA_AA_AA_AA_AA_01.1", json!({})), "aa:aa:aa:aa:aa:01"));
	}

	#[test]
	fn nickname_overrides_name_and_blank_falls_back() {
		assert_eq!(device(FIRST, "Gym buds").label(), "Gym buds");
		assert_eq!(device(FIRST, "").label(), "Headset");
		let unnamed = BluetoothDevice { address: FIRST.into(), ..Default::default() };
		assert_eq!(unnamed.label(), FIRST);
	}

	#[test]
	fn keeps_only_paired_audio_devices() {
		let dev = |alias: &str, paired: bool, icon: &str, uuids: Value| {
			json!({ "org.bluez.Device1": {
				"Address": { "type": "s", "data": format!("00:00:00:00:00:{alias}") },
				"Alias": { "type": "s", "data": alias },
				"Paired": { "type": "b", "data": paired },
				"Connected": { "type": "b", "data": alias == "AB" },
				"Icon": { "type": "s", "data": icon },
				"UUIDs": { "type": "as", "data": uuids },
			}})
		};
		let reply = json!({ "type": "a{oa{sa{sv}}}", "data": [{
			"/org/bluez": { "org.bluez.AgentManager1": {} },
			"/org/bluez/hci0/dev_1": dev("BC", true, "audio-headphones", json!([])),
			"/org/bluez/hci0/dev_2": dev("AB", true, "", json!(["0000110b-0000-1000-8000-00805f9b34fb"])),
			"/org/bluez/hci0/dev_3": dev("KB", true, "input-keyboard", json!(["00001124-0000-1000-8000-00805f9b34fb"])),
			"/org/bluez/hci0/dev_4": dev("NP", false, "audio-headset", json!([])),
		}]});
		let found = parse_paired(&reply);
		assert_eq!(found.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["AB", "BC"]);
		assert!(found[0].connected && !found[1].connected);
	}

	#[test]
	fn detection_prefers_onboard_analog_over_hdmi_and_usb() {
		let sinks = [hdmi(), usb_dac(), onboard()];
		assert_eq!(find_speakers(&sinks, None, None).unwrap(), "onboard.analog-stereo");
	}

	#[test]
	fn detection_prefers_the_current_default() {
		let sinks = [onboard(), hdmi()];
		assert_eq!(find_speakers(&sinks, None, Some("gpu.hdmi")).unwrap(), "gpu.hdmi");
	}

	#[test]
	fn detection_never_picks_the_headset_or_bluetooth() {
		let sinks = [a50(), bt_sink(FIRST)];
		assert!(find_speakers(&sinks, None, Some("a50.chat")).is_none());
		let sinks = [a50(), bt_sink(FIRST), hdmi()];
		assert_eq!(find_speakers(&sinks, None, Some("a50.chat")).unwrap(), "gpu.hdmi");
	}

	#[test]
	fn choice_beats_detection_and_survives_profile_change() {
		let choice = SpeakerChoice {
			vendor_id: "0x1022".into(),
			product_id: "0x15e3".into(),
			profile: "analog-surround-21".into(),
			..Default::default()
		};
		let sinks = [hdmi(), onboard()];
		assert_eq!(find_speakers(&sinks, Some(&choice), Some("gpu.hdmi")).unwrap(), "onboard.analog-stereo");
	}

	#[test]
	fn missing_choice_falls_back_to_detection() {
		let choice = SpeakerChoice { vendor_id: "0x1234".into(), product_id: "0x0001".into(), profile: "analog-stereo".into(), ..Default::default() };
		assert_eq!(find_speakers(&[onboard()], Some(&choice), None).unwrap(), "onboard.analog-stereo");
	}

	#[test]
	fn virtual_sink_choice_matches_by_name() {
		let virtual_sink = sink("easyeffects_sink", json!({}));
		let choice = SpeakerChoice { sink_name: "easyeffects_sink".into(), ..Default::default() };
		assert_eq!(find_speakers(&[onboard(), virtual_sink], Some(&choice), None).unwrap(), "easyeffects_sink");
	}

	#[test]
	fn unexpected_bluez_reply_is_empty() {
		assert!(parse_paired(&json!({})).is_empty());
	}
}
