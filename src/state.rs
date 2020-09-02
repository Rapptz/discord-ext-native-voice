#![allow(dead_code)]
use parking_lot::{Condvar, Mutex};
// use crossbeam_channel::{bounded, Sender, Receiver};

const DISCONNECTED: u8 = 0;
const CONNECTED: u8 = 1;
const PLAYING: u8 = 2;
const PAUSED: u8 = 3;
const FINISHED: u8 = 4;

pub struct PlayingState {
    state: Mutex<u8>,
    cond: Condvar,
}

impl Default for PlayingState {
    fn default() -> Self {
        Self {
            state: Mutex::new(DISCONNECTED),
            cond: Condvar::new(),
        }
    }
}

impl PlayingState {
    pub fn is_disconnected(&self) -> bool {
        let value = self.state.lock();
        *value == DISCONNECTED
    }

    pub fn is_connected(&self) -> bool {
        let value = self.state.lock();
        *value == CONNECTED
    }

    pub fn is_playing(&self) -> bool {
        let value = self.state.lock();
        *value == PLAYING
    }

    pub fn is_paused(&self) -> bool {
        let value = self.state.lock();
        *value == PAUSED
    }

    pub fn is_finished(&self) -> bool {
        let value = self.state.lock();
        *value == FINISHED
    }

    pub fn disconnected(&self) {
        let mut guard = self.state.lock();
        *guard = DISCONNECTED;
        self.cond.notify_all();
    }

    pub fn connected(&self) {
        let mut guard = self.state.lock();
        *guard = CONNECTED;
        self.cond.notify_all();
    }

    pub fn playing(&self) {
        let mut guard = self.state.lock();
        *guard = PLAYING;
        self.cond.notify_all();
    }

    pub fn paused(&self) {
        let mut guard = self.state.lock();
        *guard = PAUSED;
        self.cond.notify_all();
    }

    pub fn finished(&self) {
        let mut guard = self.state.lock();
        *guard = FINISHED;
        self.cond.notify_all();
    }

    fn wait_until_state(&self, state: u8) {
        let mut guard = self.state.lock();
        while *guard != state {
            self.cond.wait(&mut guard);
        }
    }

    pub fn wait_until_not_paused(&self) {
        let mut guard = self.state.lock();
        while *guard == PAUSED {
            self.cond.wait(&mut guard);
        }
    }

    pub fn wait_until_disconnected(&self) {
        self.wait_until_state(DISCONNECTED);
    }

    pub fn wait_until_connected(&self) {
        self.wait_until_state(CONNECTED);
    }

    pub fn wait_until_playing(&self) {
        self.wait_until_state(PLAYING);
    }

    pub fn wait_until_paused(&self) {
        self.wait_until_state(PAUSED);
    }

    pub fn wait_until_finished(&self) {
        self.wait_until_state(FINISHED);
    }
}
