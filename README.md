# discord-ext-native-voice

An experimental module to bring native performance to voice sending.

## Requirements

- Rust v1.45+
- The stuff in Cargo.toml, requirements.txt and requirements-dev.txt

## Installation

A working Rust compiler with `rustc` and `cargo` is required to build this package.

To build and install the package, do the following:

```bash
pip install -U -r requirements-dev.txt
pip install -U .
```

The compilation and Rust package resolution will be automatically handled by `setuptools-rust`.

## Usage

This voice implementation operates using the discord.py `VoiceProtocol` interface.

To use it, pass the class into [`VoiceChannel.connect`](https://discordpy.readthedocs.io/en/latest/api.html#discord.VoiceChannel.connect):

```python

from discord.ext.native_voice import VoiceClient

...
client = await voice_channel.connect(cls=VoiceClient)
client.play("audio.mp3")
```

The interface imitates the standard [VoiceClient](https://discordpy.readthedocs.io/en/latest/api.html#discord.VoiceClient), but it is implemented natively in Rust.

## License

MIT or Apache-2
