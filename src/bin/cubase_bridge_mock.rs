use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use cubase_mcp::bridge::{CubaseBridge, MockBridge};
use cubase_mcp::protocol::{
    BridgeError, BridgeErrorResponse, BridgeRequest, BridgeResponse, ErrorCode,
};
use serde::Serialize;

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:8765";
const MAX_BRIDGE_MESSAGE_BYTES: usize = 1024 * 1024;

fn main() -> ExitCode {
    let address = match parse_address(std::env::args().skip(1)) {
        Ok(Some(address)) => address,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("configuration error: {error}");
            return ExitCode::from(2);
        }
    };

    let listener = match TcpListener::bind(address) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("could not listen on {address}: {error}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("mock Cubase bridge listening on {address}");

    let bridge = Arc::new(MockBridge::new());
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let bridge = Arc::clone(&bridge);
                thread::spawn(move || {
                    if let Err(error) = serve_connection(stream, bridge) {
                        eprintln!("mock bridge connection ended: {error}");
                    }
                });
            }
            Err(error) => eprintln!("mock bridge accept failed: {error}"),
        }
    }

    ExitCode::SUCCESS
}

fn parse_address(
    arguments: impl IntoIterator<Item = String>,
) -> Result<Option<SocketAddr>, String> {
    let mut arguments = arguments.into_iter();
    let mut address = DEFAULT_LISTEN_ADDRESS.to_owned();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--listen" => {
                address = arguments
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| "Missing value for --listen".to_owned())?;
            }
            "-h" | "--help" => {
                println!(
                    "Cubase Bridge Protocol simulator\n\n\
                     Usage: cubase_bridge_mock [--listen <LOOPBACK_ADDRESS>]\n\n\
                     Default: {DEFAULT_LISTEN_ADDRESS}"
                );
                return Ok(None);
            }
            _ => return Err(format!("Unknown argument '{argument}'")),
        }
    }

    let address: SocketAddr = address
        .parse()
        .map_err(|error| format!("Invalid listen address '{address}': {error}"))?;
    if !address.ip().is_loopback() {
        return Err("Mock bridge must listen on a localhost interface".into());
    }
    Ok(Some(address))
}

fn serve_connection(stream: TcpStream, bridge: Arc<MockBridge>) -> io::Result<()> {
    let peer = stream.peer_addr()?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    loop {
        let mut bytes = Vec::new();
        let byte_count = reader.read_until(b'\n', &mut bytes)?;
        if byte_count == 0 {
            return Ok(());
        }
        if bytes.len() > MAX_BRIDGE_MESSAGE_BYTES {
            write_json(
                &mut writer,
                &BridgeErrorResponse::new(
                    "unknown".into(),
                    BridgeError::new(
                        ErrorCode::ProtocolError,
                        "Bridge request exceeds the 1 MiB limit",
                    ),
                ),
            )?;
            continue;
        }
        if bytes.ends_with(b"\n") {
            bytes.pop();
        }
        if bytes.ends_with(b"\r") {
            bytes.pop();
        }

        let request = match serde_json::from_slice::<BridgeRequest>(&bytes) {
            Ok(request) => request,
            Err(error) => {
                write_json(
                    &mut writer,
                    &BridgeErrorResponse::new(
                        "unknown".into(),
                        BridgeError::new(
                            ErrorCode::ProtocolError,
                            format!("Invalid bridge request: {error}"),
                        ),
                    ),
                )?;
                continue;
            }
        };
        let request_id = request.id.clone();
        let response = match request.validate() {
            Ok(()) => bridge.call(&request, Duration::from_secs(1)),
            Err(error) => Err(error),
        };

        match response {
            Ok(result) => write_json(&mut writer, &BridgeResponse::new(request_id, result))?,
            Err(error) => {
                write_json(&mut writer, &BridgeErrorResponse::new(request_id, error))?;
            }
        }
        eprintln!("mock bridge handled '{}' from {peer}", request.method);
    }
}

fn write_json(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| io::Error::other(format!("Could not encode response: {error}")))?;
    writer.write_all(b"\n")?;
    writer.flush()
}
