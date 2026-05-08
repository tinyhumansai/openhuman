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
    return;
  }
  window.__openhumanAudioBridgeInstalled = true;

  var SAMPLE_RATE = 16000;
  var ctx;
  var dest;
  var nextStartTime = 0;

  function ensureContext() {
    if (ctx) return ctx;
    try {
      ctx = new (window.AudioContext || window.webkitAudioContext)({
        sampleRate: SAMPLE_RATE,
      });
    } catch (e) {
      // Some Chromium builds don't honor the explicit sampleRate; fall
      // back to the default (the bridge will resample implicitly via
      // each AudioBuffer's declared rate).
      ctx = new (window.AudioContext || window.webkitAudioContext)();
    }
    dest = ctx.createMediaStreamDestination();
    nextStartTime = ctx.currentTime;
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
    return;
  }
  var origGum = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);

  navigator.mediaDevices.getUserMedia = function (constraints) {
    if (!constraints || !constraints.audio) {
      return origGum(constraints);
    }
    ensureContext();
    var ourStream = dest.stream;

    if (!constraints.video) {
      return Promise.resolve(ourStream);
    }
    // Combined audio + video request: pull video from the real
    // (fake-camera-backed) getUserMedia and splice in our audio track.
    return origGum({ video: constraints.video }).then(function (realStream) {
      try {
        realStream.getAudioTracks().forEach(function (t) {
          realStream.removeTrack(t);
          t.stop();
        });
      } catch (_) {}
      ourStream.getAudioTracks().forEach(function (t) {
        realStream.addTrack(t);
      });
      return realStream;
    });
  };

  // Best-effort: also patch the legacy `getUserMedia` aliases some
  // older Meet code paths still call into.
  if (typeof navigator.getUserMedia === "function") {
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
})();
