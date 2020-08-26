from . import _native_voice as _native

import discord
import asyncio
import logging

from discord.backoff import ExponentialBackoff

log = logging.getLogger(__name__)

class VoiceClient(discord.VoiceProtocol):
    def __init__(self, client, channel):
        super().__init__(client, channel)
        self._connector = _native.VoiceConnector()
        self._connection = None
        if client.user is None:
            raise ValueError('Client has not been ready and has no client user set up.')
        self._connector.user_id = client.user.id
        self._voice_state_complete = asyncio.Event()
        self._voice_server_complete = asyncio.Event()
        self._attempts = 0
        self._runner = None
        self.guild = channel.guild

    async def on_voice_state_update(self, data):
        self._connector.session_id = data['session_id']

        if self._connection is not None:
            channel_id = data['channel_id']
            if channel_id is None:
                return await self.disconnect()

            self.channel = channel_id and self.guild.get_channel(int(chananel_id))
        else:
            self._voice_state_complete.set()

    async def on_voice_server_update(self, data):
        if self._voice_server_complete.is_set():
            log.info('Ignoring extraneous voice server update.')
            return

        token = data.get('token')
        server_id = data['guild_id']
        endpoint = data.get('endpoint')

        if endpoint is None or token is None:
            log.warning('Awaiting endpoint... This requires waiting. ' \
                        'If timeout occurred considering raising the timeout and reconnecting.')
            return

        endpoint, _, _ = endpoint.rpartition(':')
        if endpoint.startswith('wss://'):
            endpoint = endpoint[6:]

        self._connector.update_socket(token, server_id, endpoint)
        self._voice_server_complete.set()

    async def voice_connect(self):
        self._attempts += 1
        await self.guild.change_voice_state(channel=self.channel)

    async def voice_disconnect(self):
        log.info('The voice handshake is being terminated for Channel ID %s (Guild ID %s)', self.channel.id, self.guild.id)
        await self.guild.change_voice_state(channel=None)

    async def connect(self, *, reconnect, timeout):
        log.info('Connecting to voice...')
        self._voice_state_complete.clear()
        self._voice_server_complete.clear()

        # This has to be created before we start the flow.
        futures = [
            self._voice_state_complete.wait(),
            self._voice_server_complete.wait(),
        ]

        # Start the connection flow
        log.info('Starting voice handshake... (connection attempt %d)', self._attempts + 1)
        await self.voice_connect()

        try:
            await discord.utils.sane_wait_for(futures, timeout=timeout)
        except asyncio.TimeoutError:
            await self.disconnect(force=True)
            raise

        log.info('Voice handshake complete. Endpoint found %s', self._connector.endpoint)
        self._voice_server_complete.clear()
        self._voice_state_complete.clear()

        loop = asyncio.get_running_loop()
        self._connection = await self._connector.connect(loop)
        if self._runner is not None:
            self._runner.cancel()

        self._runner = loop.create_task(self.reconnect_handler(reconnect, timeout))

    async def reconnect_handler(self, reconnect, timeout):
        backoff = ExponentialBackoff()
        loop = asyncio.get_running_loop()

        while True:
            try:
                await self._connection.run(loop)
            except _native_voice.ConnectionClosed as e:
                log.info('Voice connection got a clean close %s', e)
                await self.disconnect()
                return
            except _native_voice.ConnectionError as e:
                log.exception('Internal voice error: %s', e)
                await self.disconnect()
                return
            except (_native_voice.ReconnectError) as e:
                if not reconnect:
                    await self.disconnect()
                    raise

                retry = backoff.delay()
                log.exception('Disconnected from voice... Reconnecting in %.2fs.', retry)

                await asyncio.sleep(retry)
                await self.voice_disconnect()
                try:
                    await self.connect(reconnect=True, timeout=timeout)
                except asyncio.TimeoutError:
                    # at this point we've retried 5 times... let's continue the loop.
                    log.warning('Could not connect to voice... Retrying...')
                    continue
            else:
                # The function above is actually a loop already
                # So if we're here then it exited normally
                await self.disconnect()
                return

    async def disconnect(self, *, force=False):
        try:
            if self._connection is not None:
                self._connection.disconnect()
                self._connection = None

            await self.voice_disconnect()
        finally:
            self.cleanup()

    def play(self, title):
        if self._connection:
            self._connection.play(title)

    def stop(self):
        if self._connection:
            self._connection.stop()

    def is_playing(self):
        if self._connection:
            return self._connection.is_playing()
        return False
