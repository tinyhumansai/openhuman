use super::*;

// Without voice and the HTTP server the core has no socket handlers; the
// stand-ins accept and drop whatever rpc hands them.
#[cfg(not(all(feature = "voice", feature = "http-server")))]
#[tokio::test]
async fn the_voice_socket_stand_ins_drop_their_arguments() {
    voice::live::ws::handle_live_voice_ws((), (), ()).await;
    voice::streaming::handle_dictation_ws((), ()).await;
}

// With both features the real handlers are re-exported under the same paths.
#[cfg(all(feature = "voice", feature = "http-server"))]
#[test]
fn the_voice_socket_handlers_are_the_cores() {
    let _live = voice::live::ws::handle_live_voice_ws;
    let _dictation = voice::streaming::handle_dictation_ws;
}
