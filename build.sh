#!/usr/bin/env bash
# Builds the A50 Control AppImage.
#
# Checks every build prerequisite and asks before installing each missing one,
# showing the exact command first. Declining stops the script, as does running it
# with no one to answer. Nothing is installed without a yes.
#
# Usage: ./build.sh
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

say() { printf '%s\n' "$*"; }
stop() {
	say "Stopping: $*" >&2
	exit 1
}

# ── Package manager ──────────────────────────────────────────────────────────

if command -v apt-get >/dev/null; then
	pm=apt
elif command -v dnf >/dev/null; then
	pm=dnf
elif command -v pacman >/dev/null; then
	pm=pacman
else
	pm=
fi

if [ "$(id -u)" -eq 0 ]; then
	sudo=
else
	sudo=sudo
fi

apt_updated=

# Prints the command that installs the packages, for showing and for running.
install_command() {
	case $pm in
		apt) printf '%s\n' "${sudo:+$sudo }apt-get install -y $*" ;;
		dnf) printf '%s\n' "${sudo:+$sudo }dnf install -y $*" ;;
		pacman) printf '%s\n' "${sudo:+$sudo }pacman -S --needed --noconfirm $*" ;;
	esac
}

# ask <name> <why> <command>: returns only if the user says yes.
ask() {
	local reply
	say
	say "Missing: $1 ($2)"
	say "  Will run: $3"
	if ! read -r -p "  Install it? [y/N] " reply; then
		say
		stop "$1 is required and no one answered."
	fi
	case $reply in
		[yY] | [yY][eE][sS]) ;;
		*) stop "$1 is required and you declined to install it." ;;
	esac
}

# ── Checks ───────────────────────────────────────────────────────────────────

has_command() { command -v "$1" >/dev/null; }
has_library() { pkg-config --exists "$@"; }
has_appindicator() { has_library ayatana-appindicator3-0.1 || has_library appindicator3-0.1; }
has_node_and_npm() { has_command node && has_command npm; }

# need_package <name> <why> <check...> -- <apt> <dnf> <pacman>
# Package fields hold space-separated names.
need_package() {
	local name=$1 why=$2 check=()
	shift 2
	while [ "$1" != -- ]; do
		check+=("$1")
		shift
	done
	shift
	if "${check[@]}"; then
		say "Found: $name"
		return
	fi

	local packages
	case $pm in
		apt) packages=$1 ;;
		dnf) packages=$2 ;;
		pacman) packages=$3 ;;
		*) stop "$name is required, and this distro's package manager isn't supported. Install it yourself and run this again." ;;
	esac
	if [ -n "$sudo" ] && ! has_command sudo; then
		stop "$name is required, and installing it needs sudo, which isn't available."
	fi

	local command
	command=$(install_command "$packages")
	ask "$name" "$why" "$command"
	if [ "$pm" = apt ] && [ -z "$apt_updated" ]; then
		${sudo:+$sudo} apt-get update
		apt_updated=1
	fi
	$command
	"${check[@]}" || stop "$name is still missing after installing it."
}

# The AppImage bundler finds the indicator library through pkg-config, which must
# itself come first.
need_package "C compiler and build tools" "compiles native code" has_command cc -- \
	build-essential "gcc gcc-c++ make" base-devel
need_package "pkg-config" "finds system libraries" has_command pkg-config -- \
	pkg-config pkgconf-pkg-config pkgconf
need_package "file" "used by the AppImage bundler" has_command file -- \
	file file file
need_package "GTK 3 development files" "window toolkit" has_library gtk+-3.0 -- \
	libgtk-3-dev gtk3-devel gtk3
need_package "WebKitGTK 4.1 development files" "renders the app's UI" \
	has_library webkit2gtk-4.1 javascriptcoregtk-4.1 libsoup-3.0 -- \
	libwebkit2gtk-4.1-dev webkit2gtk4.1-devel webkit2gtk-4.1
need_package "D-Bus development files" "desktop notifications" has_library dbus-1 -- \
	libdbus-1-dev dbus-devel dbus
need_package "AppIndicator development files" "tray icon in the AppImage" has_appindicator -- \
	libayatana-appindicator3-dev libayatana-appindicator-gtk3-devel libayatana-appindicator

if ! has_command cargo || ! has_command rustc; then
	# Distro Rust is often too old for Tauri, so this uses rustup.
	need_package "curl" "downloads the Rust installer" has_command curl -- curl curl curl
	rustup_command="curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
	ask "Rust" "compiles the app, installed for your user only into ~/.cargo and ~/.rustup" "$rustup_command"
	sh -c "$rustup_command"
	export PATH="$HOME/.cargo/bin:$PATH"
	has_command cargo || stop "Rust is still missing after installing it."
fi
say "Found: Rust ($(rustc --version))"

need_package "Node.js and npm" "runs the Tauri build tool" has_node_and_npm -- \
	"nodejs npm" "nodejs npm" "nodejs npm"

if [ ! -x node_modules/.bin/tauri ]; then
	ask "Tauri build tool" "npm package, installed only into this folder's node_modules" "npm ci"
	npm ci
	[ -x node_modules/.bin/tauri ] || stop "The Tauri build tool is still missing after installing it."
fi
say "Found: Tauri build tool"

# ── Build ────────────────────────────────────────────────────────────────────

if has_library ayatana-appindicator3-0.1; then
	export TAURI_LINUX_AYATANA_APPINDICATOR=true
fi
# Lets the bundler's helper AppImages run on systems without FUSE 2.
export APPIMAGE_EXTRACT_AND_RUN=1

say
say "Building..."
npm run build

bundle=src-tauri/target/release/bundle/appimage
appimage=$(find "$bundle" -maxdepth 1 -name '*.AppImage' -printf '%T@ %p\n' | sort -n | tail -1 | cut -d' ' -f2-)
[ -n "$appimage" ] || stop "the build finished but no AppImage was found in $bundle."

say
say "Built: $appimage"

missing_runtime=
for tool in pactl busctl pkexec; do
	has_command "$tool" || missing_runtime="$missing_runtime $tool"
done
if [ -n "$missing_runtime" ]; then
	say "The app also needs these at runtime, which weren't found:$missing_runtime"
	say "See Install in README.md."
fi
