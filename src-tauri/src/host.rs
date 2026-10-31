//! Starting programs outside the AppImage.
//!
//! An AppImage's launcher points PATH, LD_LIBRARY_PATH, GTK_PATH and friends at
//! its mount. Anything we start inherits those, so a browser would load our bundled
//! libraries, and a restarted app would keep paths into a mount that's gone.

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

const LAUNCHER_VARS: [&str; 4] = ["APPDIR", "APPIMAGE", "ARGV0", "OWD"];
const CLOSE_RANGE_CLOEXEC: libc::c_uint = 4;

fn is_bundled(path: &str, appdir: &str) -> bool {
	path.contains("/.mount_") || (!appdir.is_empty() && path.starts_with(appdir))
}

/// `value` with its bundled path entries removed, or None if nothing is left.
fn without_bundled(value: &str, appdir: &str) -> Option<String> {
	let kept: Vec<&str> = value.split(':').filter(|part| !is_bundled(part, appdir)).collect();
	let cleaned = kept.join(":");
	(!cleaned.is_empty()).then_some(cleaned)
}

/// A command for `program` that sees the system, not the AppImage.
pub fn command(program: impl Into<PathBuf>) -> Command {
	let mut command = Command::new(program.into());
	let appdir = std::env::var("APPDIR").unwrap_or_default();
	for (key, value) in std::env::vars() {
		if LAUNCHER_VARS.contains(&key.as_str()) {
			command.env_remove(key);
		}
		else if value.split(':').any(|part| is_bundled(part, &appdir)) {
			match without_bundled(&value, &appdir) {
				Some(cleaned) => command.env(key, cleaned),
				None => command.env_remove(key),
			};
		}
	}
	if let Some(home) = std::env::var_os("HOME") {
		command.current_dir(home);
	}
	// Open files inherited from us would keep this AppImage mounted after we exit.
	// Marked close-on-exec rather than closed, because Rust's own pipe for reporting
	// exec errors is among them. Older kernels lack this, which only risks that leak.
	// SAFETY: a raw syscall with no allocation is safe between fork and exec.
	unsafe {
		command.pre_exec(|| {
			libc::syscall(libc::SYS_close_range, 3 as libc::c_uint, libc::c_uint::MAX, CLOSE_RANGE_CLOEXEC);
			Ok(())
		});
	}
	command
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn keeps_system_entries_in_order() {
		let path = "/tmp/.mount_A50-x/usr/bin:/usr/local/bin:/tmp/.mount_A50-y/bin:/usr/bin";
		assert_eq!(without_bundled(path, "").unwrap(), "/usr/local/bin:/usr/bin");
	}

	#[test]
	fn drops_values_that_are_only_bundled() {
		assert_eq!(without_bundled("/tmp/.mount_A50-x/usr/lib", ""), None);
	}

	#[test]
	fn matches_an_extracted_appdir() {
		assert_eq!(without_bundled("/tmp/squashfs-root/usr/lib:/usr/lib", "/tmp/squashfs-root").unwrap(), "/usr/lib");
	}

	#[test]
	fn leaves_unrelated_entries_alone() {
		assert_eq!(without_bundled("/usr/bin:/bin", "/tmp/.mount_A50-x").unwrap(), "/usr/bin:/bin");
	}
}
