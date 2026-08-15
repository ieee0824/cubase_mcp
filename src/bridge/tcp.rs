use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::bridge::CubaseBridge;
use crate::protocol::{
    BRIDGE_PROTOCOL_VERSION, BridgeError, BridgeIncoming, BridgeRequest, ErrorCode,
};

const MAX_BRIDGE_MESSAGE_BYTES: u64 = 1024 * 1024;
const MAX_EVENTS_BEFORE_RESPONSE: usize = 1024;

struct TcpConnection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

enum ExchangeFailure {
    Remote(BridgeError),
    Transport(BridgeError),
}

/// Newline-delimited JSON Bridge Protocol transport over localhost TCP.
pub struct TcpBridge {
    address: SocketAddr,
    connection: Mutex<Option<TcpConnection>>,
}

impl TcpBridge {
    pub fn new(address: &str) -> Result<Self, BridgeError> {
        let address: SocketAddr = address.parse().map_err(|error| {
            BridgeError::new(
                ErrorCode::InvalidArgument,
                format!("Invalid bridge address '{address}': {error}"),
            )
        })?;
        if !address.ip().is_loopback() {
            return Err(BridgeError::new(
                ErrorCode::InvalidArgument,
                "Bridge address must use a localhost IP address",
            ));
        }

        Ok(Self {
            address,
            connection: Mutex::new(None),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    fn lock_connection(&self) -> Result<MutexGuard<'_, Option<TcpConnection>>, BridgeError> {
        self.connection
            .lock()
            .map_err(|_| BridgeError::internal("TCP bridge connection lock was poisoned"))
    }

    fn connect(&self, timeout: Duration) -> Result<TcpConnection, BridgeError> {
        let stream = TcpStream::connect_timeout(&self.address, timeout)
            .map_err(|error| map_connect_error(self.address, error))?;
        stream.set_nodelay(true).map_err(map_io_error)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(map_io_error)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(map_io_error)?;
        let reader_stream = stream.try_clone().map_err(map_io_error)?;

        Ok(TcpConnection {
            reader: BufReader::new(reader_stream),
            writer: stream,
        })
    }

    fn exchange(
        connection: &mut TcpConnection,
        request: &BridgeRequest,
        timeout: Duration,
    ) -> Result<Value, ExchangeFailure> {
        let encoded = serde_json::to_vec(request).map_err(|error| {
            ExchangeFailure::Transport(BridgeError::internal(format!(
                "Could not encode bridge request: {error}"
            )))
        })?;

        connection
            .writer
            .set_write_timeout(Some(timeout))
            .map_err(|error| ExchangeFailure::Transport(map_io_error(error)))?;
        connection
            .writer
            .write_all(&encoded)
            .and_then(|_| connection.writer.write_all(b"\n"))
            .and_then(|_| connection.writer.flush())
            .map_err(|error| ExchangeFailure::Transport(map_io_error(error)))?;

        let deadline = Instant::now() + timeout;
        let mut event_count = 0;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ExchangeFailure::Transport(BridgeError::new(
                    ErrorCode::Timeout,
                    "Timed out waiting for the Cubase bridge response",
                )));
            }
            connection
                .reader
                .get_mut()
                .set_read_timeout(Some(remaining))
                .map_err(|error| ExchangeFailure::Transport(map_io_error(error)))?;

            let mut bytes = Vec::new();
            let byte_count = (&mut connection.reader)
                .take(MAX_BRIDGE_MESSAGE_BYTES + 1)
                .read_until(b'\n', &mut bytes)
                .map_err(|error| ExchangeFailure::Transport(map_io_error(error)))?;

            if byte_count == 0 {
                return Err(ExchangeFailure::Transport(BridgeError::not_connected(
                    "Cubase bridge closed the connection",
                )));
            }
            if byte_count as u64 > MAX_BRIDGE_MESSAGE_BYTES || !bytes.ends_with(b"\n") {
                return Err(ExchangeFailure::Transport(BridgeError::protocol(
                    "Bridge message exceeds the 1 MiB limit or is not newline-delimited",
                )));
            }
            bytes.pop();
            if bytes.ends_with(b"\r") {
                bytes.pop();
            }

            let incoming: BridgeIncoming = serde_json::from_slice(&bytes).map_err(|error| {
                ExchangeFailure::Transport(BridgeError::protocol(format!(
                    "Invalid bridge response: {error}"
                )))
            })?;

            match incoming {
                BridgeIncoming::Response {
                    version,
                    id,
                    result,
                } => {
                    validate_envelope(version, &id, &request.id)
                        .map_err(ExchangeFailure::Transport)?;
                    return Ok(result);
                }
                BridgeIncoming::Error { version, id, error } => {
                    validate_envelope(version, &id, &request.id)
                        .map_err(ExchangeFailure::Transport)?;
                    if error.message.is_empty() {
                        return Err(ExchangeFailure::Transport(BridgeError::protocol(
                            "Bridge error message must not be empty",
                        )));
                    }
                    return Err(ExchangeFailure::Remote(error));
                }
                BridgeIncoming::Event {
                    version,
                    event,
                    data,
                } => {
                    if version != BRIDGE_PROTOCOL_VERSION {
                        return Err(ExchangeFailure::Transport(BridgeError::protocol(format!(
                            "Event '{}' uses unsupported bridge protocol version {}",
                            event, version
                        ))));
                    }
                    if event.is_empty() || !data.is_object() {
                        return Err(ExchangeFailure::Transport(BridgeError::protocol(
                            "Bridge event requires a non-empty name and object data",
                        )));
                    }
                    event_count += 1;
                    if event_count > MAX_EVENTS_BEFORE_RESPONSE {
                        return Err(ExchangeFailure::Transport(BridgeError::protocol(
                            "Too many bridge events arrived before the response",
                        )));
                    }
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    eprintln!(
                        "{}",
                        json!({
                            "timestamp": timestamp,
                            "event": event,
                            "source": "bridge"
                        })
                    );
                }
            }
        }
    }
}

impl CubaseBridge for TcpBridge {
    fn call(&self, request: &BridgeRequest, timeout: Duration) -> Result<Value, BridgeError> {
        request.validate()?;
        let mut connection = self.lock_connection()?;
        if connection.is_none() {
            *connection = Some(self.connect(timeout)?);
        }

        let result = Self::exchange(
            connection
                .as_mut()
                .expect("connection was initialized immediately above"),
            request,
            timeout,
        );

        match result {
            Ok(value) => Ok(value),
            Err(ExchangeFailure::Remote(error)) => Err(error),
            Err(ExchangeFailure::Transport(error)) => {
                *connection = None;
                Err(error)
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.connection
            .lock()
            .map(|connection| connection.is_some())
            .unwrap_or(false)
    }
}

fn validate_envelope(version: u32, actual_id: &str, expected_id: &str) -> Result<(), BridgeError> {
    if version != BRIDGE_PROTOCOL_VERSION {
        return Err(BridgeError::protocol(format!(
            "Unsupported bridge protocol version {version}"
        )));
    }
    if actual_id != expected_id {
        return Err(BridgeError::protocol(format!(
            "Bridge response id '{actual_id}' does not match request id '{expected_id}'"
        )));
    }
    Ok(())
}

fn map_connect_error(address: SocketAddr, error: std::io::Error) -> BridgeError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        BridgeError::new(
            ErrorCode::Timeout,
            format!("Timed out connecting to Cubase bridge at {address}"),
        )
    } else {
        BridgeError::not_connected(format!(
            "Could not connect to Cubase bridge at {address}: {error}"
        ))
    }
}

fn map_io_error(error: std::io::Error) -> BridgeError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        BridgeError::new(ErrorCode::Timeout, "Cubase bridge request timed out")
    } else {
        BridgeError::not_connected(format!("Cubase bridge I/O failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use serde_json::json;

    use super::*;
    use crate::protocol::BridgeResponse;

    #[test]
    fn tcp_bridge_ignores_events_until_matching_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: BridgeRequest = serde_json::from_str(line.trim()).unwrap();

            let mut writer = stream;
            writeln!(
                writer,
                "{}",
                json!({
                    "version": 1,
                    "type": "event",
                    "event": "tempo.changed",
                    "data": {"tempo": 128.0}
                })
            )
            .unwrap();
            writeln!(
                writer,
                "{}",
                serde_json::to_string(&BridgeResponse::new(request.id, json!({"playing": false})))
                    .unwrap()
            )
            .unwrap();
        });

        let bridge = TcpBridge::new(&address.to_string()).unwrap();
        let result = bridge
            .call(
                &BridgeRequest::new("request-1".into(), "transport.get", json!({})),
                Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(result, json!({"playing": false}));
        assert!(bridge.is_connected());
        server.join().unwrap();
    }

    #[test]
    fn tcp_bridge_rejects_non_loopback_addresses() {
        let error = TcpBridge::new("192.0.2.1:8765").err().unwrap();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
    }
}
