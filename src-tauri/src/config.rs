//! App preferences (not headset settings, which live on the device) and the
//! XDG autostart entry.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// A Bluetooth headset the user chose for audio switching.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct BluetoothDevice {
	pub address: String,
	/// Name from the system Bluetooth panel, kept for when it isn't available.
	pub name: String,
	/// The user's label for this app only (empty to use `name`).
	pub nickname: String,
}

impl BluetoothDevice {
	pub fn label(&self) -> &str {
		[&self.nickname, &self.name].into_iter().find(|s| !s.is_empty()).unwrap_or(&self.address)
	}
}

/// The output the user chose as their speakers. Identified by device and
/// profile family, because the sink name changes with the profile.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SpeakerChoice {
	pub vendor_id: String,
	pub product_id: String,
	pub profile: String,
	pub sink_name: String,
	pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
	/// Switch the default sink between A50 / Bluetooth / speakers automatically.
	pub audio_switching: bool,
	/// Publish the headset battery to UPower via /dev/uhid.
	pub publish_battery: bool,
	/// Desktop notification at 15% and 5% while discharging.
	pub low_battery_alerts: bool,
	/// Headsets to switch to when connected, highest priority first.
	pub bluetooth_devices: Vec<BluetoothDevice>,
	/// Fallback output (None to detect it automatically).
	pub speakers: Option<SpeakerChoice>,
	/// The first-start speaker question has been answered.
	pub speakers_prompted: bool,
}

impl Default for Config {
	fn default() -> Self {
		Config {
			audio_switching: true,
			publish_battery: true,
			low_battery_alerts: true,
			bluetooth_devices: Vec::new(),
			speakers: None,
			speakers_prompted: false,
		}
	}
}

fn config_dir() -> PathBuf {
	std::env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
		.unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn config_path() -> PathBuf {
	config_dir().join("a50-control").join("config.json")
}

pub fn load() -> Config {
	fs::read(config_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
	let path = config_path();
	fs::create_dir_all(path.parent().unwrap())?;
	fs::write(path, serde_json::to_vec_pretty(cfg)?)
}

fn autostart_path() -> PathBuf {
	config_dir().join("autostart").join("a50-control.desktop")
}

pub fn autostart_enabled() -> bool {
	autostart_path().exists()
}

/// If autostart is on and we're running from an AppImage whose path differs
/// from the one in the entry (moved, renamed, or enabled from a dev build),
/// rewrite the entry so login start keeps working.
pub fn refresh_autostart() -> std::io::Result<()> {
	let Ok(appimage) = std::env::var("APPIMAGE") else {
		return Ok(());
	};
	match fs::read_to_string(autostart_path()) {
		Ok(entry) if !entry.contains(&format!("Exec=\"{appimage}\"")) => set_autostart(true),
		_ => Ok(()),
	}
}

/// Writes or removes the autostart entry. Inside an AppImage the stable path
/// is $APPIMAGE (the mounted executable path changes every run).
pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
	let path = autostart_path();
	if !enabled {
		return match fs::remove_file(&path) {
			Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
			_ => Ok(()),
		};
	}
	let exe = std::env::var("APPIMAGE")
		.ok()
		.or_else(|| std::env::current_exe().ok().map(|p| p.display().to_string()))
		.unwrap_or_else(|| "a50-control".into());
	fs::create_dir_all(path.parent().unwrap())?;
	fs::write(
		path,
		format!(
			"[Desktop Entry]\nType=Application\nName=A50 Control\n\
			 Comment=Astro A50 battery, settings and audio switching\n\
			 Exec=\"{exe}\" --background\nIcon=a50-control\nTerminal=false\n\
			 X-GNOME-Autostart-enabled=true\n"
		),
	)
}
