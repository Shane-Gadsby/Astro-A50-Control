//! Typed commands for the A50 Gen 4 base station.
//!
//! Command numbers and encodings come from the eh-fifty project and from
//! disassembling Astro Command Center (auto-shutoff: read 0x78 / adjust 0x74,
//! minutes, UI choices 10–30), and were checked against this base on
//! 2026-10-24. Setters change the active value only; `save` writes the active
//! values to flash so they survive a power cycle.

use crate::device::{DevError, Expect, Hid, Result, SLOW_TIMEOUT};
use serde::{Deserialize, Serialize};

const GET_HEADSET_STATUS: u8 = 0x54;
const SAVE_VALUES: u8 = 0x61;
const SET_SLIDER: u8 = 0x62;
const SET_EQ_GAIN: u8 = 0x63;
const SET_NOISE_GATE: u8 = 0x64;
const SET_ACTIVE_EQ: u8 = 0x67;
const GET_SLIDER: u8 = 0x68;
const GET_EQ_GAIN: u8 = 0x69;
const GET_NOISE_GATE: u8 = 0x6a;
const GET_ACTIVE_EQ: u8 = 0x6c;
const SET_EQ_NAME: u8 = 0x6d;
const GET_EQ_NAME: u8 = 0x6e;
const SET_EQ_BAND: u8 = 0x6f;
const GET_EQ_BAND: u8 = 0x70;
const SET_MIC_EQ: u8 = 0x71;
const GET_BALANCE: u8 = 0x72;
const SET_DEFAULT_BALANCE: u8 = 0x73;
const SET_SHUTOFF: u8 = 0x74;
const SET_ALERT_VOLUME: u8 = 0x76;
const GET_DEFAULT_BALANCE: u8 = 0x77;
const GET_SHUTOFF: u8 = 0x78;
const GET_ALERT_VOLUME: u8 = 0x7a;
const GET_MIC_EQ: u8 = 0x7b;
const GET_BATTERY: u8 = 0x7c;

const DB_OFFSET: i16 = 12;
pub const EQ_PRESETS: [u8; 3] = [1, 2, 3];
pub const EQ_MIN_GAIN: i8 = -7;
pub const EQ_MAX_GAIN: i8 = 7;
pub const EQ_MIN_FREQ: u16 = 80;
pub const EQ_MAX_FREQ: u16 = 15_000;
/// Bandwidth is Q × 4096; bands 1 and 5 are shelves with bandwidth 0.
pub const EQ_MIN_BW: u16 = 409;
pub const EQ_MAX_BW: u16 = 12_288;
pub const EQ_NAME_MAX: usize = 16;
/// The only auto-shutoff values Astro Command Center offers, in minutes.
pub const SHUTOFF_CHOICES: [u8; 5] = [10, 15, 20, 25, 30];

/// Slider ids: stream port mix (0–3), mic level (4), sidetone (5).
pub const SLIDER_COUNT: u8 = 6;

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
	pub docked: bool,
	pub on: bool,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
	pub percent: u8,
	pub charging: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct EqBand {
	pub freq: u16,
	pub bandwidth: u16,
}

#[derive(Serialize, Clone, Debug)]
pub struct EqPreset {
	pub id: u8,
	pub name: String,
	pub gains: [i8; 5],
	pub bands: [EqBand; 5],
}

#[derive(Serialize, Clone, Debug)]
pub struct Settings {
	pub shutoff_minutes: u8,
	pub active_eq: u8,
	pub presets: Vec<EqPreset>,
	pub sliders: [u8; SLIDER_COUNT as usize],
	pub noise_gate: u8,
	pub mic_eq: u8,
	pub balance: u8,
	pub default_balance: u8,
	pub alert_volume: u8,
}

fn bad(msg: impl Into<String>) -> DevError {
	DevError::BadReply(msg.into())
}

fn check_eq(preset: u8) -> Result<()> {
	if EQ_PRESETS.contains(&preset) {
		Ok(())
	}
	else {
		Err(bad(format!("no EQ preset {preset}")))
	}
}

pub fn status(h: &mut Hid) -> Result<Status> {
	let d = h.request(GET_HEADSET_STATUS, &[], Expect::Len(1))?;
	if d[0] & !0x03 != 0 {
		return Err(bad(format!("status {:#04x}", d[0])));
	}
	Ok(Status { docked: d[0] & 0x01 != 0, on: d[0] & 0x02 != 0 })
}

pub fn battery(h: &mut Hid) -> Result<Battery> {
	let d = h.request(GET_BATTERY, &[], Expect::Len(1))?;
	let percent = d[0] & 0x7f;
	if percent > 100 {
		return Err(bad(format!("battery {:#04x}", d[0])));
	}
	Ok(Battery { percent, charging: d[0] & 0x80 != 0 })
}

pub fn shutoff_minutes(h: &mut Hid) -> Result<u8> {
	Ok(h.request(GET_SHUTOFF, &[], Expect::Len(1))?[0])
}

pub fn set_shutoff_minutes(h: &mut Hid, minutes: u8) -> Result<()> {
	if !SHUTOFF_CHOICES.contains(&minutes) {
		return Err(bad(format!("auto-shutoff must be one of {SHUTOFF_CHOICES:?} minutes")));
	}
	h.request(SET_SHUTOFF, &[minutes], Expect::Echo).map(drop)
}

pub fn active_eq(h: &mut Hid) -> Result<u8> {
	let d = h.request(GET_ACTIVE_EQ, &[], Expect::Len(1))?;
	check_eq(d[0])?;
	Ok(d[0])
}

pub fn set_active_eq(h: &mut Hid, preset: u8) -> Result<()> {
	check_eq(preset)?;
	h.request(SET_ACTIVE_EQ, &[preset], Expect::Echo).map(drop)
}

pub fn eq_name(h: &mut Hid, preset: u8) -> Result<String> {
	check_eq(preset)?;
	let d = h.request(GET_EQ_NAME, &[preset, 0], Expect::Echo)?;
	let name: Vec<u8> = d.iter().skip(2).take_while(|&&c| c != 0).copied().collect();
	Ok(String::from_utf8_lossy(&name).into_owned())
}

pub fn set_eq_name(h: &mut Hid, preset: u8, name: &str) -> Result<()> {
	check_eq(preset)?;
	let name = name.trim();
	if name.is_empty() || name.len() > EQ_NAME_MAX || !name.is_ascii() {
		return Err(bad(format!("preset name must be 1–{EQ_NAME_MAX} ASCII characters")));
	}
	let mut payload = vec![preset, name.len() as u8 + 1];
	payload.extend_from_slice(name.as_bytes());
	payload.push(0);
	h.request(SET_EQ_NAME, &payload, Expect::Echo).map(drop)
}

pub fn eq_gains(h: &mut Hid, preset: u8) -> Result<[i8; 5]> {
	check_eq(preset)?;
	let d = h.request(GET_EQ_GAIN, &[preset], Expect::Echo)?;
	if d.len() < 7 {
		return Err(bad("short EQ gain reply"));
	}
	let mut gains = [0i8; 5];
	for (g, raw) in gains.iter_mut().zip(&d[2..7]) {
		*g = (*raw as i16 - DB_OFFSET) as i8;
	}
	Ok(gains)
}

pub fn set_eq_gains(h: &mut Hid, preset: u8, gains: [i8; 5]) -> Result<()> {
	check_eq(preset)?;
	if gains.iter().any(|g| !(EQ_MIN_GAIN..=EQ_MAX_GAIN).contains(g)) {
		return Err(bad(format!("EQ gains must be {EQ_MIN_GAIN}..{EQ_MAX_GAIN} dB")));
	}
	let mut payload = vec![preset];
	payload.extend(gains.iter().map(|&g| (g as i16 + DB_OFFSET) as u8));
	h.request(SET_EQ_GAIN, &payload, Expect::Echo).map(drop)
}

pub fn eq_band(h: &mut Hid, preset: u8, band: u8) -> Result<EqBand> {
	check_eq(preset)?;
	let d = h.request(GET_EQ_BAND, &[preset, band], Expect::Echo)?;
	if d.len() < 9 {
		return Err(bad("short EQ band reply"));
	}
	Ok(EqBand {
		bandwidth: u16::from_le_bytes([d[3], d[4]]),
		freq: u16::from_le_bytes([d[7], d[8]]),
	})
}

pub fn set_eq_band(h: &mut Hid, preset: u8, band: u8, eq: EqBand) -> Result<()> {
	check_eq(preset)?;
	if !(1..=5).contains(&band) {
		return Err(bad(format!("no EQ band {band}")));
	}
	if !(EQ_MIN_FREQ..=EQ_MAX_FREQ).contains(&eq.freq) {
		return Err(bad(format!("frequency must be {EQ_MIN_FREQ}–{EQ_MAX_FREQ} Hz")));
	}
	let shelf = band == 1 || band == 5;
	let bandwidth = if shelf { 0 } else { eq.bandwidth.clamp(EQ_MIN_BW, EQ_MAX_BW) };
	let [bl, bh] = bandwidth.to_le_bytes();
	let [fl, fh] = eq.freq.to_le_bytes();
	h.request(SET_EQ_BAND, &[preset, band, bl, bh, fl, fh], Expect::Echo).map(drop)
}

pub fn slider(h: &mut Hid, id: u8) -> Result<u8> {
	let d = h.request(GET_SLIDER, &[id], Expect::Echo)?;
	d.get(2).copied().ok_or_else(|| bad("short slider reply"))
}

pub fn set_slider(h: &mut Hid, id: u8, value: u8) -> Result<()> {
	if id >= SLIDER_COUNT || value > 100 {
		return Err(bad("slider out of range"));
	}
	h.request(SET_SLIDER, &[id, value], Expect::Echo).map(drop)
}

pub fn noise_gate(h: &mut Hid) -> Result<u8> {
	let d = h.request(GET_NOISE_GATE, &[], Expect::Echo)?;
	d.get(1).copied().ok_or_else(|| bad("short noise gate reply"))
}

pub fn set_noise_gate(h: &mut Hid, mode: u8) -> Result<()> {
	if mode > 3 {
		return Err(bad("noise gate mode must be 0–3"));
	}
	// This setter answers with the mode itself rather than an echo.
	let d = h.request(SET_NOISE_GATE, &[mode], Expect::Len(1))?;
	if d[0] != mode {
		return Err(bad("noise gate not applied"));
	}
	Ok(())
}

pub fn mic_eq(h: &mut Hid) -> Result<u8> {
	Ok(h.request(GET_MIC_EQ, &[0], Expect::Len(1))?[0])
}

pub fn set_mic_eq(h: &mut Hid, value: u8) -> Result<()> {
	if value > 2 {
		return Err(bad("mic EQ must be 0–2"));
	}
	h.request(SET_MIC_EQ, &[value], Expect::Echo).map(drop)
}

pub fn balance(h: &mut Hid) -> Result<u8> {
	Ok(h.request(GET_BALANCE, &[], Expect::Len(1))?[0])
}

pub fn default_balance(h: &mut Hid) -> Result<u8> {
	Ok(h.request(GET_DEFAULT_BALANCE, &[0], Expect::Len(1))?[0])
}

pub fn set_default_balance(h: &mut Hid, value: u8) -> Result<()> {
	h.request(SET_DEFAULT_BALANCE, &[value], Expect::Echo).map(drop)
}

pub fn alert_volume(h: &mut Hid) -> Result<u8> {
	Ok(h.request(GET_ALERT_VOLUME, &[0], Expect::Len(1))?[0])
}

pub fn set_alert_volume(h: &mut Hid, value: u8) -> Result<()> {
	if value > 100 {
		return Err(bad("alert volume must be 0–100"));
	}
	h.request(SET_ALERT_VOLUME, &[value], Expect::Echo).map(drop)
}

pub fn save(h: &mut Hid) -> Result<()> {
	let d = h.request_with_timeout(SAVE_VALUES, &[], Expect::Echo, SLOW_TIMEOUT)?;
	if d.get(1) != Some(&0) {
		return Err(bad("save failed"));
	}
	Ok(())
}

pub fn read_settings(h: &mut Hid) -> Result<Settings> {
	let mut presets = Vec::new();
	for id in EQ_PRESETS {
		let mut bands = [EqBand { freq: 0, bandwidth: 0 }; 5];
		for (i, b) in bands.iter_mut().enumerate() {
			*b = eq_band(h, id, i as u8 + 1)?;
		}
		presets.push(EqPreset { id, name: eq_name(h, id)?, gains: eq_gains(h, id)?, bands });
	}
	let mut sliders = [0u8; SLIDER_COUNT as usize];
	for (i, s) in sliders.iter_mut().enumerate() {
		*s = slider(h, i as u8)?;
	}
	Ok(Settings {
		shutoff_minutes: shutoff_minutes(h)?,
		active_eq: active_eq(h)?,
		presets,
		sliders,
		noise_gate: noise_gate(h)?,
		mic_eq: mic_eq(h)?,
		balance: balance(h)?,
		default_balance: default_balance(h)?,
		alert_volume: alert_volume(h)?,
	})
}
