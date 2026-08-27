use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use sysinfo::System;

use crate::config::TrackProbeInstallOptions;

pub const MIDI_REMOTE_SCRIPT: &str =
    include_str!("../cubase/midi_remote/CubaseMCP/CubaseMCP/CubaseMCP_CubaseMCP.js");
pub const TRACK_PROBE_SCRIPT: &[u8] = include_bytes!(
    "../cubase/midi_remote/CubaseMCPTrackProbe/CubaseMCPTrackProbe/CubaseMCPTrackProbe_CubaseMCPTrackProbe.js"
);

#[derive(Clone, Copy)]
struct BundledScript {
    install_subdirectory: &'static str,
    file_name: &'static str,
    contents: &'static [u8],
}

const MIDI_REMOTE: BundledScript = BundledScript {
    install_subdirectory: "CubaseMCP/CubaseMCP",
    file_name: "CubaseMCP_CubaseMCP.js",
    contents: MIDI_REMOTE_SCRIPT.as_bytes(),
};

const TRACK_PROBE: BundledScript = BundledScript {
    install_subdirectory: "CubaseMCPTrackProbe/CubaseMCPTrackProbe",
    file_name: "CubaseMCPTrackProbe_CubaseMCPTrackProbe.js",
    contents: TRACK_PROBE_SCRIPT,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackProbeInstallReport {
    pub script: &'static str,
    pub destination: PathBuf,
    pub changed: bool,
    pub embedded_source_sha256: String,
    pub previous_sha256: Option<String>,
    pub deployed_sha256: String,
    pub verified: bool,
    pub cubase_process_check: &'static str,
}

trait CubaseProcessChecker {
    fn cubase_is_running(&mut self) -> io::Result<bool>;
}

struct SystemProcessChecker;

impl CubaseProcessChecker for SystemProcessChecker {
    fn cubase_is_running(&mut self) -> io::Result<bool> {
        let system = System::new_all();
        let current_pid = sysinfo::get_current_pid().map_err(io::Error::other)?;
        if system.process(current_pid).is_none() {
            return Err(io::Error::other(
                "process enumeration did not include the installer process",
            ));
        }

        Ok(system.processes().values().any(|process| {
            is_cubase_process_name(process.name())
                || process
                    .exe()
                    .and_then(Path::file_name)
                    .is_some_and(is_cubase_process_name)
        }))
    }
}

pub fn install_midi_remote() -> io::Result<Vec<PathBuf>> {
    install_discovered(MIDI_REMOTE)
}

pub fn install_track_probe(
    options: &TrackProbeInstallOptions,
) -> io::Result<TrackProbeInstallReport> {
    let documents = documents_directory()?;
    install_track_probe_with(options, &documents, &mut SystemProcessChecker)
}

fn install_track_probe_with(
    options: &TrackProbeInstallOptions,
    documents: &Path,
    process_checker: &mut impl CubaseProcessChecker,
) -> io::Result<TrackProbeInstallReport> {
    let root = select_track_probe_root(options.midi_remote_root.as_deref(), documents)?;
    ensure_cubase_is_closed(process_checker)?;

    let install_parent = root.join("CubaseMCPTrackProbe");
    let directory = root.join(TRACK_PROBE.install_subdirectory);
    let destination = directory.join(TRACK_PROBE.file_name);
    validate_install_directory_component(&install_parent)?;
    validate_install_directory_component(&directory)?;
    let existing = read_regular_file_if_present(&destination)?;
    let embedded_digest = sha256_hex(TRACK_PROBE.contents);
    let previous_digest = existing.as_deref().map(sha256_hex);

    if existing.as_deref() == Some(TRACK_PROBE.contents) {
        validate_track_probe_directories(&root, &install_parent, &directory)?;
        if read_regular_file_if_present(&destination)?.as_deref() != Some(TRACK_PROBE.contents) {
            return Err(io::Error::other(
                "Track Probe changed while the installer was verifying the existing deployment",
            ));
        }
        validate_track_probe_directories(&root, &install_parent, &directory)?;
        return Ok(TrackProbeInstallReport {
            script: "track_probe",
            destination,
            changed: false,
            embedded_source_sha256: embedded_digest.clone(),
            previous_sha256: previous_digest,
            deployed_sha256: embedded_digest,
            verified: true,
            cubase_process_check: "no_running_process_observed",
        });
    }

    if existing.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "a different Track Probe already exists at {} (existing SHA-256 {}, embedded SHA-256 {}); the installer never replaces existing probe bytes, so preserve and remove the existing file manually before retrying",
                destination.display(),
                previous_digest.as_deref().unwrap_or("unavailable"),
                embedded_digest
            ),
        ));
    }

    let install_parent_created = create_strict_directory(&install_parent)?;
    let directory_created = match create_strict_directory(&directory) {
        Ok(created) => created,
        Err(error) => {
            if install_parent_created {
                let _ = fs::remove_dir(&install_parent);
            }
            return Err(error);
        }
    };
    let install_parent_existed = !install_parent_created;
    let directory_existed = !directory_created;

    if let Err(error) = ensure_cubase_is_closed(process_checker) {
        cleanup_new_directories(
            &install_parent,
            &directory,
            install_parent_existed,
            directory_existed,
        );
        return Err(error);
    }

    let current = match read_regular_file_if_present(&destination) {
        Ok(current) => current,
        Err(error) => {
            cleanup_new_directories(
                &install_parent,
                &directory,
                install_parent_existed,
                directory_existed,
            );
            return Err(error);
        }
    };
    if current.as_deref() != existing.as_deref() {
        cleanup_new_directories(
            &install_parent,
            &directory,
            install_parent_existed,
            directory_existed,
        );
        return Err(io::Error::other(
            "Track Probe destination changed during installation; no file was replaced",
        ));
    }

    validate_track_probe_directories(&root, &install_parent, &directory)?;

    if let Err(error) = write_new_track_probe(&destination, TRACK_PROBE.contents) {
        cleanup_new_directories(
            &install_parent,
            &directory,
            install_parent_existed,
            directory_existed,
        );
        return Err(error);
    }

    if let Err(error) = validate_track_probe_directories(&root, &install_parent, &directory) {
        return Err(io::Error::new(
            error.kind(),
            format!(
                "Track Probe directory changed after the new file was created; the path was preserved for manual inspection: {error}"
            ),
        ));
    }

    let deployed = match fs::read(&destination) {
        Ok(deployed) if deployed == TRACK_PROBE.contents => deployed,
        Ok(_) => {
            cleanup_new_directories(
                &install_parent,
                &directory,
                install_parent_existed,
                directory_existed,
            );
            return Err(io::Error::other(format!(
                "deployed Track Probe digest did not match the embedded source at {}; the unexpected file was preserved",
                destination.display()
            )));
        }
        Err(error) => {
            cleanup_new_directories(
                &install_parent,
                &directory,
                install_parent_existed,
                directory_existed,
            );
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "could not verify deployed Track Probe at {}: {error}",
                    destination.display()
                ),
            ));
        }
    };
    if let Err(error) = sync_directory(&directory) {
        cleanup_new_directories(
            &install_parent,
            &directory,
            install_parent_existed,
            directory_existed,
        );
        return Err(io::Error::new(
            error.kind(),
            format!(
                "could not durably sync Track Probe directory {}: {error}",
                directory.display()
            ),
        ));
    }

    Ok(TrackProbeInstallReport {
        script: "track_probe",
        destination,
        changed: true,
        embedded_source_sha256: embedded_digest,
        previous_sha256: None,
        deployed_sha256: sha256_hex(&deployed),
        verified: true,
        cubase_process_check: "no_running_process_observed",
    })
}

fn install_discovered(script: BundledScript) -> io::Result<Vec<PathBuf>> {
    let documents = documents_directory()?;
    let roots = discover_midi_remote_roots(&documents)?;
    install_into_roots(&roots, script)
}

fn documents_directory() -> io::Result<PathBuf> {
    dirs::document_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "The operating system did not provide a Documents directory; cannot locate the Steinberg directory",
        )
    })
}

fn discover_midi_remote_roots(documents: &Path) -> io::Result<Vec<PathBuf>> {
    let steinberg = documents.join("Steinberg");
    let mut roots = Vec::new();
    for entry in fs::read_dir(&steinberg)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !is_cubase_product_directory_name(&entry.file_name()) {
            continue;
        }
        let local = entry.path().join("MIDI Remote/Driver Scripts/Local");
        if local.is_dir() {
            roots.push(local);
        }
    }
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "No Cubase MIDI Remote Driver Scripts directory was found below {}",
                steinberg.display()
            ),
        ));
    }
    Ok(roots)
}

fn install_into_roots(roots: &[PathBuf], script: BundledScript) -> io::Result<Vec<PathBuf>> {
    let mut installed = Vec::new();
    for root in roots {
        let directory = root.join(script.install_subdirectory);
        fs::create_dir_all(&directory)?;
        let destination = directory.join(script.file_name);
        backup_if_different(&destination, script.contents)?;
        fs::write(&destination, script.contents)?;
        installed.push(destination);
    }
    Ok(installed)
}

fn select_track_probe_root(explicit: Option<&Path>, documents: &Path) -> io::Result<PathBuf> {
    if let Some(explicit) = explicit {
        return validate_midi_remote_root(explicit, documents);
    }

    let roots = discover_midi_remote_roots(documents)?;
    if roots.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "found {} Cubase MIDI Remote roots; specify exactly one with --midi-remote-root",
                roots.len()
            ),
        ));
    }
    validate_midi_remote_root(&roots[0], documents)
}

fn validate_midi_remote_root(root: &Path, documents: &Path) -> io::Result<PathBuf> {
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not resolve MIDI Remote root {}: {error}",
                root.display()
            ),
        )
    })?;
    if !canonical_root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "MIDI Remote root is not a directory: {}",
                canonical_root.display()
            ),
        ));
    }

    let steinberg = fs::canonicalize(documents.join("Steinberg"))?;
    let relative = canonical_root.strip_prefix(&steinberg).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("MIDI Remote root must be below {}", steinberg.display()),
        )
    })?;
    let components: Vec<_> = relative.components().collect();
    let expected_tail = ["MIDI Remote", "Driver Scripts", "Local"];
    let valid = components.len() == 4
        && matches!(components[0], std::path::Component::Normal(value) if is_cubase_product_directory_name(value))
        && components[1..]
            .iter()
            .zip(expected_tail)
            .all(|(actual, expected)| {
                matches!(actual, std::path::Component::Normal(value) if *value == OsStr::new(expected))
            });
    if !valid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "MIDI Remote root must have the form {}/<Cubase product>/MIDI Remote/Driver Scripts/Local",
                steinberg.display()
            ),
        ));
    }
    Ok(canonical_root)
}

fn is_cubase_product_directory_name(name: &OsStr) -> bool {
    let lowercase = name.to_string_lossy().trim().to_ascii_lowercase();
    let Some(suffix) = lowercase.strip_prefix("cubase") else {
        return false;
    };
    suffix.is_empty()
        || suffix.starts_with(' ')
        || suffix.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

fn ensure_cubase_is_closed(checker: &mut impl CubaseProcessChecker) -> io::Result<()> {
    match checker.cubase_is_running() {
        Ok(false) => Ok(()),
        Ok(true) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Cubase is running; close every Cubase instance before installing the Track Probe",
        )),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!(
                "could not prove that Cubase is closed; Track Probe was not installed: {error}"
            ),
        )),
    }
}

fn is_cubase_process_name(name: &OsStr) -> bool {
    let lowercase = name.to_string_lossy().trim().to_ascii_lowercase();
    let normalized = lowercase.strip_suffix(".exe").unwrap_or(&lowercase);
    let Some(suffix) = normalized.strip_prefix("cubase") else {
        return false;
    };
    suffix.is_empty()
        || suffix.starts_with(' ')
        || suffix.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

fn read_regular_file_if_present(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to replace symbolic link {}", path.display()),
        )),
        Ok(metadata) if !metadata.is_file() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("script destination is not a file: {}", path.display()),
        )),
        Ok(_) => fs::read(path).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn validate_install_directory_component(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to use symbolic-link directory {}", path.display()),
        )),
        Ok(metadata) if !metadata.is_dir() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("installer path is not a directory: {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn create_strict_directory(path: &Path) -> io::Result<bool> {
    match fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            validate_install_directory_component(path)?;
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn validate_track_probe_directories(
    root: &Path,
    install_parent: &Path,
    directory: &Path,
) -> io::Result<()> {
    validate_install_directory_component(install_parent)?;
    validate_install_directory_component(directory)?;

    let canonical_parent = fs::canonicalize(install_parent)?;
    let canonical_directory = fs::canonicalize(directory)?;
    if canonical_parent != root.join("CubaseMCPTrackProbe")
        || canonical_directory != root.join(TRACK_PROBE.install_subdirectory)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Track Probe install directories must not contain symbolic links, junctions, or redirected components",
        ));
    }
    Ok(())
}

fn write_new_track_probe(destination: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(destination)?;
    file.write_all(contents).and_then(|()| file.sync_all()).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not completely write new Track Probe at {}; the installer preserved the path for manual inspection: {error}",
                destination.display()
            ),
        )
    })
}

fn cleanup_new_directories(
    install_parent: &Path,
    directory: &Path,
    install_parent_existed: bool,
    directory_existed: bool,
) {
    if !directory_existed {
        let _ = fs::remove_dir(directory);
    }
    if !install_parent_existed {
        let _ = fs::remove_dir(install_parent);
    }
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> io::Result<()> {
    fs::File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> io::Result<()> {
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn backup_if_different(destination: &Path, contents: &[u8]) -> io::Result<()> {
    let Ok(existing) = fs::read(destination) else {
        return Ok(());
    };
    if existing == contents {
        return Ok(());
    }

    let mut backup_name: OsString = destination.file_name().unwrap_or_default().to_os_string();
    backup_name.push(".bak");
    fs::copy(destination, destination.with_file_name(backup_name))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    enum StubOutcome {
        Running(bool),
        Failure,
    }

    struct StubProcessChecker {
        outcomes: VecDeque<StubOutcome>,
        calls: usize,
    }

    impl StubProcessChecker {
        fn new(outcomes: impl IntoIterator<Item = StubOutcome>) -> Self {
            Self {
                outcomes: outcomes.into_iter().collect(),
                calls: 0,
            }
        }
    }

    impl CubaseProcessChecker for StubProcessChecker {
        fn cubase_is_running(&mut self) -> io::Result<bool> {
            self.calls += 1;
            match self.outcomes.pop_front() {
                Some(StubOutcome::Running(running)) => Ok(running),
                Some(StubOutcome::Failure) => Err(io::Error::other("simulated scan failure")),
                None => Err(io::Error::other("unexpected extra process scan")),
            }
        }
    }

    #[test]
    fn installer_writes_expected_vendor_device_layout() {
        let root = unique_test_root("production");
        fs::create_dir_all(&root).unwrap();

        let paths = install_into_roots(std::slice::from_ref(&root), MIDI_REMOTE).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0],
            root.join("CubaseMCP/CubaseMCP/CubaseMCP_CubaseMCP.js")
        );
        assert_eq!(fs::read_to_string(&paths[0]).unwrap(), MIDI_REMOTE_SCRIPT);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn track_probe_installer_writes_verifies_and_reports_fresh_install() {
        let fixture = InstallFixture::new("fresh", &["Cubase"]);
        let root = fixture.midi_remote_root("Cubase");
        let mut checker =
            StubProcessChecker::new([StubOutcome::Running(false), StubOutcome::Running(false)]);

        let report = install_track_probe_with(
            &track_probe_options(Some(root)),
            &fixture.documents,
            &mut checker,
        )
        .unwrap();

        assert!(report.changed);
        assert!(report.verified);
        assert_eq!(report.previous_sha256, None);
        assert_eq!(
            report.embedded_source_sha256,
            sha256_hex(TRACK_PROBE_SCRIPT)
        );
        assert_eq!(report.deployed_sha256, report.embedded_source_sha256);
        assert_eq!(fs::read(&report.destination).unwrap(), TRACK_PROBE_SCRIPT);
        assert_eq!(checker.calls, 2);
    }

    #[test]
    fn same_track_probe_is_a_verified_noop() {
        let fixture = InstallFixture::new("noop", &["Cubase"]);
        let root = fixture.midi_remote_root("Cubase");
        let destination = track_probe_destination(&root);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, TRACK_PROBE_SCRIPT).unwrap();
        let mut checker = StubProcessChecker::new([StubOutcome::Running(false)]);

        let report = install_track_probe_with(
            &track_probe_options(Some(root)),
            &fixture.documents,
            &mut checker,
        )
        .unwrap();

        assert!(!report.changed);
        assert_eq!(report.previous_sha256, Some(sha256_hex(TRACK_PROBE_SCRIPT)));
        assert_eq!(checker.calls, 1);
        assert_eq!(fs::read(destination).unwrap(), TRACK_PROBE_SCRIPT);
    }

    #[test]
    fn different_track_probe_is_always_rejected_without_modification() {
        let fixture = InstallFixture::new("existing-different", &["Cubase"]);
        let root = fixture.midi_remote_root("Cubase");
        let destination = track_probe_destination(&root);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, b"local probe").unwrap();
        let mut checker = StubProcessChecker::new([StubOutcome::Running(false)]);

        let error = install_track_probe_with(
            &track_probe_options(Some(root)),
            &fixture.documents,
            &mut checker,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&destination).unwrap(), b"local probe");
        assert!(destination.parent().unwrap().read_dir().unwrap().count() == 1);
    }

    #[test]
    fn running_cubase_or_process_scan_failure_makes_no_filesystem_change() {
        for (label, outcome) in [
            ("running", StubOutcome::Running(true)),
            ("scan-failure", StubOutcome::Failure),
        ] {
            let fixture = InstallFixture::new(label, &["Cubase"]);
            let root = fixture.midi_remote_root("Cubase");
            let install_parent = root.join("CubaseMCPTrackProbe");
            let mut checker = StubProcessChecker::new([outcome]);

            assert!(
                install_track_probe_with(
                    &track_probe_options(Some(root)),
                    &fixture.documents,
                    &mut checker,
                )
                .is_err()
            );
            assert!(!install_parent.exists());
        }
    }

    #[test]
    fn cubase_detected_before_create_leaves_no_probe_artifacts() {
        let fixture = InstallFixture::new("second-scan", &["Cubase"]);
        let root = fixture.midi_remote_root("Cubase");
        let install_parent = root.join("CubaseMCPTrackProbe");
        let mut checker =
            StubProcessChecker::new([StubOutcome::Running(false), StubOutcome::Running(true)]);

        assert!(
            install_track_probe_with(
                &track_probe_options(Some(root)),
                &fixture.documents,
                &mut checker,
            )
            .is_err()
        );
        assert!(!install_parent.exists());
    }

    #[test]
    fn ambiguous_or_outside_root_is_rejected_without_process_scan() {
        let fixture = InstallFixture::new("root-validation", &["Cubase", "Cubase LE AI Elements"]);
        let mut ambiguous_checker = StubProcessChecker::new([]);
        assert!(
            install_track_probe_with(
                &track_probe_options(None),
                &fixture.documents,
                &mut ambiguous_checker,
            )
            .is_err()
        );
        assert_eq!(ambiguous_checker.calls, 0);

        let outside = fixture.base.join("Outside");
        fs::create_dir_all(&outside).unwrap();
        let mut outside_checker = StubProcessChecker::new([]);
        assert!(
            install_track_probe_with(
                &track_probe_options(Some(outside)),
                &fixture.documents,
                &mut outside_checker,
            )
            .is_err()
        );
        assert_eq!(outside_checker.calls, 0);
    }

    #[test]
    fn non_cubase_steinberg_products_are_never_selected_or_accepted() {
        let fixture = InstallFixture::new("non-cubase-root", &["Nuendo 14"]);
        let nuendo_root = fixture.midi_remote_root("Nuendo 14");

        let mut discovered_checker = StubProcessChecker::new([]);
        assert!(
            install_track_probe_with(
                &track_probe_options(None),
                &fixture.documents,
                &mut discovered_checker,
            )
            .is_err()
        );
        assert_eq!(discovered_checker.calls, 0);

        let mut explicit_checker = StubProcessChecker::new([]);
        assert!(
            install_track_probe_with(
                &track_probe_options(Some(nuendo_root)),
                &fixture.documents,
                &mut explicit_checker,
            )
            .is_err()
        );
        assert_eq!(explicit_checker.calls, 0);
    }

    #[test]
    fn discovery_ignores_non_cubase_roots_when_one_cubase_root_exists() {
        let fixture = InstallFixture::new("mixed-products", &["Nuendo 14", "Cubase 15"]);
        let roots = discover_midi_remote_roots(&fixture.documents).unwrap();

        assert_eq!(roots, vec![fixture.midi_remote_root("Cubase 15")]);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_destination_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;

        let fixture = InstallFixture::new("symlink", &["Cubase"]);
        let root = fixture.midi_remote_root("Cubase");
        let destination = track_probe_destination(&root);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let target = fixture.base.join("target.js");
        fs::write(&target, b"target contents").unwrap();
        symlink(&target, &destination).unwrap();
        let mut checker = StubProcessChecker::new([StubOutcome::Running(false)]);

        assert!(
            install_track_probe_with(
                &track_probe_options(Some(root)),
                &fixture.documents,
                &mut checker,
            )
            .is_err()
        );
        assert_eq!(fs::read(target).unwrap(), b"target contents");
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_install_directory_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;

        let fixture = InstallFixture::new("directory-symlink", &["Cubase"]);
        let root = fixture.midi_remote_root("Cubase");
        let target = fixture.base.join("outside-directory");
        let target_directory = target.join("CubaseMCPTrackProbe");
        fs::create_dir_all(&target_directory).unwrap();
        let target_script = target_directory.join(TRACK_PROBE.file_name);
        fs::write(&target_script, TRACK_PROBE_SCRIPT).unwrap();
        symlink(&target, root.join("CubaseMCPTrackProbe")).unwrap();
        let mut checker = StubProcessChecker::new([StubOutcome::Running(false)]);

        assert!(
            install_track_probe_with(
                &track_probe_options(Some(root)),
                &fixture.documents,
                &mut checker,
            )
            .is_err()
        );
        assert_eq!(fs::read(target_script).unwrap(), TRACK_PROBE_SCRIPT);
    }

    #[test]
    fn fresh_publish_never_overwrites_a_destination_that_appeared_late() {
        let root = unique_test_root("late-fresh-destination");
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("probe.js");
        fs::write(&destination, b"late destination").unwrap();

        assert!(write_new_track_probe(&destination, TRACK_PROBE_SCRIPT).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"late destination");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn process_name_detection_is_specific_to_cubase_products() {
        for name in [
            "Cubase",
            "Cubase 15",
            "Cubase15.exe",
            "Cubase LE AI Elements",
        ] {
            assert!(is_cubase_process_name(OsStr::new(name)), "{name}");
        }
        for name in [
            "cubase_mcp",
            "cubase_track_probe_collector",
            "CubaseHelper",
            "NotCubase",
        ] {
            assert!(!is_cubase_process_name(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn product_directory_detection_is_specific_to_cubase_products() {
        for name in ["Cubase", "Cubase 15", "Cubase LE AI Elements"] {
            assert!(is_cubase_product_directory_name(OsStr::new(name)), "{name}");
        }
        for name in ["Nuendo 14", "WaveLab", "CubaseHelper", "NotCubase"] {
            assert!(
                !is_cubase_product_directory_name(OsStr::new(name)),
                "{name}"
            );
        }
    }

    #[test]
    fn embedded_track_probe_digest_matches_reviewed_source() {
        assert_eq!(
            sha256_hex(TRACK_PROBE_SCRIPT),
            "71bf8b7d46ec859d9f6a2d2d2e704b7188719d02991118dc5e3da42fd695c290"
        );
    }

    fn unique_test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("cubase-mcp-installer-{label}-{unique}"))
    }

    fn track_probe_options(midi_remote_root: Option<PathBuf>) -> TrackProbeInstallOptions {
        TrackProbeInstallOptions { midi_remote_root }
    }

    fn track_probe_destination(root: &Path) -> PathBuf {
        root.join(TRACK_PROBE.install_subdirectory)
            .join(TRACK_PROBE.file_name)
    }

    struct InstallFixture {
        base: PathBuf,
        documents: PathBuf,
    }

    impl InstallFixture {
        fn new(label: &str, products: &[&str]) -> Self {
            let base = unique_test_root(label);
            let documents = base.join("Documents");
            for product in products {
                fs::create_dir_all(
                    documents
                        .join("Steinberg")
                        .join(product)
                        .join("MIDI Remote/Driver Scripts/Local"),
                )
                .unwrap();
            }
            Self { base, documents }
        }

        fn midi_remote_root(&self, product: &str) -> PathBuf {
            self.documents
                .join("Steinberg")
                .join(product)
                .join("MIDI Remote/Driver Scripts/Local")
        }
    }

    impl Drop for InstallFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }
}
