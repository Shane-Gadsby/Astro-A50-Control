mod audio;
mod config;
mod device;
mod protocol;
mod service;
mod uhid;

use config::Config;
use protocol::EqBand;
use serde::Deserialize;
use serde_json::Value;
use service::{CallFn, Msg, State};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::image::Image;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;

const RULES: &str = include_str!("../../packaging/70-a50-control.rules");
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
const TRAY_ICON_CHARGING: &[u8] = include_bytes!("../icons/tray-charging.png");

pub fn log(msg: &str) {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	// Local wall-clock HH:MM:SS without pulling in a time crate.
	let mut tm: libc::tm = unsafe { std::mem::zeroed() };
	let t = now as libc::time_t;
	// SAFETY: localtime_r writes into our tm and reads t.
	unsafe { libc::localtime_r(&t, &mut tm) };
	eprintln!("{:02}:{:02}:{:02} [a50-control] {msg}", tm.tm_hour, tm.tm_min, tm.tm_sec);
}

struct Ctx {
	tx: Mutex<Sender<Msg>>,
	state: Arc<Mutex<State>>,
	config: Arc<Mutex<Config>>,
}

impl Ctx {
	fn call(&self, mutates: bool, f: CallFn) -> Result<Value, String> {
		let (reply, rx) = mpsc::channel();
		self.tx.lock().unwrap().send(Msg::Call { f, mutates, reply }).map_err(|_| "service stopped".to_string())?;
		rx.recv().map_err(|_| "service stopped".to_string())?
	}
}

struct TrayItems {
	tray: TrayIcon,
	battery: MenuItem<tauri::Wry>,
	headset: MenuItem<tauri::Wry>,
	/// Whether the charging icon is showing, so the icon is only swapped on change.
	charging_icon: Mutex<bool>,
}

fn ok<T: serde::Serialize>(v: T) -> device::Result<Value> {
	Ok(serde_json::to_value(v).unwrap_or(Value::Null))
}

#[tauri::command]
fn get_state(ctx: tauri::State<Ctx>) -> State {
	ctx.state.lock().unwrap().clone()
}

#[tauri::command]
fn get_config(ctx: tauri::State<Ctx>) -> Config {
	ctx.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_config(ctx: tauri::State<Ctx>, cfg: Config) -> Result<(), String> {
	config::save(&cfg).map_err(|e| e.to_string())?;
	*ctx.config.lock().unwrap() = cfg;
	let _ = ctx.tx.lock().unwrap().send(Msg::ConfigChanged);
	Ok(())
}

#[tauri::command]
async fn read_settings(ctx: tauri::State<'_, Ctx>) -> Result<Value, String> {
	ctx.call(false, Box::new(|h| ok(protocol::read_settings(h)?)))
}

/// One settings change from the UI.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Change {
	Shutoff { minutes: u8 },
	ActiveEq { preset: u8 },
	EqGains { preset: u8, gains: [i8; 5] },
	EqBand { preset: u8, band: u8, freq: u16, bandwidth: u16 },
	EqName { preset: u8, name: String },
	Slider { id: u8, value: u8 },
	NoiseGate { mode: u8 },
	MicEq { value: u8 },
	DefaultBalance { value: u8 },
	AlertVolume { value: u8 },
}

#[tauri::command]
async fn apply(ctx: tauri::State<'_, Ctx>, change: Change) -> Result<Value, String> {
	ctx.call(
		true,
		Box::new(move |h| {
			match change {
				Change::Shutoff { minutes } => protocol::set_shutoff_minutes(h, minutes)?,
				Change::ActiveEq { preset } => protocol::set_active_eq(h, preset)?,
				Change::EqGains { preset, gains } => protocol::set_eq_gains(h, preset, gains)?,
				Change::EqBand { preset, band, freq, bandwidth } => {
					protocol::set_eq_band(h, preset, band, EqBand { freq, bandwidth })?
				}
				Change::EqName { preset, name } => protocol::set_eq_name(h, preset, &name)?,
				Change::Slider { id, value } => protocol::set_slider(h, id, value)?,
				Change::NoiseGate { mode } => protocol::set_noise_gate(h, mode)?,
				Change::MicEq { value } => protocol::set_mic_eq(h, value)?,
				Change::DefaultBalance { value } => protocol::set_default_balance(h, value)?,
				Change::AlertVolume { value } => protocol::set_alert_volume(h, value)?,
			}
			Ok(Value::Null)
		}),
	)
}

#[tauri::command]
async fn paired_bluetooth_devices() -> Vec<audio::PairedDevice> {
	audio::paired_devices()
}

#[tauri::command]
fn system_status() -> service::SystemStatus {
	service::system_status()
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
	config::set_autostart(enabled).map_err(|e| e.to_string())
}

/// Installs the udev rules with pkexec (graphical password prompt).
#[tauri::command]
async fn install_rules() -> Result<(), String> {
	let tmp = std::env::temp_dir().join("70-a50-control.rules");
	std::fs::write(&tmp, RULES).map_err(|e| e.to_string())?;
	let script = format!(
		"install -m 0644 '{}' /etc/udev/rules.d/70-a50-control.rules && \
		 udevadm control --reload && modprobe uhid; \
		 chgrp plugdev /dev/uhid && chmod 0660 /dev/uhid; \
		 udevadm trigger --subsystem-match=hidraw",
		tmp.display()
	);
	let out = std::process::Command::new("pkexec")
		.args(["sh", "-c", &script])
		.output()
		.map_err(|e| format!("pkexec: {e}"))?;
	let _ = std::fs::remove_file(&tmp);
	if out.status.success() {
		Ok(())
	}
	else {
		Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
	}
}

fn show_window(app: &AppHandle) {
	if let Some(w) = app.get_webview_window("main") {
		let _ = w.show();
		let _ = w.unminimize();
		let _ = w.set_focus();
	}
}

fn update_tray(app: &AppHandle, st: &State) {
	let Some(items) = app.try_state::<TrayItems>() else {
		return;
	};
	let (battery, title) = match st.battery {
		Some(b) => (
			format!("Battery: {}%{}", b.percent, if b.charging { " (charging)" } else { "" }),
			Some(format!("{}%", b.percent)),
		),
		None => ("Battery: —".to_string(), None),
	};
	let headset = match (st.connected, st.status) {
		(false, _) => "Base station not connected".to_string(),
		(true, None) => "Headset: unknown".to_string(),
		(true, Some(s)) => format!(
			"Headset: {}{}",
			if s.on { "on" } else { "off" },
			if s.docked { ", docked" } else { "" }
		),
	};
	// Docked and charging: show a charging icon instead of the percentage label
	// (the exact level stays in the tooltip and menu).
	let charging = st.status.map(|s| s.docked).unwrap_or(false) && st.battery.map(|b| b.charging).unwrap_or(false);
	let title = if charging { None } else { title };
	let mut showing = items.charging_icon.lock().unwrap();
	if *showing != charging {
		let bytes = if charging { TRAY_ICON_CHARGING } else { TRAY_ICON };
		if let Ok(icon) = Image::from_bytes(bytes) {
			let _ = items.tray.set_icon(Some(icon));
			*showing = charging;
		}
	}
	drop(showing);
	let _ = items.battery.set_text(&battery);
	let _ = items.headset.set_text(&headset);
	let _ = items.tray.set_title(title.as_deref());
	let _ = items.tray.set_tooltip(Some(format!("A50 Control — {battery}")));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	let background = std::env::args().any(|a| a == "--background");

	tauri::Builder::default()
		.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
		.plugin(tauri_plugin_notification::init())
		.invoke_handler(tauri::generate_handler![
			get_state,
			get_config,
			set_config,
			read_settings,
			apply,
			paired_bluetooth_devices,
			system_status,
			set_autostart,
			install_rules
		])
		.setup(move |app| {
			let handle = app.handle().clone();

			let battery = MenuItem::with_id(app, "battery", "Battery: —", false, None::<&str>)?;
			let headset = MenuItem::with_id(app, "headset", "Headset: unknown", false, None::<&str>)?;
			let open = MenuItem::with_id(app, "open", "Open A50 Control", true, None::<&str>)?;
			let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
			let menu = Menu::with_items(
				app,
				&[&battery, &headset, &PredefinedMenuItem::separator(app)?, &open, &quit],
			)?;
			let tray = TrayIconBuilder::with_id("a50")
				.icon(Image::from_bytes(TRAY_ICON)?)
				.menu(&menu)
				.tooltip("A50 Control")
				.on_menu_event(|app, event| match event.id.as_ref() {
					"open" => show_window(app),
					"quit" => app.exit(0),
					_ => {}
				})
				.build(app)?;
			app.manage(TrayItems { tray, battery, headset, charging_icon: Mutex::new(false) });

			let state = Arc::new(Mutex::new(State::default()));
			let config = Arc::new(Mutex::new(config::load()));
			let emit_handle = handle.clone();
			let notify_handle = handle.clone();
			let tx = service::spawn(
				state.clone(),
				config.clone(),
				service::Hooks {
					on_state: Box::new(move |st| {
						let _ = emit_handle.emit("state", st);
						update_tray(&emit_handle, st);
					}),
					notify: Box::new(move |title, body| {
						let _ = notify_handle.notification().builder().title(title).body(body).show();
					}),
				},
			);
			app.manage(Ctx { tx: Mutex::new(tx), state, config });

			if let Err(e) = config::refresh_autostart() {
				log(&format!("could not update autostart entry: {e}"));
			}
			if !background {
				show_window(&handle);
			}
			log(&format!("started{}", if background { " in background" } else { "" }));
			Ok(())
		})
		.on_window_event(|window, event| {
			// Closing the window keeps the service running in the tray.
			if let WindowEvent::CloseRequested { api, .. } = event {
				api.prevent_close();
				let _ = window.hide();
			}
		})
		.run(tauri::generate_context!())
		.expect("error while running A50 Control");
}
