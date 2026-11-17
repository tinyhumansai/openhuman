// The live voice handler and its disabled-feature stand-in share the same path.
#[cfg(all(feature = "voice", feature = "http-server"))]
use super::*;

#[cfg(all(feature = "voice", feature = "http-server"))]
#[test]
fn the_live_voice_socket_handler_is_the_core() {
    let _live = voice::live::ws::handle_live_voice_ws;
}
