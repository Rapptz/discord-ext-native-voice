#![allow(dead_code)]

use tungstenite::error::Error as TungError;
use tungstenite::protocol::{frame::coding::CloseCode, frame::CloseFrame, WebSocket};
use tungstenite::Message;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use std::io::ErrorKind;

use native_tls::{TlsConnector, TlsStream};

use crate::error::*;
use crate::payloads::*;
use crate::state::PlayingState;

pub struct DiscordVoiceProtocol {
    pub endpoint: String,
    pub endpoint_ip: String,
    user_id: String,
    server_id: String,
    pub session_id: String,
    pub token: String,
    pub recent_acks: std::collections::VecDeque<f64>,
    ws: WebSocket<TlsStream<TcpStream>>,
    close_code: u16,
    state: Arc<PlayingState>,
    socket: Option<UdpSocket>,
    pub port: u16,
    heartbeat_interval: u64,
    pub last_heartbeat: Instant,
    pub ssrc: u32,
    pub encryption: EncryptionMode,
    pub secret_key: [u8; 32],
}

pub struct ProtocolBuilder {
    endpoint: String,
    user_id: String,
    server_id: String,
    session_id: String,
    token: String,
}

impl ProtocolBuilder {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            user_id: String::new(),
            server_id: String::new(),
            session_id: String::new(),
            token: String::new(),
        }
    }

    pub fn user(&mut self, user_id: String) -> &mut Self {
        self.user_id = user_id;
        self
    }

    pub fn server(&mut self, server_id: String) -> &mut Self {
        self.server_id = server_id;
        self
    }

    pub fn session(&mut self, session_id: String) -> &mut Self {
        self.session_id = session_id;
        self
    }

    pub fn auth(&mut self, token: String) -> &mut Self {
        self.token = token;
        self
    }

    pub fn connect(self) -> Result<DiscordVoiceProtocol, ProtocolError> {
        let ws = {
            let connector = TlsConnector::new()?;
            let stream = TcpStream::connect((self.endpoint.as_str(), 443))?;
            let stream = connector.connect(&self.endpoint, stream)?;
            let mut url = String::from("wss://");
            url.push_str(self.endpoint.as_str());
            url.push_str("/?v=4");
            println!("Connecting to {:?}", &url);
            match tungstenite::client::client(&url, stream) {
                Ok((ws, _)) => ws,
                Err(e) => return Err(custom_error(e.to_string().as_str())),
            }
        };

        Ok(DiscordVoiceProtocol {
            endpoint: self.endpoint,
            user_id: self.user_id,
            server_id: self.server_id,
            session_id: self.session_id,
            token: self.token,
            recent_acks: std::collections::VecDeque::with_capacity(20),
            close_code: 0,
            ws,
            socket: None,
            heartbeat_interval: std::u64::MAX,
            port: 0,
            ssrc: 0,
            endpoint_ip: String::default(),
            encryption: EncryptionMode::default(),
            last_heartbeat: Instant::now(),
            secret_key: [0; 32],
            state: Arc::new(PlayingState::default()),
        })
    }
}

impl DiscordVoiceProtocol {
    pub fn clone_socket(&self) -> Result<UdpSocket, ProtocolError> {
        match &self.socket {
            Some(ref socket) => Ok(socket.try_clone()?),
            None => Err(custom_error("No socket found")),
        }
    }

    pub fn clone_state(&self) -> Arc<PlayingState> {
        Arc::clone(&self.state)
    }

    pub fn finish_flow(&mut self, resume: bool) -> Result<(), ProtocolError> {
        // get the op HELLO
        self.poll()?;
        if resume {
            self.resume()?;
        } else {
            self.identify()?;
        }

        while self.secret_key.iter().all(|&c| c == 0) {
            self.poll()?;
        }
        Ok(())
    }

    pub fn close(&mut self, code: u16) -> Result<(), ProtocolError> {
        self.state.disconnected();
        self.close_code = code;
        self.ws.close(Some(CloseFrame {
            code: CloseCode::from(code),
            reason: std::borrow::Cow::Owned("closing connection".to_string()),
        }))?;
        Ok(())
    }

    pub fn poll(&mut self) -> Result<(), ProtocolError> {
        if self.last_heartbeat.elapsed().as_millis() as u64 >= self.heartbeat_interval {
            self.heartbeat()?;
        }

        let msg = {
            match self.ws.read_message() {
                Err(TungError::Io(ref e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                {
                    // We'll just continue reading since we timed out?
                    return Ok(());
                }
                Err(e) => return Err(ProtocolError::from(e)),
                Ok(msg) => msg,
            }
        };

        match msg {
            Message::Text(string) => {
                let payload: RawReceivedPayload = serde_json::from_str(string.as_str())?;
                println!("Received payload: {:?}", &payload);
                match payload.op {
                    Opcode::HELLO => {
                        let payload: Hello = serde_json::from_str(payload.d.get())?;
                        let interval = payload.heartbeat_interval as u64;
                        self.heartbeat_interval = interval.min(5000);
                        // Get the original stream
                        let socket = self.ws.get_ref().get_ref();
                        socket.set_read_timeout(Some(std::time::Duration::from_millis(5000)))?;
                        self.last_heartbeat = Instant::now();
                    }
                    Opcode::READY => {
                        let payload: Ready = serde_json::from_str(payload.d.get())?;
                        self.handle_ready(payload)?;
                    }
                    Opcode::HEARTBEAT => {
                        self.heartbeat()?;
                    }
                    Opcode::HEARTBEAT_ACK => {
                        let now = Instant::now();
                        let delta = now.duration_since(self.last_heartbeat);
                        if self.recent_acks.len() == 20 {
                            self.recent_acks.pop_front();
                        }
                        self.recent_acks.push_back(delta.as_secs_f64());
                    }
                    Opcode::SESSION_DESCRIPTION => {
                        let payload: SessionDescription = serde_json::from_str(payload.d.get())?;
                        self.encryption = EncryptionMode::from_str(payload.mode.as_str())?;
                        self.secret_key = payload.secret_key;
                        self.state.connected();
                    }
                    // The rest are unhandled for now
                    _ => {}
                }
            }
            Message::Close(msg) => {
                println!("Received close frame: {:?}", &msg);
                if let Some(frame) = msg {
                    self.close_code = u16::from(frame.code);
                }
                self.state.disconnected();
                return Err(ProtocolError::Closed(self.close_code));
            }
            _ => {}
        }

        Ok(())
    }

    fn get_latency(&self) -> f64 {
        *self.recent_acks.back().unwrap_or(&f64::NAN)
    }

    fn get_average_latency(&self) -> f64 {
        if self.recent_acks.len() == 0 {
            f64::NAN
        } else {
            self.recent_acks.iter().sum::<f64>() / self.recent_acks.len() as f64
        }
    }

    fn heartbeat(&mut self) -> Result<(), ProtocolError> {
        let msg = Heartbeat::now();
        println!("Heatbeating... {:?}", &msg);
        self.ws
            .write_message(Message::text(serde_json::to_string(&msg)?))?;
        self.last_heartbeat = Instant::now();
        Ok(())
    }

    fn identify(&mut self) -> Result<(), ProtocolError> {
        let msg = Identify::new(IdentifyInfo {
            server_id: self.server_id.clone(),
            user_id: self.user_id.clone(),
            session_id: self.session_id.clone(),
            token: self.token.clone(),
        });
        println!("Identifying... {:?}", &msg);
        self.ws
            .write_message(Message::text(serde_json::to_string(&msg)?))?;
        Ok(())
    }

    fn resume(&mut self) -> Result<(), ProtocolError> {
        let msg = Resume::new(ResumeInfo {
            token: self.token.clone(),
            server_id: self.server_id.clone(),
            session_id: self.session_id.clone(),
        });
        println!("Resuming... {:?}", &msg);
        self.ws
            .write_message(Message::text(serde_json::to_string(&msg)?))?;
        Ok(())
    }

    fn handle_ready(&mut self, payload: Ready) -> Result<(), ProtocolError> {
        self.ssrc = payload.ssrc;
        self.port = payload.port;
        self.encryption = payload.get_encryption_mode()?;
        self.endpoint_ip = payload.ip;
        let addr = SocketAddr::new(
            IpAddr::V4(self.endpoint_ip.as_str().parse::<Ipv4Addr>()?),
            self.port,
        );
        println!("Address found: {:?}", &addr);
        // I'm unsure why I have to explicitly bind with Rust
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(&addr)?;
        self.socket = Some(socket);

        // attempt to do this up to 5 times
        let (ip, port) = {
            let mut retries = 0;
            loop {
                match self.udp_discovery() {
                    Ok(x) => break x,
                    Err(e) => {
                        if retries < 5 {
                            retries += 1;
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
        };

        println!("UDP discovery found: {}:{}", &ip, &port);

        // select protocol
        let to_send = SelectProtocol::from_addr(ip, port, self.encryption);
        self.ws
            .write_message(Message::text(serde_json::to_string(&to_send)?))?;
        Ok(())
    }

    fn get_socket<'a>(&'a self) -> Result<&'a UdpSocket, ProtocolError> {
        match &self.socket {
            Some(s) => Ok(s),
            None => Err(custom_error("no socket found")),
        }
    }

    fn udp_discovery(&mut self) -> Result<(String, u16), ProtocolError> {
        let socket = self.get_socket()?;
        // Generate a packet
        let mut buffer: [u8; 70] = [0; 70];
        buffer[0..2].copy_from_slice(&1u16.to_be_bytes()); // 1 = send
        buffer[2..4].copy_from_slice(&70u16.to_be_bytes()); // 70 = length
        buffer[4..8].copy_from_slice(&self.ssrc.to_be_bytes()); // the SSRC

        // rest of this is unused
        // let's send the packet
        socket.send(&buffer)?;

        // receive the new buffer
        let mut buffer: [u8; 70] = [0; 70];
        socket.recv(&mut buffer)?;

        // The IP is surrounded by 4 leading bytes and ends on the first encounter of a null byte
        let ip_end = &buffer[4..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| custom_error("could not find end of IP"))?;
        let ip: String = {
            let ip_slice = &buffer[4..4 + ip_end];
            let as_str = std::str::from_utf8(ip_slice)
                .map_err(|_| custom_error("invalid IP found (not UTF-8"))?;
            String::from(as_str)
        };
        // The port is the last 2 bytes in big endian
        // can't use regular slices with this API
        let port = u16::from_be_bytes([buffer[68], buffer[69]]);
        Ok((ip, port))
    }

    pub fn speaking(&mut self, flags: SpeakingFlags) -> Result<(), ProtocolError> {
        let msg: Speaking = Speaking::new(flags);
        self.ws
            .write_message(Message::text(serde_json::to_string(&msg)?))?;
        Ok(())
    }

    fn start_handshaking(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }
}
