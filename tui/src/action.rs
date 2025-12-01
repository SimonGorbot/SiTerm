use serde::{Deserialize, Serialize};
use strum::Display;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceMessage {
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Display, Serialize, Deserialize)]
pub enum Action {
    Tick,
    Render,
    Resize(u16, u16),
    Suspend,
    Resume,
    Quit,
    ClearScreen,
    Error(String),
    ShowPreconnect,
    ShowConnecting,
    ShowMain,
    ShowError(String),
    RefreshPorts,
    PortsUpdated(Vec<String>),
    Connect { port: String, baud_rate: u32 },
    ConnectionEstablished { port: String, baud_rate: u32 },
    ConnectionFailed(String),
    SendCommand(String),
    CommandSent(String),
    IncomingMessage(DeviceMessage),
}
