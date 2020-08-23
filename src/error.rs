use std::net::{TcpStream, AddrParseError};

#[derive(Debug)]
pub enum ProtocolError {
    Serde(serde_json::error::Error),
    Opus(audiopus::error::Error),
    Nacl(xsalsa20poly1305::aead::Error),
    WebSocket(tungstenite::error::Error),
    Io(std::io::Error),
    Closed(u16),
}

pub(crate) fn custom_error(text: &str) -> ProtocolError {
    let inner = std::io::Error::new(std::io::ErrorKind::Other, text);
    ProtocolError::Io(inner)
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::Serde(ref e) => e.fmt(f),
            ProtocolError::WebSocket(ref e) => e.fmt(f),
            ProtocolError::Opus(ref e) => e.fmt(f),
            ProtocolError::Nacl(ref e) => e.fmt(f),
            ProtocolError::Io(ref e) => e.fmt(f),
            ProtocolError::Closed(code) => write!(f, "WebSocket connection closed (code: {})", code),
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            ProtocolError::Serde(ref e) => Some(e),
            ProtocolError::WebSocket(ref e) => Some(e),
            ProtocolError::Opus(ref e) => Some(e),
            ProtocolError::Io(ref e) => Some(e),
            ProtocolError::Nacl(_) => None,
            ProtocolError::Closed(_) => None,
        }
    }
}

impl From<serde_json::error::Error> for ProtocolError {
    fn from(err: serde_json::error::Error) -> Self {
        Self::Serde(err)
    }
}

impl From<tungstenite::error::Error> for ProtocolError {
    fn from(err: tungstenite::error::Error) -> Self {
        Self::WebSocket(err)
    }
}

impl From<std::io::Error> for ProtocolError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<AddrParseError> for ProtocolError {
    fn from(_: AddrParseError) -> Self {
        custom_error("invalid IP address")
    }
}

impl From<native_tls::Error> for ProtocolError {
    fn from(err: native_tls::Error) -> Self {
        let inner = std::io::Error::new(std::io::ErrorKind::Other, err.to_string());
        Self::Io(inner)
    }
}

impl From<native_tls::HandshakeError<TcpStream>> for ProtocolError {
    fn from(err: native_tls::HandshakeError<TcpStream>) -> Self {
        let inner = std::io::Error::new(std::io::ErrorKind::Other, err.to_string());
        Self::Io(inner)
    }
}

impl From<audiopus::error::Error> for ProtocolError {
    fn from(err: audiopus::error::Error) -> Self {
        Self::Opus(err)
    }
}

impl From<xsalsa20poly1305::aead::Error> for ProtocolError {
    fn from(err: xsalsa20poly1305::aead::Error) -> Self {
        Self::Nacl(err)
    }
}
