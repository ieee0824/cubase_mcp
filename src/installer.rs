use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const MIDI_REMOTE_SCRIPT: &str =
    include_str!("../cubase/midi_remote/CubaseMCP/CubaseMCP/CubaseMCP_CubaseMCP.js");

const INSTALL_SUBDIRECTORY: &str = "CubaseMCP/CubaseMCP";
const SCRIPT_FILE_NAME: &str = "CubaseMCP_CubaseMCP.js";

pub fn install_midi_remote() -> io::Result<Vec<PathBuf>> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "HOME is not set; cannot locate the Steinberg documents directory",
        )
    })?;
    let steinberg = PathBuf::from(home).join("Documents/Steinberg");
    let mut roots = Vec::new();
    for entry in fs::read_dir(&steinberg)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let local = entry.path().join("MIDI Remote/Driver Scripts/Local");
        if local.is_dir() {
            roots.push(local);
        }
    }
    if roots.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "No Cubase MIDI Remote Driver Scripts directory was found below {}",
                steinberg.display()
            ),
        ));
    }
    install_into_roots(&roots)
}

fn install_into_roots(roots: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut installed = Vec::new();
    for root in roots {
        let directory = root.join(INSTALL_SUBDIRECTORY);
        fs::create_dir_all(&directory)?;
        let destination = directory.join(SCRIPT_FILE_NAME);
        backup_if_different(&destination)?;
        fs::write(&destination, MIDI_REMOTE_SCRIPT)?;
        installed.push(destination);
    }
    Ok(installed)
}

fn backup_if_different(destination: &Path) -> io::Result<()> {
    let Ok(existing) = fs::read(destination) else {
        return Ok(());
    };
    if existing == MIDI_REMOTE_SCRIPT.as_bytes() {
        return Ok(());
    }

    let mut backup_name: OsString = destination.file_name().unwrap_or_default().to_os_string();
    backup_name.push(".bak");
    fs::copy(destination, destination.with_file_name(backup_name))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn installer_writes_expected_vendor_device_layout() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cubase-mcp-installer-{unique}"));
        fs::create_dir_all(&root).unwrap();

        let paths = install_into_roots(std::slice::from_ref(&root)).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0],
            root.join("CubaseMCP/CubaseMCP/CubaseMCP_CubaseMCP.js")
        );
        assert_eq!(fs::read_to_string(&paths[0]).unwrap(), MIDI_REMOTE_SCRIPT);

        fs::remove_dir_all(root).unwrap();
    }
}
