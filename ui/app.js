'use strict';

let {invoke} = window.__TAURI__.core,
	{listen} = window.__TAURI__.event,
	$ = id => document.getElementById(id),
	SHUTOFF_CHOICES = [10, 15, 20, 25, 30],
	SETTINGS_RETRY_MS = 2000,
	SETTINGS_RETRY_MAX_MS = 30000,
	UPDATE_CHECK_MS = 6 * 60 * 60 * 1000,
	BAND_LABELS = ['Low shelf', 'Low mid', 'Mid', 'High mid', 'High shelf'],
	state = null,
	settings = null,
	config = null,
	pairedDevices = [],
	speakerOptions = {outputs: [], detected: null, selected: null},
	settingsFailures = 0,
	settingsRetryAt = 0,
	pendingUpdate = null,
	notifiedUpdate = null,
	dismissedUpdate = null,
	selectedPreset = 1,
	loadingSettings = false,
	toastTimer;

// ── Helpers ──────────────────────────────────────────────────────────────────

function toast(msg, isError) {
	let el = $('toast');
	el.textContent = msg;
	el.classList.toggle('error', !!isError);
	el.hidden = false;
	clearTimeout(toastTimer);
	toastTimer = setTimeout(() => {
		el.hidden = true;
	}, isError ? 5000 : 1800);
}

async function apply(change) {
	try {
		await invoke('apply', {change});
		return true;
	}
	catch (err) {
		toast(String(err), true);
		return false;
	}
}

function headsetReachable() {
	return !!(state && state.connected && state.status && (state.status.on || state.status.docked));
}

function setOutput(input) {
	let out = document.querySelector(`output[data-for="${input.id}"]`);
	if (!out) {
		return;
	}
	if (input.id === 'balance') {
		let chat = Math.round(input.value / 255 * 100);
		out.textContent = `game ${100 - chat}% · chat ${chat}%`;
	}
	else {
		out.textContent = `${input.value}%`;
	}
}

// ── Status header ────────────────────────────────────────────────────────────

function renderState() {
	let st = state,
		b = st.battery,
		chips = [];

	$('battery-text').textContent = b ? `${b.percent}%` : '—';
	$('battery-fill').style.width = b ? `${b.percent}%` : '0';
	$('battery').classList.toggle('low', !!b && !b.charging && b.percent <= 15);

	if (!st.connected) {
		$('headline').textContent = 'Base station not connected';
		$('subline').textContent = st.error || 'Plug in the A50 base station.';
	}
	else if (!st.status) {
		$('headline').textContent = 'Reading headset…';
		$('subline').textContent = st.error || '';
	}
	else {
		let s = st.status;
		$('headline').textContent = s.docked
			? (b && b.charging ? 'Docked · charging' : 'Docked')
			: (s.on ? 'Headset on' : 'Headset off');
		$('subline').textContent = st.audio_target
			? `Audio → ${st.audio_target}`
			: '';
	}

	if (st.connected) {
		chips.push(['Base connected', true]);
	}
	if (st.battery_published) {
		chips.push(['In system battery menu', true]);
	}
	if (st.bluetooth_connected) {
		chips.push([`${st.bluetooth_connected} connected`, true]);
	}
	$('chips').replaceChildren(...chips.map(([text, on]) => {
		let el = document.createElement('span');
		el.className = 'chip' + (on ? ' on' : '');
		el.textContent = text;
		return el;
	}));

	let banner = $('banner');
	if (st.other_holders && st.other_holders.length) {
		banner.textContent = `Another program also has the A50 open: ${st.other_holders.join(', ')}. `
			+ 'Its traffic is filtered out, but quit it (or the old a50-watcher) for the most reliable results.';
		banner.hidden = false;
	}
	else {
		banner.hidden = true;
	}

	let reachable = headsetReachable();
	for (let card of document.querySelectorAll('.needs-headset')) {
		card.classList.toggle('disabled', !reachable || !settings);
	}
	if (reachable && !settings && Date.now() >= settingsRetryAt) {
		loadSettings();
	}
	if (!reachable) {
		settings = null;
	}
	$('battery-pub-hint').textContent = st.battery_published
		? 'Showing now.'
		: 'Needs the system setup below. Shown while the headset is on or docked.';
}

// ── Headset settings ─────────────────────────────────────────────────────────

async function loadSettings() {
	if (loadingSettings) {
		return;
	}
	loadingSettings = true;
	try {
		settings = await invoke('read_settings');
		selectedPreset = settings.active_eq;
		settingsFailures = 0;
		settingsRetryAt = 0;
		renderSettings();
	}
	catch (err) {
		settings = null;
		// renderState retries, so back off or a failing headset gets hammered.
		let delay = Math.min(SETTINGS_RETRY_MS * 2 ** settingsFailures, SETTINGS_RETRY_MAX_MS);
		settingsRetryAt = Date.now() + delay;
		setTimeout(() => state && renderState(), delay);
		if (!settingsFailures) {
			toast(`Couldn't read headset settings: ${err}`, true);
		}
		settingsFailures++;
	}
	finally {
		loadingSettings = false;
		if (state) {
			renderState();
		}
	}
}

function renderSettings() {
	let s = settings;

	$('shutoff').replaceChildren(...SHUTOFF_CHOICES.map(min => {
		let btn = document.createElement('button');
		btn.setAttribute('role', 'radio');
		btn.setAttribute('aria-checked', String(s.shutoff_minutes === min));
		btn.textContent = `${min} min`;
		btn.onclick = async () => {
			if (await apply({kind: 'shutoff', minutes: min})) {
				s.shutoff_minutes = min;
				renderSettings();
				toast(`Auto power-off set to ${min} minutes`);
			}
		};
		return btn;
	}));

	for (let i = 0; i < 6; i++) {
		let input = $(`slider-${i}`);
		input.value = s.sliders[i];
		setOutput(input);
	}
	$('noise-gate').value = s.noise_gate;
	$('mic-eq').value = s.mic_eq;
	$('balance').value = s.default_balance;
	setOutput($('balance'));
	$('alert-volume').value = s.alert_volume;
	setOutput($('alert-volume'));

	renderEq();
}

function renderEq() {
	let s = settings,
		preset = s.presets.find(p => p.id === selectedPreset);

	$('eq-tabs').replaceChildren(...s.presets.map(p => {
		let btn = document.createElement('button');
		btn.setAttribute('role', 'tab');
		btn.setAttribute('aria-selected', String(p.id === selectedPreset));
		btn.textContent = p.name || `Preset ${p.id}`;
		btn.onclick = () => {
			selectedPreset = p.id;
			renderEq();
		};
		return btn;
	}));

	$('eq-name').value = preset.name;
	let active = s.active_eq === preset.id;
	$('eq-activate').hidden = active;
	$('eq-active-badge').hidden = !active;

	$('bands').replaceChildren(...preset.gains.map((gain, i) => {
		let band = document.createElement('div'),
			shelf = i === 0 || i === 4;
		band.className = 'band';
		band.innerHTML = `
			<small>${BAND_LABELS[i]}</small>
			<span class="gain"></span>
			<div class="fader"><input type="range" min="-7" max="7" step="1" aria-label="${BAND_LABELS[i]} gain"></div>
			<div class="band-inputs">
				<label><small>Hz</small><input type="number" min="80" max="15000" step="1" class="freq"></label>
				${shelf ? '' : '<label><small>Q</small><input type="number" min="0.1" max="3" step="0.1" class="q"></label>'}
			</div>`;
		let range = band.querySelector('input[type="range"]'),
			gainLabel = band.querySelector('.gain'),
			freq = band.querySelector('.freq'),
			q = band.querySelector('.q'),
			showGain = v => {
				gainLabel.textContent = `${v > 0 ? '+' : ''}${v} dB`;
			};

		range.value = gain;
		showGain(gain);
		range.oninput = () => showGain(Number(range.value));
		range.onchange = async () => {
			let gains = [...preset.gains];
			gains[i] = Number(range.value);
			if (await apply({kind: 'eq_gains', preset: preset.id, gains})) {
				preset.gains = gains;
			}
			else {
				range.value = preset.gains[i];
				showGain(preset.gains[i]);
			}
		};

		freq.value = preset.bands[i].freq;
		if (q) {
			q.value = (preset.bands[i].bandwidth / 4096).toFixed(1);
		}
		let sendBand = async () => {
			let f = Math.round(Number(freq.value)),
				bw = q ? Math.round(Number(q.value) * 4096) : 0;
			if (!(f >= 80 && f <= 15000)) {
				toast('Frequency must be 80–15000 Hz', true);
				freq.value = preset.bands[i].freq;
				return;
			}
			if (await apply({kind: 'eq_band', preset: preset.id, band: i + 1, freq: f, bandwidth: bw})) {
				preset.bands[i] = {freq: f, bandwidth: bw};
			}
		};
		freq.onchange = sendBand;
		if (q) {
			q.onchange = sendBand;
		}
		return band;
	}));
}

// ── App config & system setup ────────────────────────────────────────────────

async function saveConfig() {
	try {
		await invoke('set_config', {cfg: config});
	}
	catch (err) {
		toast(`Couldn't save preferences: ${err}`, true);
	}
}

function bluetoothButton(text, label, disabled, onclick) {
	let btn = document.createElement('button');
	btn.className = 'bt-button';
	btn.type = 'button';
	btn.textContent = text;
	btn.setAttribute('aria-label', label);
	btn.disabled = disabled;
	btn.onclick = onclick;
	return btn;
}

function moveBluetoothDevice(from, to) {
	let [device] = config.bluetooth_devices.splice(from, 1);
	config.bluetooth_devices.splice(to, 0, device);
	saveConfig();
	renderBluetooth();
}

function renderBluetooth() {
	let devices = config.bluetooth_devices,
		listed = new Set(devices.map(d => d.address)),
		available = pairedDevices.filter(d => !listed.has(d.address));

	$('bt-list').hidden = !devices.length;
	$('bt-list').replaceChildren(...devices.map((device, i) => {
		let row = document.createElement('li'),
			info = document.createElement('div'),
			name = document.createElement('span'),
			detail = document.createElement('small'),
			nickname = document.createElement('input'),
			paired = pairedDevices.find(d => d.address === device.address),
			status = !paired ? 'not paired' : (paired.connected ? 'connected' : 'not connected');

		name.textContent = device.name || device.address;
		detail.textContent = `${device.address} · ${status}`;
		info.className = 'bt-info';
		info.append(name, detail);

		nickname.value = device.nickname;
		nickname.placeholder = 'Nickname';
		nickname.spellcheck = false;
		nickname.setAttribute('aria-label', `Nickname for ${device.name}`);
		nickname.onchange = () => {
			device.nickname = nickname.value.trim();
			nickname.value = device.nickname;
			saveConfig();
		};

		row.append(
			info,
			nickname,
			bluetoothButton('↑', `Move ${device.name} up`, i === 0, () => moveBluetoothDevice(i, i - 1)),
			bluetoothButton('↓', `Move ${device.name} down`, i === devices.length - 1, () => moveBluetoothDevice(i, i + 1)),
			bluetoothButton('Remove', `Remove ${device.name}`, false, () => {
				devices.splice(i, 1);
				saveConfig();
				renderBluetooth();
			}),
		);
		return row;
	}));

	let select = $('bt-add-select');
	select.replaceChildren(...available.map(d => new Option(d.name, d.address)));
	if (!available.length) {
		select.append(new Option(pairedDevices.length ? 'All paired headsets added' : 'No paired headsets found', ''));
	}
	select.disabled = !available.length;
	$('bt-add').disabled = !available.length;
}

async function refreshBluetooth() {
	pairedDevices = await invoke('paired_bluetooth_devices');
	// Follow renames made in the Bluetooth panel.
	let renamed = false;
	for (let device of config.bluetooth_devices) {
		let paired = pairedDevices.find(d => d.address === device.address);
		if (paired && paired.name !== device.name) {
			device.name = paired.name;
			renamed = true;
		}
	}
	if (renamed) {
		saveConfig();
	}
	// Don't rebuild the list under someone typing a nickname.
	if (!(document.activeElement && document.activeElement.matches('#bt-list input'))) {
		renderBluetooth();
	}
}

async function setSpeakers(choice) {
	config.speakers = choice;
	config.speakers_prompted = true;
	await saveConfig();
	await refreshSpeakers();
	promptForUpdate();
}

function renderSpeakers() {
	let {outputs, detected, selected} = speakerOptions,
		select = $('speakers-select'),
		found = outputs.find(o => o.sink_name === detected),
		auto = new Option(found ? `Automatic (${found.label})` : 'Automatic', 'auto'),
		options = [auto, ...outputs.map((o, i) => new Option(o.label, String(i)))],
		current = outputs.findIndex(o => o.sink_name === selected);

	if (config.speakers && current < 0) {
		options.push(new Option(`${config.speakers.label} (not connected)`, 'saved'));
	}
	select.replaceChildren(...options);
	select.value = !config.speakers ? 'auto' : (current < 0 ? 'saved' : String(current));
}

function promptForSpeakers() {
	let dialog = $('speakers-dialog'),
		{outputs, detected} = speakerOptions,
		checked = Math.max(outputs.findIndex(o => o.sink_name === detected), 0);

	$('speaker-choices').replaceChildren(...outputs.map((output, i) => {
		let row = document.createElement('label'),
			radio = document.createElement('input'),
			text = document.createElement('span');
		row.className = 'speaker-choice';
		radio.type = 'radio';
		radio.name = 'speaker-choice';
		radio.value = String(i);
		radio.checked = i === checked;
		text.textContent = output.sink_name === detected ? `${output.label} (detected)` : output.label;
		row.append(radio, text);
		return row;
	}));
	dialog.showModal();
}

async function refreshSpeakers() {
	speakerOptions = await invoke('speaker_options');
	if (document.activeElement !== $('speakers-select')) {
		renderSpeakers();
	}
	if (!config.speakers_prompted && speakerOptions.outputs.length && !$('speakers-dialog').open) {
		promptForSpeakers();
	}
}

function promptForUpdate() {
	let dialog = $('update-dialog'),
		update = pendingUpdate;
	if (!update || update.version === dismissedUpdate || dialog.open || $('speakers-dialog').open) {
		return;
	}
	$('update-text').textContent = `A50 Control ${update.version} is available. You have ${update.current_version}. Download it and restart now?`;
	$('update-notes').textContent = update.notes || '';
	$('update-notes').hidden = !update.notes;
	$('update-install').textContent = 'Update and restart';
	$('update-install').disabled = false;
	$('update-later').disabled = false;
	dialog.showModal();
	if (notifiedUpdate !== update.version) {
		notifiedUpdate = update.version;
		invoke('notify_update', {version: update.version});
	}
}

async function checkForUpdate(manual) {
	if (!manual && !config.check_updates) {
		return;
	}
	try {
		pendingUpdate = await invoke('check_for_update');
	}
	catch (err) {
		// A background check failing (offline, say) isn't worth interrupting for.
		if (manual) {
			toast(`Couldn't check for updates: ${err}`, true);
		}
		return;
	}
	if (manual) {
		dismissedUpdate = null;
		if (!pendingUpdate) {
			toast('A50 Control is up to date');
		}
	}
	promptForUpdate();
}

async function renderSetup() {
	let sys = await invoke('system_status'),
		el = $('setup'),
		ready = sys.hidraw_ok && sys.uhid_ok && sys.rules_installed;

	$('autostart').checked = sys.autostart;
	el.innerHTML = `
		<div class="line"><span class="${sys.hidraw_ok ? 'ok' : 'bad'}">${sys.hidraw_ok ? '✓' : '✗'}</span>
			Access to the A50 base station</div>
		<div class="line"><span class="${sys.uhid_ok ? 'ok' : 'bad'}">${sys.uhid_ok ? '✓' : '✗'}</span>
			Permission to publish the battery (/dev/uhid)</div>
		<div class="line"><span class="${sys.rules_installed ? 'ok' : 'bad'}">${sys.rules_installed ? '✓' : '✗'}</span>
			Battery shown as a headset (udev rules)</div>
		${sys.in_plugdev ? '' : '<div class="line bad">Your user is not in the plugdev group: run <code>sudo usermod -aG plugdev $USER</code> and log in again.</div>'}
		${ready ? '' : '<div class="line"><button class="primary" id="install-rules">Set up system permissions…</button><small>Installs udev rules; asks for your password.</small></div>'}`;
	let btn = $('install-rules');
	if (btn) {
		btn.onclick = async () => {
			btn.disabled = true;
			try {
				await invoke('install_rules');
				toast('System permissions installed');
			}
			catch (err) {
				toast(`Setup failed: ${err}`, true);
			}
			renderSetup();
		};
	}
}

function bindControls() {
	for (let i = 0; i < 6; i++) {
		let input = $(`slider-${i}`);
		input.oninput = () => setOutput(input);
		input.onchange = async () => {
			let value = Number(input.value);
			if (await apply({kind: 'slider', id: i, value})) {
				settings.sliders[i] = value;
			}
		};
	}
	$('balance').oninput = () => setOutput($('balance'));
	$('balance').onchange = async () => {
		let value = Number($('balance').value);
		if (await apply({kind: 'default_balance', value})) {
			settings.default_balance = value;
		}
	};
	$('alert-volume').oninput = () => setOutput($('alert-volume'));
	$('alert-volume').onchange = async () => {
		let value = Number($('alert-volume').value);
		if (await apply({kind: 'alert_volume', value})) {
			settings.alert_volume = value;
		}
	};
	$('noise-gate').onchange = async () => {
		let mode = Number($('noise-gate').value);
		if (await apply({kind: 'noise_gate', mode})) {
			settings.noise_gate = mode;
		}
	};
	$('mic-eq').onchange = async () => {
		let value = Number($('mic-eq').value);
		if (await apply({kind: 'mic_eq', value})) {
			settings.mic_eq = value;
		}
	};
	$('eq-activate').onclick = async () => {
		if (await apply({kind: 'active_eq', preset: selectedPreset})) {
			settings.active_eq = selectedPreset;
			renderEq();
		}
	};
	$('eq-name').onchange = async () => {
		let name = $('eq-name').value.trim().toUpperCase(),
			preset = settings.presets.find(p => p.id === selectedPreset);
		if (await apply({kind: 'eq_name', preset: selectedPreset, name})) {
			preset.name = name;
		}
		renderEq();
	};

	let bindToggle = (id, key) => {
		$(id).checked = config[key];
		$(id).onchange = () => {
			config[key] = $(id).checked;
			saveConfig();
		};
	};
	bindToggle('cfg-audio', 'audio_switching');
	bindToggle('cfg-battery', 'publish_battery');
	bindToggle('cfg-alerts', 'low_battery_alerts');
	bindToggle('cfg-updates', 'check_updates');
	$('update-check').onclick = () => checkForUpdate(true);
	$('update-dialog').addEventListener('cancel', ev => {
		if ($('update-later').disabled) {
			ev.preventDefault();
		}
		else {
			dismissedUpdate = pendingUpdate && pendingUpdate.version;
		}
	});
	$('update-later').onclick = () => {
		dismissedUpdate = pendingUpdate.version;
		$('update-dialog').close();
	};
	$('update-install').onclick = async () => {
		$('update-install').disabled = true;
		$('update-later').disabled = true;
		$('update-install').textContent = 'Downloading…';
		try {
			await invoke('install_update');
		}
		catch (err) {
			$('update-dialog').close();
			dismissedUpdate = pendingUpdate.version;
			toast(`Update failed: ${err}`, true);
		}
	};
	$('speakers-select').onchange = () => {
		let {value} = $('speakers-select');
		if (value !== 'saved') {
			setSpeakers(value === 'auto' ? null : speakerOptions.outputs[Number(value)]);
		}
	};
	$('speakers-dialog').addEventListener('cancel', ev => ev.preventDefault());
	$('speakers-confirm').onclick = () => {
		let picked = document.querySelector('input[name="speaker-choice"]:checked');
		$('speakers-dialog').close();
		setSpeakers(picked ? speakerOptions.outputs[Number(picked.value)] : null);
	};
	$('speakers-auto').onclick = () => {
		$('speakers-dialog').close();
		setSpeakers(null);
	};
	$('bt-add').onclick = () => {
		let device = pairedDevices.find(d => d.address === $('bt-add-select').value);
		if (device) {
			config.bluetooth_devices.push({address: device.address, name: device.name, nickname: ''});
			saveConfig();
			renderBluetooth();
		}
	};
	$('autostart').onchange = async () => {
		try {
			await invoke('set_autostart', {enabled: $('autostart').checked});
		}
		catch (err) {
			toast(String(err), true);
		}
	};
}

// ── Start ────────────────────────────────────────────────────────────────────

(async () => {
	config = await invoke('get_config');
	bindControls();
	renderSetup();
	refreshBluetooth();
	refreshSpeakers();
	state = await invoke('get_state');
	renderState();
	await listen('state', ev => {
		let wasReachable = headsetReachable(),
			wasConnected = state.bluetooth_connected,
			wasSink = state.audio_sink;
		state = ev.payload;
		if (state.bluetooth_connected !== wasConnected) {
			refreshBluetooth();
		}
		if (state.audio_sink !== wasSink) {
			refreshSpeakers();
		}
		if (!wasReachable && headsetReachable()) {
			settings = null;
			settingsFailures = 0;
			settingsRetryAt = 0;
		}
		renderState();
	});
	window.addEventListener('focus', () => {
		renderSetup();
		refreshBluetooth();
		refreshSpeakers();
		promptForUpdate();
	});
	checkForUpdate(false);
	setInterval(() => checkForUpdate(false), UPDATE_CHECK_MS);
})();
