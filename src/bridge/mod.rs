mod midi;
mod tcp;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Value, json};

use crate::protocol::{BridgeError, BridgeRequest, ErrorCode};

pub use midi::{
    DEFAULT_FROM_CUBASE_PORT, DEFAULT_TO_CUBASE_PORT, MIN_MIDI_TIMEOUT_MS, MidiBridge,
    MidiPortListing,
};
pub use tcp::TcpBridge;

/// A transport-neutral Cubase bridge endpoint.
pub trait CubaseBridge: Send + Sync {
    fn call(&self, request: &BridgeRequest, timeout: Duration) -> Result<Value, BridgeError>;

    /// Reports whether the daemon currently has a live link to the bridge.
    ///
    /// This is deliberately a hint. The authoritative Cubase connection state
    /// is returned by `system.get_status`.
    fn is_connected(&self) -> bool;
}

#[derive(Debug, Clone)]
struct MockState {
    project_open: bool,
    playing: bool,
    recording: bool,
    tempo: f64,
    bars: u64,
    beats: u64,
    ticks: u64,
}

/// Deterministic in-process bridge used for development and tests.
#[derive(Debug)]
pub struct MockBridge {
    connected: AtomicBool,
    state: Mutex<MockState>,
}

impl Default for MockBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBridge {
    pub fn new() -> Self {
        Self {
            connected: AtomicBool::new(true),
            state: Mutex::new(MockState {
                project_open: true,
                playing: false,
                recording: false,
                tempo: 120.0,
                bars: 1,
                beats: 1,
                ticks: 0,
            }),
        }
    }

    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Release);
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, MockState>, BridgeError> {
        self.state
            .lock()
            .map_err(|_| BridgeError::internal("Mock bridge state lock was poisoned"))
    }

    fn status(state: &MockState) -> Value {
        json!({
            "connected": true,
            "project_open": state.project_open,
            "playing": state.playing,
            "recording": state.recording,
            "tempo": state.tempo
        })
    }

    fn transport(state: &MockState) -> Value {
        json!({
            "playing": state.playing,
            "recording": state.recording,
            "tempo": state.tempo,
            "position": {
                "bars": state.bars,
                "beats": state.beats,
                "ticks": state.ticks
            }
        })
    }

    fn capabilities() -> Value {
        json!({
            "transport": {
                "read": true,
                "write": true
            },
            "tracks": {
                "list": false,
                "select": false,
                "mute": false,
                "solo": false,
                "volume": false,
                "pan": false
            },
            "markers": false,
            "commands": false,
            "audio_analysis": false,
            "plugin_parameters": false
        })
    }
}

impl CubaseBridge for MockBridge {
    fn call(&self, request: &BridgeRequest, _timeout: Duration) -> Result<Value, BridgeError> {
        request.validate()?;
        if !self.is_connected() {
            return Err(BridgeError::not_connected(
                "The mock Cubase bridge is disconnected",
            ));
        }

        let mut state = self.lock_state()?;
        if !state.project_open && request.method.starts_with("transport.") {
            return Err(BridgeError::new(
                ErrorCode::ProjectNotOpen,
                "No Cubase project is open",
            ));
        }

        match request.method.as_str() {
            "system.get_status" => Ok(Self::status(&state)),
            "transport.play" => {
                state.playing = true;
                Ok(json!({}))
            }
            "transport.stop" => {
                state.playing = false;
                state.recording = false;
                Ok(json!({}))
            }
            "transport.record" => {
                state.playing = true;
                state.recording = true;
                Ok(json!({}))
            }
            "transport.get" => Ok(Self::transport(&state)),
            "capabilities.get" => Ok(Self::capabilities()),
            _ => Err(BridgeError::new(
                ErrorCode::NotSupported,
                format!("Bridge method '{}' is not supported", request.method),
            )),
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str, method: &str) -> BridgeRequest {
        BridgeRequest::new(id.into(), method, json!({}))
    }

    #[test]
    fn mock_transport_operations_update_state() {
        let bridge = MockBridge::new();
        bridge
            .call(&request("1", "transport.record"), Duration::from_secs(1))
            .unwrap();
        let transport = bridge
            .call(&request("2", "transport.get"), Duration::from_secs(1))
            .unwrap();
        assert_eq!(transport["playing"], true);
        assert_eq!(transport["recording"], true);

        bridge
            .call(&request("3", "transport.stop"), Duration::from_secs(1))
            .unwrap();
        let transport = bridge
            .call(&request("4", "transport.get"), Duration::from_secs(1))
            .unwrap();
        assert_eq!(transport["playing"], false);
        assert_eq!(transport["recording"], false);
    }

    #[test]
    fn disconnected_mock_returns_standard_error() {
        let bridge = MockBridge::new();
        bridge.set_connected(false);
        let error = bridge
            .call(&request("1", "transport.play"), Duration::from_secs(1))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotConnected);
    }
}
