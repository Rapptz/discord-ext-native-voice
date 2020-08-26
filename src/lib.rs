use pyo3::prelude::*;
use pyo3::types::{PyDict, PyBytes};
use pyo3::create_exception;

use std::thread;
use std::sync::Arc;

use parking_lot::Mutex;

pub mod protocol;
pub mod player;
pub mod payloads;
pub mod error;
pub(crate) mod state;

create_exception!(_native_voice, ReconnectError, pyo3::exceptions::Exception);
create_exception!(_native_voice, ConnectionError, pyo3::exceptions::Exception);
create_exception!(_native_voice, ConnectionClosed, pyo3::exceptions::Exception);

fn code_can_be_handled(code: u16) -> bool {
    // Non-resumable close-codes are:
    // 1000 - normal closure
    // 4014 - voice channel deleted
    // 4015 - voice server crash
    code != 1000 && code != 4014 && code != 4015
}

impl std::convert::From<error::ProtocolError> for PyErr {
    fn from(err: error::ProtocolError) -> Self {
        match err {
            error::ProtocolError::Closed(code) if code_can_be_handled(code) => {
                ReconnectError::py_err(code)
            },
            error::ProtocolError::Closed(code) => {
                ConnectionClosed::py_err(code)
            }
            _ => {
                ConnectionError::py_err(err.to_string())
            }
        }
    }
}

fn set_result(py: Python, loop_: PyObject, future: PyObject, result: PyObject) -> PyResult<()> {
    let set = future.getattr(py, "set_result")?;
    loop_.call_method1(py, "call_soon_threadsafe", (set, result))?;
    Ok(())
}

fn set_exception(py: Python, loop_: PyObject, future: PyObject, exception: PyErr) -> PyResult<()> {
    let set = future.getattr(py, "set_exception")?;
    loop_.call_method1(py, "call_soon_threadsafe", (set, exception.to_object(py)))?;
    Ok(())
}

#[pyclass]
struct VoiceConnection {
    protocol: Arc<Mutex<protocol::DiscordVoiceProtocol>>,
    player: Option<player::AudioPlayer>,
}

#[pymethods]
impl VoiceConnection {
    #[text_signature = "(loop, /)"]
    fn run(&mut self, py: Python, loop_: PyObject) -> PyResult<PyObject> {
        let (future, result): (PyObject, PyObject) = {
            let fut: PyObject = loop_.call_method0(py, "create_future")?.into();
            (fut.clone_ref(py), fut)
        };

        let proto = Arc::clone(&self.protocol);
        thread::spawn(move || {
            loop {
                let result = {
                    // TODO: consider not using locks?
                    let mut guard = proto.lock();
                    guard.poll()
                };
                if let Err(e) = result {
                    let gil = Python::acquire_gil();
                    let py = gil.python();
                    match e {
                        error::ProtocolError::Closed(code) if code_can_be_handled(code) => {
                            let _ = set_result(py, loop_, future, py.None());
                            break;
                        }
                        _ => {
                            let _ = set_exception(py, loop_, future, PyErr::from(e));
                            break;
                        }
                    }
                }
            }
        });
        Ok(result)
    }

    fn disconnect(&mut self) -> PyResult<()> {
        let mut guard = self.protocol.lock();
        guard.close(1000)?;
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(player) = &self.player {
            player.stop();
        }
    }

    fn play(&mut self, input: String) -> PyResult<()> {
        if let Some(player) = &self.player {
            player.stop();
        }

        let source = Box::new(player::FFmpegPCMAudio::new(input.as_str())?);
        let player = player::AudioPlayer::new(|error| {
            println!("Audio Player Error: {:?}", error);
        }, Arc::clone(&self.protocol), Arc::new(Mutex::new(source)));

        self.player = Some(player);
        Ok(())
    }

    fn is_playing(&self) -> bool {
        if let Some(player) = &self.player {
            player.is_playing()
        }
        else {
            false
        }
    }

    #[getter]
    fn encryption_mode(&self) -> PyResult<String> {
        let encryption = {
            let proto = self.protocol.lock();
            proto.encryption
        };
        Ok(encryption.into())
    }

    #[getter]
    fn secret_key(&self) -> PyResult<Vec<u8>> {
        let secret_key = {
            let proto = self.protocol.lock();
            proto.secret_key
        };
        Ok(secret_key.into())
    }

    fn send_playing(&self) -> PyResult<()> {
        let mut proto = self.protocol.lock();
        proto.speaking(payloads::SpeakingFlags::microphone())?;
        Ok(())
    }

    fn get_state<'py>(&self, py: Python<'py>) -> PyResult<&'py PyDict> {
        let result = PyDict::new(py);
        let proto = self.protocol.lock();
        result.set_item("secret_key", Vec::<u8>::from(proto.secret_key))?;
        result.set_item("encryption_mode", Into::<String>::into(proto.encryption))?;
        result.set_item("endpoint", proto.endpoint.clone())?;
        result.set_item("endpoint_ip", proto.endpoint_ip.clone())?;
        result.set_item("port", proto.port)?;
        result.set_item("token", proto.token.clone())?;
        result.set_item("ssrc", proto.ssrc)?;
        result.set_item("last_heartbeat", proto.last_heartbeat.elapsed().as_secs_f32())?;
        result.set_item("player_connected", self.player.is_some())?;
        Ok(result)
    }
}

#[pyclass]
struct VoiceConnector {
    #[pyo3(get, set)]
    session_id: String,
    #[pyo3(get)]
    endpoint: String,
    #[pyo3(get)]
    server_id: String,
    #[pyo3(get, set)]
    user_id: u64,
    token: String,
}

// __new__ -> VoiceConnector
// update_socket -> bool
// connect -> Future<()>
// disconnect -> None

#[pymethods]
impl VoiceConnector {
    #[new]
    fn new() -> Self {
        Self {
            session_id: String::new(),
            endpoint: String::new(),
            token: String::new(),
            server_id: String::new(),
            user_id: 0,
        }
    }

    fn update_socket(&mut self, token: String, server_id: String, endpoint: String) -> PyResult<()> {
        self.token = token;
        self.server_id = server_id;
        self.endpoint = endpoint;
        Ok(())
    }

    #[text_signature = "(loop, /)"]
    fn connect(&mut self, py: Python, loop_: PyObject) -> PyResult<PyObject> {
        let (future, result): (PyObject, PyObject) = {
            let fut: PyObject = loop_.call_method0(py, "create_future")?.into();
            (fut.clone_ref(py), fut)
        };

        let mut builder = protocol::ProtocolBuilder::new(self.endpoint.clone());
        builder.server(self.server_id.clone())
               .session(self.session_id.clone())
               .auth(self.token.clone())
               .user(self.user_id.to_string());

        thread::spawn(move || {
            let result = {
                match builder.connect() {
                    Err(e) => Err(e),
                    Ok(mut protocol) => {
                        protocol.finish_flow(false).and(Ok(protocol))
                    }
                }
            };
            let gil = Python::acquire_gil();
            let py = gil.python();
            let _ = match result {
                Err(e) => {
                    set_exception(py, loop_, future, PyErr::from(e))
                }
                Ok(protocol) => {
                    let object = VoiceConnection {
                        protocol: Arc::new(Mutex::new(protocol)),
                        player: None,
                    };
                    set_result(py, loop_, future, object.into_py(py))
                }
            };
        });
        Ok(result)
    }
}

use xsalsa20poly1305::XSalsa20Poly1305;
use xsalsa20poly1305::aead::{Aead, Buffer, AeadInPlace, NewAead, generic_array::GenericArray};

#[pyclass]
struct Debugger {
    opus: audiopus::coder::Encoder,
    cipher: XSalsa20Poly1305,
    sequence: u16,
    timestamp: u32,
    #[pyo3(get, set)]
    ssrc: u32,
    lite_nonce: u32,
}

fn get_encoder() -> Result<audiopus::coder::Encoder, error::ProtocolError> {
    let mut encoder = audiopus::coder::Encoder::new(audiopus::SampleRate::Hz48000,
        audiopus::Channels::Stereo,
  audiopus::Application::Audio)?;

    encoder.set_bitrate(audiopus::Bitrate::BitsPerSecond(128 * 1024))?;
    encoder.enable_inband_fec()?;
    encoder.set_packet_loss_perc(15)?;
    encoder.set_bandwidth(audiopus::Bandwidth::Fullband)?;
    encoder.set_signal(audiopus::Signal::Auto)?;
    Ok(encoder)
}

#[pymethods]
impl Debugger {
    #[new]
    fn new(secret_key: Vec<u8>) -> PyResult<Self> {
        let encoder = get_encoder()?;
        let key = GenericArray::clone_from_slice(secret_key.as_ref());
        let cipher = XSalsa20Poly1305::new(&key);
        Ok(Self {
            opus: encoder,
            cipher,
            sequence: 0,
            timestamp: 0,
            ssrc: 0,
            lite_nonce: 0,
        })
    }

    fn encode_opus<'py>(&self, py: Python<'py>, buffer: &PyBytes) -> PyResult<&'py PyBytes> {
        let bytes = buffer.as_bytes();
        if bytes.len() != 3840 {
            return Err(pyo3::exceptions::ValueError::py_err("byte length must be 3840 bytes"));
        }

        let as_i16: &[i16] = unsafe {
            std::slice::from_raw_parts(bytes.as_ptr() as *const i16, bytes.len() / 2)
        };

        let mut output = [0u8; 2000];
        match self.opus.encode(&as_i16, &mut output) {
            Ok(size) => Ok(PyBytes::new(py, &output[..size])),
            Err(e) => Err(pyo3::exceptions::RuntimeError::py_err(e.to_string())),
        }
    }

    fn encrypt<'py>(&self, py: Python<'py>, nonce: &PyBytes, buffer: &PyBytes) -> PyResult<&'py PyBytes> {
        let nonce = GenericArray::from_slice(nonce.as_bytes());
        match self.cipher.encrypt(nonce, buffer.as_bytes()) {
            Ok(text) => Ok(PyBytes::new(py, text.as_slice())),
            Err(_) => Err(pyo3::exceptions::RuntimeError::py_err("Could not encrypt for whatever reason"))
        }
    }

    fn prepare_packet<'py>(&mut self, py: Python<'py>, buffer: &PyBytes) -> PyResult<&'py PyBytes> {
        let bytes = buffer.as_bytes();
        if bytes.len() != 3840 {
            return Err(pyo3::exceptions::ValueError::py_err("byte length must be 3840 bytes"));
        }

        let pcm: &[i16] = unsafe {
            std::slice::from_raw_parts(bytes.as_ptr() as *const i16, bytes.len() / 2)
        };

        let mut output = [0u8; player::MAX_BUFFER_SIZE];
        let offset = match self.opus.encode(&pcm, &mut output[12..]) {
            Ok(size) => size,
            Err(e) => return Err(pyo3::exceptions::RuntimeError::py_err(e.to_string())),
        };

        self.sequence = self.sequence.wrapping_add(1);
        output[0] = 0x80;
        output[1] = 0x78;
        output[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        output[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        output[8..12].copy_from_slice(&self.ssrc.to_be_bytes());

        let mut nonce = [0u8; 24];
        nonce[0..4].copy_from_slice(&self.lite_nonce.to_be_bytes());
        let mut buffer = player::InPlaceBuffer::new(&mut output[12..], offset);
        if let Err(e) = self.cipher.encrypt_in_place(GenericArray::from_slice(&nonce), b"", &mut buffer) {
            return Err(pyo3::exceptions::RuntimeError::py_err(e.to_string()));
        }

        if let Err(e) =  buffer.extend_from_slice(&nonce) {
            return Err(pyo3::exceptions::RuntimeError::py_err(e.to_string()));
        }

        self.lite_nonce = self.lite_nonce.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(player::SAMPLES_PER_FRAME);
        let size = buffer.len();
        Ok(PyBytes::new(py, &output[0..size]))
    }
}

#[pymodule]
fn _native_voice(py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<VoiceConnection>()?;
    m.add_class::<VoiceConnector>()?;
    m.add_class::<Debugger>()?;
    m.add("ReconnectError", py.get_type::<ReconnectError>())?;
    m.add("ConnectionError", py.get_type::<ConnectionError>())?;
    m.add("ConnectionClosed", py.get_type::<ConnectionClosed>())?;
    Ok(())
}
