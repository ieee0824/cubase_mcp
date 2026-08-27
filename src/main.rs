use std::process::ExitCode;
use std::sync::Arc;

use cubase_mcp::bridge::{CubaseBridge, MidiBridge, MockBridge, TcpBridge};
use cubase_mcp::config::{CliAction, Config};
use cubase_mcp::installer::{install_midi_remote, install_track_probe};
use cubase_mcp::mcp::McpServer;
use cubase_mcp::service::IntegrationService;

fn main() -> ExitCode {
    let config = match Config::from_process() {
        Ok(CliAction::Run(config)) => config,
        Ok(CliAction::PrintHelp) => {
            print!("{}", Config::help());
            return ExitCode::SUCCESS;
        }
        Ok(CliAction::PrintVersion) => {
            println!("cubase_mcp {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(CliAction::ListMidiPorts) => {
            return match MidiBridge::list_ports() {
                Ok(ports) => {
                    match serde_json::to_string_pretty(&ports) {
                        Ok(json) => println!("{json}"),
                        Err(error) => {
                            eprintln!("could not encode MIDI port list: {error}");
                            return ExitCode::FAILURE;
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("could not list MIDI ports: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        Ok(CliAction::InstallMidiRemote) => {
            return match install_midi_remote() {
                Ok(paths) => {
                    for path in paths {
                        println!("installed {}", path.display());
                    }
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("could not install Cubase MIDI Remote script: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        Ok(CliAction::InstallTrackProbe(options)) => {
            return match install_track_probe(&options) {
                Ok(report) => match serde_json::to_string_pretty(&report) {
                    Ok(json) => {
                        println!("{json}");
                        ExitCode::SUCCESS
                    }
                    Err(error) => {
                        eprintln!("could not encode Track Probe install report: {error}");
                        ExitCode::FAILURE
                    }
                },
                Err(error) => {
                    eprintln!("could not install Cubase Track API probe: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        Err(error) => {
            eprintln!("configuration error: {error}\n\n{}", Config::help());
            return ExitCode::from(2);
        }
    };

    let bridge: Arc<dyn CubaseBridge> = match config.bridge_mode.as_str() {
        "mock" => Arc::new(MockBridge::new()),
        "midi" => {
            let bridge = match (&config.midi_input_port, &config.midi_output_port) {
                (Some(input), Some(output)) => MidiBridge::new_with_ports(input, output),
                (None, None) => MidiBridge::new_virtual(),
                _ => unreachable!("MIDI port pair was validated by Config"),
            };
            match bridge {
                Ok(bridge) => Arc::new(bridge),
                Err(error) => {
                    eprintln!("could not initialize MIDI bridge: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        "tcp" => match TcpBridge::new(&config.bridge_address) {
            Ok(bridge) => Arc::new(bridge),
            Err(error) => {
                eprintln!("configuration error: {error}");
                return ExitCode::from(2);
            }
        },
        _ => unreachable!("bridge mode was validated by Config"),
    };

    let service = IntegrationService::new(bridge, config.timeout);
    let mut server = McpServer::new(service);

    if let Err(error) = server.serve_stdio() {
        eprintln!("MCP server stopped with an error: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
