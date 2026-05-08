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
      img.onerror = function (e) { reject(e); };
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

  // rAF loop. We keep a small per-frame phase counter for a gentle
  // sine-wave bob so the camera reads as a live feed (Meet's outbound
  // codec drops static frames, which can manifest as a "frozen camera"
  // banner to other participants).
  let frame = 0;
  function tick() {
    frame++;
    ctx.fillStyle = '#F7F4EE';
    ctx.fillRect(0, 0, W, H);
    const img = moodImgs[currentMood];
    if (img) {
      // Fit with 12% margin so Meet's rounded tile mask doesn't crop.
      const margin = 0.12;
      const tw = W * (1 - 2 * margin);
      const th = H * (1 - 2 * margin);
      const scale = Math.min(tw / img.naturalWidth, th / img.naturalHeight);
      // Subtle bob: ±6px vertical, ~2s period.
      const bob = Math.sin(frame / (FPS * 2 / Math.PI)) * 6;
      const dw = img.naturalWidth * scale;
      const dh = img.naturalHeight * scale;
      const dx = (W - dw) / 2;
      const dy = (H - dh) / 2 + bob;
      ctx.drawImage(img, dx, dy, dw, dh);
    }
    requestAnimationFrame(tick);
  }
  requestAnimationFrame(tick);

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
  const md = navigator.mediaDevices;
  if (!md) {
    console.warn(TAG, 'navigator.mediaDevices missing — cannot install bridge');
    return;
  }
  const origGetUserMedia = md.getUserMedia ? md.getUserMedia.bind(md) : null;
  const origEnumerateDevices = md.enumerateDevices ? md.enumerateDevices.bind(md) : null;

  function wantsVideo(constraints) {
    if (!constraints) return false;
    const v = constraints.video;
    return v === true || (v && typeof v === 'object');
  }
  function wantsAudio(constraints) {
    if (!constraints) return false;
    const a = constraints.audio;
    return a === true || (a && typeof a === 'object');
  }

  md.getUserMedia = async function (constraints) {
    console.log(TAG, 'getUserMedia intercepted', JSON.stringify(constraints || {}));
    if (!wantsVideo(constraints)) {
      // Audio-only call — pass through unchanged.
      if (origGetUserMedia) return origGetUserMedia(constraints);
      throw new Error('getUserMedia(audio) not available');
    }
    await ready;
    // Build the returned stream: our video track + (optionally) the
    // user's real microphone audio, fetched via the original API.
    const tracks = [];
    const videoTrack = stream.getVideoTracks()[0];
    if (videoTrack) tracks.push(videoTrack.clone());
    if (wantsAudio(constraints) && origGetUserMedia) {
      try {
        const audioStream = await origGetUserMedia({ audio: constraints.audio });
        audioStream.getAudioTracks().forEach(function (t) { tracks.push(t); });
      } catch (err) {
        console.warn(TAG, 'real audio capture failed, returning video-only', err);
      }
    }
    return new MediaStream(tracks);
  };

  md.enumerateDevices = async function () {
    const real = origEnumerateDevices ? await origEnumerateDevices() : [];
    // Drop real cameras so Meet's device picker doesn't offer them as
    // alternatives; keep mics + speakers untouched.
    const filtered = real.filter(function (d) { return d.kind !== 'videoinput'; });
    filtered.unshift({
      deviceId: 'openhuman-mascot',
      kind: 'videoinput',
      label: 'OpenHuman Mascot',
      groupId: 'openhuman',
      toJSON: function () {
        return {
          deviceId: this.deviceId,
          kind: this.kind,
          label: this.label,
          groupId: this.groupId,
        };
      },
    });
    return filtered;
  };

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
