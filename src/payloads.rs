use serde::{Serialize, Deserialize};
use serde_json::value::RawValue;

use std::{str::FromStr, time::{SystemTime, UNIX_EPOCH, Instant}};
use crate::error::{custom_error, ProtocolError};

// Static typed models to convert to
// A lot of boilerplate lol

pub struct Opcode;

impl Opcode {
    pub const IDENTIFY: u8 = 0;
    pub const SELECT_PROTOCOL: u8 = 1;
    pub const READY: u8 = 2;
    pub const HEARTBEAT: u8 = 3;
    pub const SESSION_DESCRIPTION: u8 = 4;
    pub const SPEAKING: u8 = 5;
    pub const HEARTBEAT_ACK: u8 = 6;
    pub const RESUME: u8 = 7;
    pub const HELLO: u8 = 8;
    pub const RESUMED: u8 = 9;
    pub const CLIENT_CONNECT: u8 = 12;
    pub const CLIENT_DISCONNECT: u8 = 13;
}

// These are sent

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ResumeInfo {
    pub token: String,
    pub server_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Resume {
    pub op: u8,
    pub d: ResumeInfo,
}

impl Resume {
    pub fn new(info: ResumeInfo) -> Self {
        Self {
            op: Opcode::RESUME,
            d: info
        }
    }
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct IdentifyInfo {
    pub server_id: String,
    pub user_id: String,
    pub session_id: String,
    pub token: String,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Identify {
    pub op: u8,
    pub d: IdentifyInfo,
}

impl Identify {
    pub(crate) fn new(info: IdentifyInfo) -> Self {
        Self {
            op: Opcode::IDENTIFY,
            d: info,
        }
    }
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SelectProtocolInfo {
    pub address: String,
    pub port: u16,
    pub mode: String,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SelectProtocolWrapper {
    pub protocol: String,
    pub data: SelectProtocolInfo,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SelectProtocol {
    pub op: u8,
    pub d: SelectProtocolWrapper,
}

impl SelectProtocol {
    pub fn new(info: SelectProtocolInfo) -> Self {
        Self {
            op: Opcode::SELECT_PROTOCOL,
            d: SelectProtocolWrapper {
                protocol: "udp".to_string(),
                data: info,
            }
        }
    }

    pub fn from_addr(address: String, port: u16, mode: EncryptionMode) -> Self {
        Self {
            op: Opcode::SELECT_PROTOCOL,
            d: SelectProtocolWrapper {
                protocol: "udp".to_string(),
                data: SelectProtocolInfo {
                    address: address,
                    port: port,
                    mode: mode.into(),
                },
            }
        }
    }
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Heartbeat {
    op: u8,
    d: u64,
}

impl Heartbeat {
    pub fn new(instant: Instant) -> Self {
        Self {
            op: Opcode::HEARTBEAT,
            d: instant.elapsed().as_millis() as u64,
        }
    }

    pub fn now() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards");
        Self {
            op: Opcode::HEARTBEAT,
            d: now.as_millis() as u64,
        }
    }
}

// These can be received and sent

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct SpeakingFlags {
    value: u8,
}

impl Default for SpeakingFlags {
    fn default() -> Self {
        Self { value: 0 }
    }
}

impl SpeakingFlags {
    pub const MICROPHONE: u8 = 1 << 0;
    pub const SOUNDSHARE: u8 = 1 << 1;
    pub const PRIORITY: u8 = 1 << 2;

    pub fn new(value: u8) -> Self {
        Self {
            value: value
        }
    }

    pub fn off() -> Self {
        Self {
            value: 0,
        }
    }

    pub fn microphone() -> Self {
        Self {
            value: Self::MICROPHONE,
        }
    }

    pub fn soundshare() -> Self {
        Self {
            value: Self::SOUNDSHARE,
        }
    }

    pub fn priority() -> Self {
        Self {
            value: Self::PRIORITY,
        }
    }

    pub fn toggle(&mut self, value: u8) -> &mut Self {
        self.value |= value;
        self
    }
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SpeakingInfo {
    speaking: u8,
    delay: u8,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Speaking {
    op: u8,
    d: SpeakingInfo,
}

impl Speaking {
    pub fn new(flags: SpeakingFlags) -> Self {
        Self {
            op: Opcode::SPEAKING,
            d: SpeakingInfo {
                delay: 0,
                speaking: flags.value,
            }
        }
    }
}

// These are receive only

#[derive(Debug, Serialize, Deserialize)]
pub struct RawReceivedPayload<'a> {
    pub op: u8,
    #[serde(borrow)]
    pub d: &'a RawValue,
}

// This just has a data of null, so ignore it
#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Resumed;

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatAck(u64);

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SessionDescription {
    pub mode: String,
    pub secret_key: [u8; 32],
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Ready {
    pub ssrc: u32,
    pub ip: String,
    pub port: u16,
    pub modes: Vec<String>,
    #[serde(skip)]
    heartbeat_interval: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub heartbeat_interval: f64,
}

/// These are encryption modes ordered by priority
#[derive(PartialOrd, Ord, Eq, PartialEq, Copy, Clone)]
pub enum EncryptionMode {
    XSalsa20Poly1305 = 0,
    XSalsa20Poly1305Suffix = 1,
    XSalsa20Poly1305Lite = 2,
}

impl Default for EncryptionMode {
    fn default() -> Self {
        EncryptionMode::XSalsa20Poly1305
    }
}

impl Into<String> for EncryptionMode {
    fn into(self) -> String {
        match self {
            EncryptionMode::XSalsa20Poly1305 => "xsalsa20_poly1305".to_owned(),
            EncryptionMode::XSalsa20Poly1305Suffix => "xsalsa20_poly1305_suffix".to_owned(),
            EncryptionMode::XSalsa20Poly1305Lite => "xsalsa20_poly1305_lite".to_owned(),
        }
    }
}

impl FromStr for EncryptionMode {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "xsalsa20_poly1305_lite" => Ok(EncryptionMode::XSalsa20Poly1305Lite),
            "xsalsa20_poly1305_suffix" => Ok(EncryptionMode::XSalsa20Poly1305Suffix),
            "xsalsa20_poly1305" => Ok(EncryptionMode::XSalsa20Poly1305),
            _ => Err(custom_error("unknown encryption mode"))
        }
    }
}

impl Ready {
    pub fn get_encryption_mode(&self) -> Result<EncryptionMode, ProtocolError> {
        self.modes.iter()
                  .map(|s| s.parse::<EncryptionMode>())
                  .filter_map(Result::ok)
                  .max()
                  .ok_or(custom_error("No best supported encryption mode found"))
    }
}
