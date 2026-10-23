#!/bin/sh
# Builds GNOME Power Statistics (gnome-power-manager 43.0) with model-names.patch:
# the device list shows a peripheral's model name ("Astro A50 Headset") instead of
# only its kind, and capitalises kinds the app has no translation for.
#
# Installs the binary to /path/to/gnome-power-statistics
# and a user launcher overriding the system one. The system package is left alone.
# No sudo: the upower-glib headers are unpacked from the .deb into the build dir.
set -eu

DEST=${DEST:-/path/to/gnome-power-statistics}
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

apt-get source --download-only gnome-power-manager=43.0-2build2
tar xJf gnome-power-manager_43.0.orig.tar.xz
apt-get download libupower-glib-dev
dpkg-deb -x libupower-glib-dev_*.deb sysroot
LIBDIR=$WORK/sysroot/usr/lib/x86_64-linux-gnu
sed -i "s|^prefix=.*|prefix=$WORK/sysroot/usr|" "$LIBDIR/pkgconfig/upower-glib.pc"
ln -sf /usr/lib/x86_64-linux-gnu/libupower-glib.so.3 "$LIBDIR/libupower-glib.so"

cd gnome-power-manager-43.0
patch -p1 < "$HERE/model-names.patch"
CFLAGS="-I$WORK/sysroot/usr/include" PKG_CONFIG_PATH="$LIBDIR/pkgconfig" \
	meson setup build --prefix=/usr
ninja -C build src/gnome-power-statistics

mkdir -p "$DEST" "$HOME/.local/share/applications"
install -m 0755 build/src/gnome-power-statistics "$DEST/"
sed -E "s|^Exec=gnome-power-statistics|Exec=$DEST/gnome-power-statistics|" \
	/usr/share/applications/org.gnome.PowerStats.desktop \
	> "$HOME/.local/share/applications/org.gnome.PowerStats.desktop"
echo "Installed $DEST/gnome-power-statistics"
