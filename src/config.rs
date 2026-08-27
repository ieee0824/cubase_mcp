use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crate::bridge::{DEFAULT_FROM_CUBASE_PORT, DEFAULT_TO_CUBASE_PORT, MIN_MIDI_TIMEOUT_MS};

const DEFAULT_BRIDGE_MODE: &str = "tcp";
const DEFAULT_BRIDGE_ADDRESS: &str = "127.0.0.1:8765";
const DEFAULT_TIMEOUT_MS: u64 = 2_000;
const MAX_TIMEOUT_MS: u64 = 600_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bridge_mode: String,
    pub bridge_address: String,
    pub midi_input_port: Option<String>,
    pub midi_output_port: Option<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackProbeInstallOptions {
    pub midi_remote_root: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    Run(Config),
    InstallMidiRemote,
    InstallTrackProbe(TrackProbeInstallOptions),
    ListMidiPorts,
    PrintHelp,
    PrintVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecialAction {
    InstallMidiRemote,
    InstallTrackProbe,
    ListMidiPorts,
    PrintHelp,
    PrintVersion,
}

impl Config {
    pub fn from_process() -> Result<CliAction, String> {
        let bridge_mode =
            env::var("CUBASE_MCP_BRIDGE_MODE").unwrap_or_else(|_| DEFAULT_BRIDGE_MODE.to_owned());
        let bridge_address = env::var("CUBASE_MCP_BRIDGE_ADDRESS")
            .unwrap_or_else(|_| DEFAULT_BRIDGE_ADDRESS.to_owned());
        let timeout_ms = env::var("CUBASE_MCP_TIMEOUT_MS")
            .ok()
            .map(|value| parse_timeout(&value))
            .transpose()?
            .unwrap_or(DEFAULT_TIMEOUT_MS);

        Self::parse(
            env::args().skip(1),
            Config {
                bridge_mode,
                bridge_address,
                midi_input_port: None,
                midi_output_port: None,
                timeout: Duration::from_millis(timeout_ms),
            },
        )
    }

    pub fn parse(
        arguments: impl IntoIterator<Item = String>,
        mut config: Config,
    ) -> Result<CliAction, String> {
        let mut arguments = arguments.into_iter();
        let mut special_action = None;
        let mut run_option_seen = false;
        let mut midi_remote_root = None;
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--bridge" => {
                    run_option_seen = true;
                    config.bridge_mode = next_value(&mut arguments, "--bridge")?;
                }
                "--bridge-address" => {
                    run_option_seen = true;
                    config.bridge_address = next_value(&mut arguments, "--bridge-address")?;
                }
                "--midi-input" => {
                    run_option_seen = true;
                    config.midi_input_port = Some(next_value(&mut arguments, "--midi-input")?);
                }
                "--midi-output" => {
                    run_option_seen = true;
                    config.midi_output_port = Some(next_value(&mut arguments, "--midi-output")?);
                }
                "--timeout-ms" => {
                    run_option_seen = true;
                    let value = next_value(&mut arguments, "--timeout-ms")?;
                    config.timeout = Duration::from_millis(parse_timeout(&value)?);
                }
                "--install-midi-remote" => select_special_action(
                    &mut special_action,
                    SpecialAction::InstallMidiRemote,
                    &argument,
                )?,
                "--install-track-probe" => select_special_action(
                    &mut special_action,
                    SpecialAction::InstallTrackProbe,
                    &argument,
                )?,
                "--midi-remote-root" => {
                    if midi_remote_root.is_some() {
                        return Err("--midi-remote-root must not be repeated".into());
                    }
                    midi_remote_root = Some(PathBuf::from(next_path_value(
                        &mut arguments,
                        "--midi-remote-root",
                    )?));
                }
                "--list-midi-ports" => select_special_action(
                    &mut special_action,
                    SpecialAction::ListMidiPorts,
                    &argument,
                )?,
                "-h" | "--help" => {
                    select_special_action(&mut special_action, SpecialAction::PrintHelp, &argument)?
                }
                "-V" | "--version" => select_special_action(
                    &mut special_action,
                    SpecialAction::PrintVersion,
                    &argument,
                )?,
                _ => return Err(format!("Unknown argument '{argument}'")),
            }
        }

        if let Some(action) = special_action {
            if run_option_seen {
                return Err("Bridge options cannot be combined with a one-shot CLI action".into());
            }
            return match action {
                SpecialAction::InstallTrackProbe => {
                    Ok(CliAction::InstallTrackProbe(TrackProbeInstallOptions {
                        midi_remote_root,
                    }))
                }
                SpecialAction::InstallMidiRemote => {
                    reject_track_probe_only_options(&midi_remote_root)?;
                    Ok(CliAction::InstallMidiRemote)
                }
                SpecialAction::ListMidiPorts => {
                    reject_track_probe_only_options(&midi_remote_root)?;
                    Ok(CliAction::ListMidiPorts)
                }
                SpecialAction::PrintHelp => {
                    reject_track_probe_only_options(&midi_remote_root)?;
                    Ok(CliAction::PrintHelp)
                }
                SpecialAction::PrintVersion => {
                    reject_track_probe_only_options(&midi_remote_root)?;
                    Ok(CliAction::PrintVersion)
                }
            };
        }

        reject_track_probe_only_options(&midi_remote_root)?;

        if !matches!(config.bridge_mode.as_str(), "tcp" | "midi" | "mock") {
            return Err(format!(
                "Unsupported bridge mode '{}'; expected 'tcp', 'midi', or 'mock'",
                config.bridge_mode
            ));
        }
        if config.bridge_address.trim().is_empty() {
            return Err("Bridge address must not be empty".into());
        }
        if config.bridge_mode == "midi"
            && config.timeout < Duration::from_millis(MIN_MIDI_TIMEOUT_MS)
        {
            return Err(format!(
                "MIDI bridge timeout must be at least {MIN_MIDI_TIMEOUT_MS} milliseconds so instance discovery can complete"
            ));
        }
        match (&config.midi_input_port, &config.midi_output_port) {
            (Some(_), None) | (None, Some(_)) => {
                return Err("--midi-input and --midi-output must be provided together".into());
            }
            (Some(_), Some(_)) if config.bridge_mode != "midi" => {
                return Err("MIDI port options require '--bridge midi'".into());
            }
            (Some(input), Some(output)) => {
                validate_midi_remote_port_names(input, output)?;
            }
            _ => {}
        }

        Ok(CliAction::Run(config))
    }

    pub fn defaults() -> Self {
        Self {
            bridge_mode: DEFAULT_BRIDGE_MODE.into(),
            bridge_address: DEFAULT_BRIDGE_ADDRESS.into(),
            midi_input_port: None,
            midi_output_port: None,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }

    pub fn help() -> &'static str {
        "Cubase MCP integration daemon\n\
         \n\
         Usage: cubase_mcp [OPTIONS]\n\
         \n\
         Options:\n\
           --bridge <tcp|midi|mock>  Bridge implementation (default: tcp)\n\
           --bridge-address <ADDR>   Local TCP bridge address (default: 127.0.0.1:8765)\n\
           --midi-input <NAME>       Existing 'From Cubase' input port (default: virtual port)\n\
           --midi-output <NAME>      Existing 'To Cubase' output port (default: virtual port)\n\
           --timeout-ms <MILLIS>     Bridge request timeout (default: 2000; MIDI minimum: 500)\n\
           --install-midi-remote     Install the bundled Cubase MIDI Remote script\n\
           --install-track-probe     Install the read-only Track API research probe\n\
           --midi-remote-root <DIR>  Exact existing 'Driver Scripts/Local' root for the probe\n\
           --list-midi-ports         List existing MIDI input/output ports as JSON\n\
           -h, --help                Print help\n\
           -V, --version             Print version\n\
         \n\
         Environment:\n\
           CUBASE_MCP_BRIDGE_MODE\n\
           CUBASE_MCP_BRIDGE_ADDRESS\n\
           CUBASE_MCP_TIMEOUT_MS\n"
    }
}

fn select_special_action(
    selected: &mut Option<SpecialAction>,
    action: SpecialAction,
    option: &str,
) -> Result<(), String> {
    if selected.is_some() {
        return Err(format!(
            "One-shot CLI actions are mutually exclusive; cannot add '{option}'"
        ));
    }
    *selected = Some(action);
    Ok(())
}

fn reject_track_probe_only_options(midi_remote_root: &Option<PathBuf>) -> Result<(), String> {
    if midi_remote_root.is_some() {
        return Err("--midi-remote-root requires --install-track-probe".into());
    }
    Ok(())
}

fn validate_midi_remote_port_names(input: &str, output: &str) -> Result<(), String> {
    if !input.contains(DEFAULT_FROM_CUBASE_PORT) {
        return Err(format!(
            "MIDI input port name must contain '{DEFAULT_FROM_CUBASE_PORT}' so the bundled Cubase MIDI Remote script can detect it"
        ));
    }
    if !output.contains(DEFAULT_TO_CUBASE_PORT) {
        return Err(format!(
            "MIDI output port name must contain '{DEFAULT_TO_CUBASE_PORT}' so the bundled Cubase MIDI Remote script can detect it"
        ));
    }
    Ok(())
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Missing value for {option}"))
}

fn next_path_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    let value = next_value(arguments, option)?;
    if value.starts_with('-') {
        return Err(format!("Missing value for {option}"));
    }
    Ok(value)
}

fn parse_timeout(value: &str) -> Result<u64, String> {
    let timeout = value
        .parse::<u64>()
        .map_err(|_| format!("Invalid timeout '{value}'; expected milliseconds as an integer"))?;
    if timeout == 0 || timeout > MAX_TIMEOUT_MS {
        return Err(format!(
            "Timeout must be between 1 and {MAX_TIMEOUT_MS} milliseconds"
        ));
    }
    Ok(timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_overrides_defaults() {
        let action = Config::parse(
            [
                "--bridge".into(),
                "mock".into(),
                "--timeout-ms".into(),
                "75".into(),
            ],
            Config::defaults(),
        )
        .unwrap();

        let CliAction::Run(config) = action else {
            panic!("expected run action");
        };
        assert_eq!(config.bridge_mode, "mock");
        assert_eq!(config.timeout, Duration::from_millis(75));
    }

    #[test]
    fn rejects_unbounded_timeout() {
        let result = Config::parse(["--timeout-ms".into(), "600001".into()], Config::defaults());
        assert!(result.is_err());
    }

    #[test]
    fn selects_track_probe_installer() {
        let root = PathBuf::from("/tmp/example/MIDI Remote/Driver Scripts/Local");
        let action = Config::parse(
            [
                "--install-track-probe".into(),
                "--midi-remote-root".into(),
                root.display().to_string(),
            ],
            Config::defaults(),
        )
        .unwrap();
        assert_eq!(
            action,
            CliAction::InstallTrackProbe(TrackProbeInstallOptions {
                midi_remote_root: Some(root),
            })
        );
    }

    #[test]
    fn rejects_track_probe_options_without_track_probe_action() {
        let arguments = vec!["--midi-remote-root".into(), "/tmp/root".into()];
        assert!(Config::parse(arguments, Config::defaults()).is_err());
    }

    #[test]
    fn rejects_combined_actions_run_options_and_trailing_unknowns() {
        for arguments in [
            vec!["--install-track-probe".into(), "--list-midi-ports".into()],
            vec![
                "--bridge".into(),
                "mock".into(),
                "--install-track-probe".into(),
            ],
            vec![
                "--install-track-probe".into(),
                "--bridge".into(),
                "mock".into(),
            ],
            vec!["--install-track-probe".into(), "--unknown".into()],
        ] {
            assert!(Config::parse(arguments, Config::defaults()).is_err());
        }
    }

    #[test]
    fn rejects_missing_track_probe_root_value() {
        let error = Config::parse(
            [
                "--install-track-probe".into(),
                "--midi-remote-root".into(),
                "--list-midi-ports".into(),
            ],
            Config::defaults(),
        )
        .unwrap_err();
        assert!(error.contains("Missing value for --midi-remote-root"));
    }

    #[test]
    fn help_lists_strict_track_probe_installer_options() {
        let help = Config::help();
        assert!(help.contains("--install-track-probe"));
        assert!(help.contains("--midi-remote-root"));
    }

    #[test]
    fn existing_string_options_may_start_with_a_hyphen() {
        let action = Config::parse(
            [
                "--bridge".into(),
                "midi".into(),
                "--midi-input".into(),
                "- Cubase MCP From Cubase".into(),
                "--midi-output".into(),
                "- Cubase MCP To Cubase".into(),
            ],
            Config::defaults(),
        )
        .unwrap();

        let CliAction::Run(config) = action else {
            panic!("expected run action");
        };
        assert_eq!(
            config.midi_input_port.as_deref(),
            Some("- Cubase MCP From Cubase")
        );
        assert_eq!(
            config.midi_output_port.as_deref(),
            Some("- Cubase MCP To Cubase")
        );
    }

    #[test]
    fn rejects_midi_timeout_too_short_for_discovery() {
        let error = Config::parse(
            [
                "--bridge".into(),
                "midi".into(),
                "--timeout-ms".into(),
                (MIN_MIDI_TIMEOUT_MS - 1).to_string(),
            ],
            Config::defaults(),
        )
        .expect_err("short MIDI timeout must be rejected");

        assert!(error.contains(&MIN_MIDI_TIMEOUT_MS.to_string()));
    }

    #[test]
    fn accepts_existing_ports_detected_by_the_bundled_remote() {
        let action = Config::parse(
            [
                "--bridge".into(),
                "midi".into(),
                "--midi-input".into(),
                format!("Driver: {DEFAULT_FROM_CUBASE_PORT} 1"),
                "--midi-output".into(),
                format!("Driver: {DEFAULT_TO_CUBASE_PORT} 1"),
            ],
            Config::defaults(),
        )
        .unwrap();

        assert!(matches!(action, CliAction::Run(_)));
    }

    #[test]
    fn rejects_existing_ports_the_bundled_remote_cannot_detect() {
        let error = Config::parse(
            [
                "--bridge".into(),
                "midi".into(),
                "--midi-input".into(),
                "Arbitrary Input".into(),
                "--midi-output".into(),
                "Arbitrary Output".into(),
            ],
            Config::defaults(),
        )
        .expect_err("arbitrary port names must be rejected");

        assert!(error.contains(DEFAULT_FROM_CUBASE_PORT));
    }
}
