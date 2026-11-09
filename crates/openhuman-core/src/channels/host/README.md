# Channel host callbacks

This directory contains OpenHuman-owned operations exposed to the
TinyChannels module, plus host event subscribers. Production startup registers
the typed callback implementation with a private module runtime lease. The
callbacks use the configuration and session captured when that lease opens;
they do not construct providers or serve as a fallback when module loading
fails.

`channel_events.rs` prepares inbound context and publishes turn-state and
approval events. `remote_control.rs` answers module requests using OpenHuman's
thread store and runtime context. `adapters.rs` implements host-owned
allowlist and event operations. Voice requests use `ChannelVoiceHost` directly;
OpenHuman retains no provider host-builder or legacy trait adapter.

Module callbacks must not serve or unserve TinyBus objects while handling a
request. Runtime startup owns lease creation and shutdown, and closes the
module before releasing the captured callback state.
