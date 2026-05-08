// OpenHuman audio bridge for the embedded Google Meet webview.
//
// Installed via CDP `Page.addScriptToEvaluateOnNewDocument` from the
// Tauri shell (`app/src-tauri/src/meet_audio/inject.rs`) so it runs at
// document-start, *before* Meet's join page calls
// `navigator.mediaDevices.getUserMedia`. The shell then triggers a
// `Page.reload` so that even an already-navigated meet page picks up
// the override.
//
// What this script does:
//
// 1. Builds a 16 kHz mono Web-Audio graph whose
//    `MediaStreamAudioDestinationNode` provides an audio MediaStream
//    track the page can hand to its RTCPeerConnection.
// 2. Monkey-patches `navigator.mediaDevices.getUserMedia` so any audio
//    request returns our destination stream (and combined audio+video
//    requests get the real video track from Chromium's fake-camera Y4M
//    plus our audio track).
// 3. Exposes `window.__openhumanFeedPcm(b64)` — the Tauri shell calls
//    this on a ~100 ms cadence via CDP `Runtime.evaluate` to push the
//    next chunk of synthesized PCM16LE bytes from
//    `openhuman.meet_agent_poll_speech`.
//
// JS-injection note: the project's broader rule (CLAUDE.md) is "no new
// JS in embedded provider webviews". The Meet call window is a special
// case — it is a dedicated top-level window for a single audio-bridging
// purpose where the public `CefAudioHandler` API is sufficient for the
// listen path but Chromium's audio *input* path has no comparable
// public hook short of a from-source rebuild. The user has explicitly
// authorized this injection for the speak path; legacy provider
// webviews keep the no-JS rule.

(function () {
  if (window.__openhumanAudioBridgeInstalled) {
    console.log("[openhuman-audio-bridge] already installed; skipping");
    return;
  }
  window.__openhumanAudioBridgeInstalled = true;
  console.log("[openhuman-audio-bridge] install begin");

  var SAMPLE_RATE = 16000;
  var ctx;
  var dest;
  var nextStartTime = 0;

  function ensureContext() {
    if (ctx) {
      console.log(
        "[openhuman-audio-bridge] reuse AudioContext state=" + ctx.state
      );
      return ctx;
    }
    var requestedRate = SAMPLE_RATE;
    try {
      ctx = new (window.AudioContext || window.webkitAudioContext)({
        sampleRate: SAMPLE_RATE,
      });
    } catch (e) {
      // Some Chromium builds don't honor the explicit sampleRate; fall
      // back to the default (the bridge will resample implicitly via
      // each AudioBuffer's declared rate).
      console.warn(
        "[openhuman-audio-bridge] AudioContext sampleRate hint rejected; falling back to default rate err=" +
          e
      );
      ctx = new (window.AudioContext || window.webkitAudioContext)();
    }
    dest = ctx.createMediaStreamDestination();
    nextStartTime = ctx.currentTime;
    console.log(
      "[openhuman-audio-bridge] AudioContext created requested_rate=" +
        requestedRate +
        " actual_rate=" +
        ctx.sampleRate +
        " state=" +
        ctx.state
    );
    return ctx;
  }

  function decodeBase64Pcm16leToFloat32(b64) {
    var bin = atob(b64);
    var len = bin.length;
    if (len % 2 !== 0) {
      // Trailing byte = corrupt frame; drop it rather than read past
      // the end and emit a click.
      len = len - 1;
    }
    var out = new Float32Array(len / 2);
    for (var i = 0, j = 0; j < len; i++, j += 2) {
      var lo = bin.charCodeAt(j);
      var hi = bin.charCodeAt(j + 1);
      var v = (hi << 8) | lo;
      if (v & 0x8000) v -= 0x10000;
      out[i] = v / 32768;
    }
    return out;
  }

  // Public push API. Returns the duration in seconds the chunk added
  // to the queue, mostly for diagnostics; the shell ignores it.
  window.__openhumanFeedPcm = function (b64) {
    if (!b64) return 0;
    try {
      ensureContext();
      var samples = decodeBase64Pcm16leToFloat32(b64);
      if (!samples.length) return 0;
      var buffer = ctx.createBuffer(1, samples.length, SAMPLE_RATE);
      buffer.copyToChannel(samples, 0, 0);
      var src = ctx.createBufferSource();
      src.buffer = buffer;
      src.connect(dest);
      // Schedule strictly after the previous chunk so successive
      // 100 ms feeds line up gaplessly. If the queue has emptied
      // (caller fell behind), restart at currentTime so we don't try
      // to play in the past.
      if (nextStartTime < ctx.currentTime) {
        nextStartTime = ctx.currentTime;
      }
      src.start(nextStartTime);
      nextStartTime += buffer.duration;
      // High-frequency log gated by a counter so we don't drown the
      // console at 10 Hz; emit ~1 in 50 frames (~5 s cadence at the
      // shell's 100 ms feed rate).
      window.__openhumanFeedCounter = (window.__openhumanFeedCounter || 0) + 1;
      if (window.__openhumanFeedCounter % 50 === 1) {
        console.log(
          "[openhuman-audio-bridge] feed sampled chunk_dur=" +
            buffer.duration.toFixed(3) +
            "s queue_ahead=" +
            (nextStartTime - ctx.currentTime).toFixed(3) +
            "s frame=" +
            window.__openhumanFeedCounter
        );
      }
      return buffer.duration;
    } catch (e) {
      console.warn("[openhuman-audio-bridge] feed failed:", e);
      return 0;
    }
  };

  // Public introspection — useful from the shell side via
  // Runtime.evaluate to confirm the bridge is alive.
  window.__openhumanAudioBridgeInfo = function () {
    return {
      installed: true,
      sample_rate: SAMPLE_RATE,
      audio_context_state: ctx ? ctx.state : "not-created",
      next_start_time: nextStartTime,
      destination_track_count: dest ? dest.stream.getAudioTracks().length : 0,
    };
  };

  // Override getUserMedia so Meet's audio requests are served from our
  // bridge stream. We delegate video to the original implementation so
  // Chromium's fake-camera Y4M (mascot) keeps working.
  if (
    !navigator.mediaDevices ||
    typeof navigator.mediaDevices.getUserMedia !== "function"
  ) {
    console.warn(
      "[openhuman-audio-bridge] navigator.mediaDevices.getUserMedia missing; interception disabled"
    );
    return;
  }
  var origGum = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);

  // Build a fresh audio MediaStream backed by clones of the bridge's
  // destination tracks. Returning the singleton `dest.stream` directly
  // would let any caller's `track.stop()` (e.g. Meet during preview
  // teardown / track renegotiation) permanently kill the bridge. Each
  // call gets its own track lifecycle.
  function freshAudioStream() {
    ensureContext();
    return new MediaStream(
      dest.stream.getAudioTracks().map(function (t) {
        return t.clone();
      })
    );
  }

  navigator.mediaDevices.getUserMedia = function (constraints) {
    if (!constraints || !constraints.audio) {
      console.log(
        "[openhuman-audio-bridge] getUserMedia passthrough (no audio)"
      );
      return origGum(constraints);
    }

    if (!constraints.video) {
      console.log(
        "[openhuman-audio-bridge] getUserMedia intercepted audio-only"
      );
      return Promise.resolve(freshAudioStream());
    }
    // Combined audio + video request: pull video from the real
    // (fake-camera-backed) getUserMedia and splice in fresh clones of
    // our audio tracks.
    console.log(
      "[openhuman-audio-bridge] getUserMedia intercepted audio+video; splicing audio onto fake-camera stream"
    );
    return origGum({ video: constraints.video }).then(function (realStream) {
      try {
        realStream.getAudioTracks().forEach(function (t) {
          realStream.removeTrack(t);
          t.stop();
        });
      } catch (_) {}
      freshAudioStream()
        .getAudioTracks()
        .forEach(function (t) {
          realStream.addTrack(t);
        });
      return realStream;
    });
  };

  // Best-effort: also patch the legacy `getUserMedia` aliases some
  // older Meet code paths still call into.
  if (typeof navigator.getUserMedia === "function") {
    console.log("[openhuman-audio-bridge] patching legacy navigator.getUserMedia");
    var origLegacy = navigator.getUserMedia.bind(navigator);
    navigator.getUserMedia = function (constraints, success, failure) {
      navigator.mediaDevices
        .getUserMedia(constraints)
        .then(success, failure)
        .catch(function (e) {
          if (failure) failure(e);
          else origLegacy(constraints, success, failure);
        });
    };
  }
  console.log("[openhuman-audio-bridge] install complete");
})();
