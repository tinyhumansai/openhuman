// OpenHuman Meet camera bridge.
//
// Replaces the agent's outbound video stream with a programmatically
// drawn mascot. Runs at document-start in the Meet webview (installed
// via Page.addScriptToEvaluateOnNewDocument by `inject.rs`), so the
// monkey-patches on `navigator.mediaDevices.{getUserMedia,
// enumerateDevices}` are in place before Meet's app code requests the
// camera.
//
// The two `__OPENHUMAN_MASCOT_*_DATAURI__` placeholders are substituted
// from Rust at install time with `data:image/svg+xml;base64,...` URIs
// for the idle and thinking mascot SVGs, so the bridge is fully
// self-contained — no network fetch from inside meet.google.com's
// origin sandbox.
(function () {
  if (window.__openhumanCameraBridge) return;
  const TAG = '[openhuman-camera-bridge]';
  const W = 640;
  const H = 480;
  const FPS = 30;
  const TOGGLE_INTERVAL_MS = 5000;

  const MASCOTS = {
    idle: '__OPENHUMAN_MASCOT_IDLE_DATAURI__',
    thinking: '__OPENHUMAN_MASCOT_THINKING_DATAURI__',
  };

  // Mood state. `currentMood` drives every frame; `setMood` is the
  // public host-callable API and also the sink for the auto-toggle.
  let currentMood = 'idle';
  const moodImgs = { idle: null, thinking: null };

  function loadImage(src) {
    return new Promise(function (resolve, reject) {
      const img = new Image();
      img.onload = function () { resolve(img); };
      img.onerror = function (e) {
        console.warn(TAG, 'image decode failed for src head=', (src || '').slice(0, 120));
        reject(new Error('img.onerror'));
      };
      img.src = src;
    });
  }

  // Build the canvas + decode mascots once. Ready promise gates the
  // capture-stream construction so getUserMedia callers always get a
  // canvas with valid frames in flight.
  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext('2d', { alpha: false });
  // Calm, off-white background — matches Meet's tile chrome.
  ctx.fillStyle = '#F7F4EE';
  ctx.fillRect(0, 0, W, H);

  const ready = (async function () {
    try {
      moodImgs.idle = await loadImage(MASCOTS.idle);
      moodImgs.thinking = await loadImage(MASCOTS.thinking);
      console.log(TAG, 'mascots decoded',
        'idle=' + moodImgs.idle.naturalWidth + 'x' + moodImgs.idle.naturalHeight,
        'thinking=' + moodImgs.thinking.naturalWidth + 'x' + moodImgs.thinking.naturalHeight);
    } catch (err) {
      console.warn(TAG, 'mascot decode failed', err);
    }
  })();

  // setInterval-driven render loop, NOT requestAnimationFrame: the
  // meet window is frequently backgrounded behind the main openhuman
  // window during the agent flow, and Chromium throttles rAF to ~0Hz
  // in background tabs. setInterval keeps firing regardless of focus,
  // which is what we need for the outbound camera to stay live.
  // The small per-frame phase counter drives a gentle sine-wave bob
  // so the camera reads as a live feed (Meet's outbound codec drops
  // static frames, which can show up as a "frozen camera" banner).
  let frame = 0;
  function tick() {
    frame++;
    ctx.fillStyle = '#F7F4EE';
    ctx.fillRect(0, 0, W, H);
    const img = moodImgs[currentMood];
    if (img) {
      const margin = 0.12;
      const tw = W * (1 - 2 * margin);
      const th = H * (1 - 2 * margin);
      const scale = Math.min(tw / img.naturalWidth, th / img.naturalHeight);
      const bob = Math.sin(frame / (FPS * 2 / Math.PI)) * 6;
      const dw = img.naturalWidth * scale;
      const dh = img.naturalHeight * scale;
      const dx = (W - dw) / 2;
      const dy = (H - dh) / 2 + bob;
      ctx.drawImage(img, dx, dy, dw, dh);
    }
  }
  setInterval(tick, Math.round(1000 / FPS));

  // Capture-stream once both mascots are decoded; before then the
  // canvas just shows the background fill, which is fine — Meet won't
  // ask for the camera until the user is past the lobby anyway.
  const stream = canvas.captureStream(FPS);
  const fakeVideoTrack = stream.getVideoTracks()[0];
  if (fakeVideoTrack) {
    // Lie about device label so Meet's tile shows a friendly name.
    try {
      Object.defineProperty(fakeVideoTrack, 'label', {
        value: 'OpenHuman Mascot',
        configurable: true,
      });
    } catch (_) {}
  }

  // ---- monkey-patch ----------------------------------------------------
  // Important: the audio bridge (audio_bridge.js) installs its own
  // getUserMedia override BEFORE we run, and it already handles every
  // shape of constraint correctly — including audio+video, where it
  // pulls the fake-camera Y4M video and splices in its own audio. We
  // must NOT build a new MediaStream from cloned tracks: doing so
  // creates duplicate audio senders against the same destination,
  // which manifests at WebRTC negotiation as
  // "BUNDLE group contains a codec collision between [111: audio/opus]
  // and [111: audio/opus]" and breaks the Meet join flow.
  //
  // Correct shape: let the audio bridge produce the canonical stream,
  // then swap *only* the video track in place.
  const md = navigator.mediaDevices;
  if (!md) {
    console.warn(TAG, 'navigator.mediaDevices missing — cannot install bridge');
    return;
  }
  const origGetUserMedia = md.getUserMedia ? md.getUserMedia.bind(md) : null;
  if (!origGetUserMedia) {
    console.warn(TAG, 'navigator.mediaDevices.getUserMedia missing — cannot install bridge');
    return;
  }

  function wantsVideo(constraints) {
    if (!constraints) return false;
    const v = constraints.video;
    return v === true || (v && typeof v === 'object');
  }

  md.getUserMedia = async function (constraints) {
    console.log(TAG, 'getUserMedia intercepted', JSON.stringify(constraints || {}));
    if (!wantsVideo(constraints)) {
      return origGetUserMedia(constraints);
    }
    await ready;
    // Run the existing chain (audio bridge + Chromium) with the full
    // original constraints so it returns a properly assembled stream.
    const realStream = await origGetUserMedia(constraints);
    // Drop whatever video track came back (the fake-camera Y4M) and
    // splice our canvas track in. addTrack/removeTrack on a live
    // MediaStream is the supported way to mutate a stream returned
    // from getUserMedia without re-allocating it.
    try {
      realStream.getVideoTracks().forEach(function (t) {
        realStream.removeTrack(t);
        t.stop();
      });
    } catch (err) {
      console.warn(TAG, 'failed to strip original video tracks', err);
    }
    const ours = stream.getVideoTracks()[0];
    if (ours) {
      realStream.addTrack(ours.clone());
    } else {
      console.warn(TAG, 'no canvas video track available — returning audio-only');
    }
    return realStream;
  };

  // Note: we deliberately do NOT override enumerateDevices. The
  // process-level --use-fake-device-for-media-stream flag already
  // surfaces a "Fake Video Capture" device, which Meet picks by
  // default. Returning custom plain objects from enumerateDevices
  // can break Meet's device-picker code paths that expect real
  // MediaDeviceInfo instances.

  // ---- host API --------------------------------------------------------
  window.__openhumanSetMood = function (mood) {
    if (!Object.prototype.hasOwnProperty.call(MASCOTS, mood)) {
      console.warn(TAG, 'unknown mood', mood);
      return false;
    }
    if (currentMood !== mood) {
      currentMood = mood;
      console.log(TAG, 'mood ->', mood);
    }
    return true;
  };
  window.__openhumanCameraBridgeInfo = function () {
    return {
      installed: true,
      currentMood: currentMood,
      hasIdle: !!moodImgs.idle,
      hasThinking: !!moodImgs.thinking,
      frame: frame,
    };
  };

  // Default driver: toggle every 5s. Once the agent state machine wires
  // host-side `set_mood` calls, we can drop this fallback — but having
  // it on by default keeps the visible behavior even if the host loop
  // never starts.
  setInterval(function () {
    window.__openhumanSetMood(currentMood === 'idle' ? 'thinking' : 'idle');
  }, TOGGLE_INTERVAL_MS);

  window.__openhumanCameraBridge = true;
  console.log(TAG, 'installed');
})();
