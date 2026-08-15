use std::env;
use std::time::Duration;

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

pub enum CliAction {
    Run(Config),
    InstallMidiRemote,
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
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--bridge" => {
                    config.bridge_mode = next_value(&mut arguments, "--bridge")?;
                }
                "--bridge-address" => {
                    config.bridge_address = next_value(&mut arguments, "--bridge-address")?;
                }
                "--midi-input" => {
                    config.midi_input_port = Some(next_value(&mut arguments, "--midi-input")?);
                }
                "--midi-output" => {
                    config.midi_output_port = Some(next_value(&mut arguments, "--midi-output")?);
                }
                "--timeout-ms" => {
                    let value = next_value(&mut arguments, "--timeout-ms")?;
                    config.timeout = Duration::from_millis(parse_timeout(&value)?);
                }
                "--install-midi-remote" => return Ok(CliAction::InstallMidiRemote),
                "--list-midi-ports" => return Ok(CliAction::ListMidiPorts),
                "-h" | "--help" => return Ok(CliAction::PrintHelp),
                "-V" | "--version" => return Ok(CliAction::PrintVersion),
                _ => return Err(format!("Unknown argument '{argument}'")),
            }
        }

        if !matches!(config.bridge_mode.as_str(), "tcp" | "midi" | "mock") {
            return Err(format!(
                "Unsupported bridge mode '{}'; expected 'tcp', 'midi', or 'mock'",
                config.bridge_mode
            ));
        }
        if config.bridge_address.trim().is_empty() {
            return Err("Bridge address must not be empty".into());
        }
        match (&config.midi_input_port, &config.midi_output_port) {
            (Some(_), None) | (None, Some(_)) => {
                return Err("--midi-input and --midi-output must be provided together".into());
            }
            (Some(_), Some(_)) if config.bridge_mode != "midi" => {
                return Err("MIDI port options require '--bridge midi'".into());
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
           --midi-input <NAME>       Existing MIDI input port (default: virtual port)\n\
           --midi-output <NAME>      Existing MIDI output port (default: virtual port)\n\
           --timeout-ms <MILLIS>     Bridge request timeout (default: 2000)\n\
           --install-midi-remote     Install the bundled Cubase MIDI Remote script\n\
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

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Missing value for {option}"))
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
}
