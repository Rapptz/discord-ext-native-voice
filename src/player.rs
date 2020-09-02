use crate::error::ProtocolError;
use crate::payloads::{EncryptionMode, SpeakingFlags};
use crate::protocol::DiscordVoiceProtocol;
use crate::state::PlayingState;

use parking_lot::Mutex;
use std::io::ErrorKind;
use std::io::Read;
use std::net::UdpSocket;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use std::process::{Child, Command, Stdio};

use rand::RngCore;
use xsalsa20poly1305::aead::Buffer;
use xsalsa20poly1305::aead::{generic_array::GenericArray, AeadInPlace, NewAead};
use xsalsa20poly1305::XSalsa20Poly1305;

pub const SAMPLING_RATE: u16 = 48000;
pub const CHANNELS: u16 = 2;
pub const FRAME_LENGTH: u16 = 20;
pub const SAMPLE_SIZE: u16 = 4; // 16-bits / 8 * channels
pub const SAMPLES_PER_FRAME: u32 = ((SAMPLING_RATE / 1000) * FRAME_LENGTH) as u32;
pub const FRAME_SIZE: u32 = SAMPLES_PER_FRAME * SAMPLE_SIZE as u32;

pub enum AudioType {
    Opus,
    Pcm,
}

pub trait AudioSource: Send {
    /// The audio type of this source
    /// If AudioType is Opus then the data will be passed as-is to discord
    fn get_type(&self) -> AudioType {
        AudioType::Pcm
    }

    /// Reads a frame of audio (20ms 16-bit stereo 48000Hz)
    /// Returns Some(num) where num is number of frames written to the buffer
    /// Returns None if the audio source has terminated
    /// This is only called if the AudioType is PCM.
    fn read_pcm_frame(&mut self, _buffer: &mut [i16]) -> Option<usize> {
        unimplemented!()
    }

    /// Same as read_pcm_frame except for opus encoded audio
    fn read_opus_frame(&mut self, _buffer: &mut [u8]) -> Option<usize> {
        unimplemented!()
    }
}

pub struct FFmpegPCMAudio {
    process: Child,
}

impl FFmpegPCMAudio {
    pub fn new(input: &str) -> Result<Self, ProtocolError> {
        let process = Command::new("ffmpeg")
            .arg("-i")
            .arg(&input)
            .args(&[
                "-f",
                "s16le",
                "-ar",
                "48000",
                "-ac",
                "2",
                "-loglevel",
                "warning",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .spawn()?;
        Ok(Self { process })
    }
}

impl AudioSource for FFmpegPCMAudio {
    fn read_pcm_frame(&mut self, buffer: &mut [i16]) -> Option<usize> {
        let stdout = self.process.stdout.as_mut().unwrap();
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, buffer.len() * 2)
        };
        stdout.read_exact(bytes).map(|_| buffer.len()).ok()
    }
}

impl Drop for FFmpegPCMAudio {
    fn drop(&mut self) {
        if let Err(e) = self.process.kill() {
            println!("Could not kill ffmpeg process: {:?}", e);
        }
    }
}

/// In order to efficiently manage a buffer we need to prepend some bytes during
/// packet creation, so a specific offset of that buffer has to modified
/// This type is a wrapper that allows me to do that.
pub struct InPlaceBuffer<'a> {
    slice: &'a mut [u8],
    length: usize,
    capacity: usize,
}

impl InPlaceBuffer<'_> {
    pub fn new<'a>(slice: &'a mut [u8], length: usize) -> InPlaceBuffer<'a> {
        InPlaceBuffer {
            capacity: slice.len(),
            slice,
            length,
        }
    }
}

impl<'a> AsRef<[u8]> for InPlaceBuffer<'a> {
    fn as_ref(&self) -> &[u8] {
        &self.slice[..self.length]
    }
}

impl<'a> AsMut<[u8]> for InPlaceBuffer<'a> {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.slice[..self.length]
    }
}

impl Buffer for InPlaceBuffer<'_> {
    fn extend_from_slice(&mut self, other: &[u8]) -> Result<(), xsalsa20poly1305::aead::Error> {
        if self.length + other.len() > self.capacity {
            Err(xsalsa20poly1305::aead::Error)
        } else {
            self.slice[self.length..self.length + other.len()].copy_from_slice(&other);
            self.length += other.len();
            Ok(())
        }
    }

    fn truncate(&mut self, len: usize) {
        // No need to drop since u8 are basic types
        if len < self.length {
            for i in self.slice[len..].iter_mut() {
                *i = 0;
            }
            self.length = len;
        }
    }

    fn len(&self) -> usize {
        self.length
    }

    fn is_empty(&self) -> bool {
        self.slice.is_empty()
    }
}

/// The maximum buffer size. 1275 is the maximum size  of an ideal Opus frame packet.
/// 24 bytes is for the nonce when constructing the audio packet
/// 12 bytes is for the header.
/// 24 bytes for the xsalsa20poly1305 nonce (again)
/// 16 bytes for the xsalsa20poly1305 tag
/// 12 extra bytes of space
pub const MAX_BUFFER_SIZE: usize = 1275 + 24 + 12 + 24 + 16 + 12;
pub const BUFFER_OFFSET: usize = 12;
type PacketBuffer = [u8; MAX_BUFFER_SIZE];

struct AudioEncoder {
    opus: audiopus::coder::Encoder,
    cipher: XSalsa20Poly1305,
    sequence: u16,
    timestamp: u32,
    lite_nonce: u32,
    ssrc: u32,
    pcm_buffer: [i16; 1920],
    // It's a re-used buffer that is used for multiple things
    // 1) The opus encoding result goes here
    // 2) The cipher is done in-place
    // 3) The final packet to send is through this buffer as well
    buffer: PacketBuffer,
    encrypter: fn(
        &XSalsa20Poly1305,
        u32,
        &[u8],
        &mut dyn Buffer,
    ) -> Result<(), xsalsa20poly1305::aead::Error>,
}

fn encrypt_xsalsa20_poly1305(
    cipher: &XSalsa20Poly1305,
    _lite: u32,
    header: &[u8],
    data: &mut dyn Buffer,
) -> Result<(), xsalsa20poly1305::aead::Error> {
    let mut nonce: [u8; 24] = [0; 24];
    nonce[0..12].copy_from_slice(&header);

    cipher.encrypt_in_place(GenericArray::from_slice(&nonce), b"", data)?;
    data.extend_from_slice(&nonce)?;
    Ok(())
}

fn encrypt_xsalsa20_poly1305_suffix(
    cipher: &XSalsa20Poly1305,
    _lite: u32,
    _header: &[u8],
    data: &mut dyn Buffer,
) -> Result<(), xsalsa20poly1305::aead::Error> {
    let mut nonce: [u8; 24] = [0; 24];
    rand::thread_rng().fill_bytes(&mut nonce);

    cipher.encrypt_in_place(GenericArray::from_slice(&nonce), b"", data)?;
    data.extend_from_slice(&nonce)?;
    Ok(())
}

fn encrypt_xsalsa20_poly1305_lite(
    cipher: &XSalsa20Poly1305,
    lite: u32,
    _header: &[u8],
    data: &mut dyn Buffer,
) -> Result<(), xsalsa20poly1305::aead::Error> {
    let mut nonce: [u8; 24] = [0; 24];
    nonce[0..4].copy_from_slice(&lite.to_be_bytes());

    cipher.encrypt_in_place(GenericArray::from_slice(&nonce), b"", data)?;
    data.extend_from_slice(&nonce[0..4])?;
    Ok(())
}

impl AudioEncoder {
    fn from_protocol(protocol: &DiscordVoiceProtocol) -> Result<Self, ProtocolError> {
        let mut encoder = audiopus::coder::Encoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Stereo,
            audiopus::Application::Audio,
        )?;

        encoder.set_bitrate(audiopus::Bitrate::BitsPerSecond(128000))?;
        encoder.enable_inband_fec()?;
        encoder.set_packet_loss_perc(15)?;
        encoder.set_bandwidth(audiopus::Bandwidth::Fullband)?;
        encoder.set_signal(audiopus::Signal::Auto)?;

        let key = GenericArray::clone_from_slice(&protocol.secret_key);
        let cipher = XSalsa20Poly1305::new(&key);

        let encrypter = match &protocol.encryption {
            EncryptionMode::XSalsa20Poly1305 => encrypt_xsalsa20_poly1305,
            EncryptionMode::XSalsa20Poly1305Suffix => encrypt_xsalsa20_poly1305_suffix,
            EncryptionMode::XSalsa20Poly1305Lite => encrypt_xsalsa20_poly1305_lite,
        };

        Ok(Self {
            opus: encoder,
            cipher,
            encrypter,
            sequence: 0,
            timestamp: 0,
            lite_nonce: 0,
            ssrc: protocol.ssrc,
            pcm_buffer: [0i16; 1920],
            buffer: [0; MAX_BUFFER_SIZE],
        })
    }

    /// Formulates the audio packet.
    /// By the time this function is called, the buffer should have the opus data
    /// already loaded at buffer[BUFFER_OFFSET..]
    /// Takes everything after BUFFER_OFFSET + `size` and encrypts it
    fn prepare_packet(&mut self, size: usize) -> Result<usize, xsalsa20poly1305::aead::Error> {
        let mut header = [0u8; BUFFER_OFFSET];
        header[0] = 0x80;
        header[1] = 0x78;
        header[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        header[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        header[8..BUFFER_OFFSET].copy_from_slice(&self.ssrc.to_be_bytes());
        self.buffer[0..BUFFER_OFFSET].copy_from_slice(&header);

        let mut buffer = InPlaceBuffer::new(&mut self.buffer[BUFFER_OFFSET..], size);
        (self.encrypter)(&self.cipher, self.lite_nonce, &header, &mut buffer)?;
        self.lite_nonce = self.lite_nonce.wrapping_add(1);
        Ok(buffer.len())
    }

    fn encode_pcm_buffer(&mut self, size: usize) -> Result<usize, audiopus::error::Error> {
        self.opus.encode(&self.pcm_buffer[..size], &mut self.buffer)
    }

    /// Sends already opus encoded data over the wire
    fn send_opus_packet(
        &mut self,
        socket: &UdpSocket,
        addr: &std::net::SocketAddr,
        size: usize,
    ) -> Result<(), ProtocolError> {
        self.sequence = self.sequence.wrapping_add(1);
        let size = self.prepare_packet(size)?;
        // println!("Sending buffer: {:?}", &self.buffer[0..size]);
        match socket.send_to(&self.buffer[0..size], addr) {
            Err(ref e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                println!(
                    "A packet has been dropped (seq: {}, timestamp: {})",
                    &self.sequence, &self.timestamp
                );
                return Ok(());
            }
            Err(e) => return Err(ProtocolError::from(e)),
            _ => {}
        };

        self.timestamp = self.timestamp.wrapping_add(SAMPLES_PER_FRAME);
        Ok(())
    }
}

type Protocol = Arc<Mutex<DiscordVoiceProtocol>>;
type Source = Arc<Mutex<Box<dyn AudioSource>>>;

#[allow(dead_code)]
pub struct AudioPlayer {
    thread: thread::JoinHandle<()>,
    protocol: Protocol,
    state: Arc<PlayingState>,
    source: Source,
}

fn audio_play_loop(
    protocol: &Protocol,
    state: &Arc<PlayingState>,
    source: &Source,
) -> Result<(), ProtocolError> {
    let mut next_iteration = Instant::now();

    let (mut encoder, mut socket) = {
        let mut proto = protocol.lock();
        proto.speaking(SpeakingFlags::microphone())?;
        (AudioEncoder::from_protocol(&*proto)?, proto.clone_socket()?)
    };

    let addr = socket.peer_addr()?;
    println!("Socket connected to: {:?}", &addr);

    loop {
        if state.is_finished() {
            break;
        }

        if state.is_paused() {
            // Wait until we're no longer paused
            state.wait_until_not_paused();
            continue;
        }

        if state.is_disconnected() {
            // Wait until we're connected again to reset our state
            state.wait_until_connected();
            next_iteration = Instant::now();

            let proto = protocol.lock();
            encoder = AudioEncoder::from_protocol(&*proto)?;
            socket = proto.clone_socket()?;
        }

        next_iteration += Duration::from_millis(20);
        let buffer_size = {
            let mut aud = source.lock();
            match aud.get_type() {
                AudioType::Opus => aud.read_opus_frame(&mut encoder.buffer[BUFFER_OFFSET..]),
                AudioType::Pcm => {
                    if let Some(num) = aud.read_pcm_frame(&mut encoder.pcm_buffer) {
                        // println!("Read {} bytes", &num);
                        match encoder.encode_pcm_buffer(num) {
                            Ok(bytes) => {
                                // println!("Encoded {} bytes", &bytes);
                                Some(bytes)
                            }
                            Err(e) => {
                                println!("Error encoding bytes: {:?}", &e);
                                return Err(e.into());
                            }
                        }
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(size) = buffer_size {
            if size != 0 {
                encoder.send_opus_packet(&socket, &addr, size)?;
                let now = Instant::now();
                next_iteration = next_iteration.max(now);
                thread::sleep(next_iteration - now);
            }
        } else {
            state.finished();
        }
    }

    Ok(())
}

impl AudioPlayer {
    pub fn new<After>(after: After, protocol: Protocol, source: Source) -> Self
    where
        After: FnOnce(Option<ProtocolError>) -> (),
        After: Send + 'static,
    {
        let state = {
            let guard = protocol.lock();
            guard.clone_state()
        };

        Self {
            protocol: Arc::clone(&protocol),
            state: Arc::clone(&state),
            source: Arc::clone(&source),
            thread: thread::spawn(move || {
                let mut current_error = None;
                if let Err(e) = audio_play_loop(&protocol, &state, &source) {
                    current_error = Some(e);
                }
                {
                    let mut proto = protocol.lock();
                    // ignore the error
                    let _ = proto.speaking(SpeakingFlags::off());
                }
                after(current_error);
            }),
        }
    }

    pub fn pause(&self) {
        self.state.paused();
    }

    pub fn resume(&self) {
        self.state.playing();
    }

    pub fn stop(&self) {
        self.state.finished()
    }

    pub fn is_paused(&self) -> bool {
        self.state.is_paused()
    }

    pub fn is_playing(&self) -> bool {
        self.state.is_playing()
    }
}
