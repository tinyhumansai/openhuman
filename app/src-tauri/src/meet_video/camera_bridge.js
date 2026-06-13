// OpenHuman Meet camera bridge.
//
// Replaces the agent's outbound video stream with a pre-rendered mascot
// frame stream produced by the main OpenHuman renderer (a hidden
// Remotion composition). Runs post-reload via Runtime.evaluate (see
// `inject.rs` for the rationale).
//
// ## Frame source: WS frame bus, with static-SVG fallback
//
// Primary: connect to `ws://127.0.0.1:<frameBusPort>` (Rust-hosted, see
// `frame_bus.rs`) and pump incoming binary JPEG frames straight onto
// our 1280x720 capture canvas. This is what the user sees in Meet.
//
// Fallback: if the WS hasn't delivered a frame in the last 500 ms (or
// the port is 0 — meaning the producer never came up), draw the
// inlined idle / thinking mascot SVGs with a gentle sine bob. Same
// behavior the bridge had before the frame bus existed; keeps Meet
// from showing a black or frozen camera if the producer crashes.
//
// The `__OPENHUMAN_*` placeholders are substituted from Rust at install
// time so the script is fully self-contained — no network fetch from
// inside meet.google.com's origin sandbox.
(function () {
  if (window.__openhumanCameraBridge) return;
  const TAG = '[openhuman-camera-bridge]';
  const W = 1280;
  const H = 720;
  const FPS = 30;
  const FRAME_BUS_PORT = __OPENHUMAN_FRAME_BUS_PORT__;
  // The static-SVG path is **cold-start only**: we use it before the
  // first remote frame arrives so the camera isn't black during the
  // ~1s producer connect handshake. Once any remote frame has been
  // seen, we keep drawing the last bitmap forever — switching back to
  // the static SVG when the producer hiccups would morph the mascot
  // visually (different artwork) and read as flicker. Drawing a stale
  // bitmap is much less jarring; if the producer truly dies the user
  // sees a frozen feed (with a tiny synthetic bob to keep the codec
  // sending), which we can detect via __openhumanCameraBridgeInfo.
  const TOGGLE_INTERVAL_MS = 5000;

  const MASCOTS = {
    idle: '__OPENHUMAN_MASCOT_IDLE_DATAURI__',
    thinking: '__OPENHUMAN_MASCOT_THINKING_DATAURI__',
  };

  // Mood drives the fallback only — the WS path renders whatever the
  // producer sends. Kept so `__openhumanSetMood` still works during
  // outages.
  let currentMood = 'idle';
  const moodImgs = { idle: null, thinking: null };

  // Latest bitmap from the WS frame bus + when it arrived. Tick loop
  // reads both atomically; ImageBitmap is cheap to draw repeatedly.
  let latestRemoteBitmap = null;
  let latestRemoteAt = 0;
  let remoteFrameCount = 0;
  let droppedOutOfOrder = 0;
  // Monotonic frame counter for out-of-order decode protection. WS
  // messages can bunch up when the kernel coalesces TCP packets, and
  // `createImageBitmap` is async — so two decodes can be in flight at
  // once and finish in arbitrary order. Without a seq, an older frame
  // can clobber a newer one and the mascot visibly rewinds.
  let nextRecvSeq = 0;
  let lastAcceptedSeq = -1;
  let wsState = 'init';
  let lastRemoteBitmapInfo = null;
  let lastDrawSource = 'cold-start';
  let lastCanvasProbe = null;
  let lastOutboundVideoStats = null;
  let lastOutboundStatsAt = 0;
  const peerConnections = new Set();

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

  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext('2d', { alpha: false });
  ctx.fillStyle = '#F7F4EE';
  ctx.fillRect(0, 0, W, H);

  // Decode the fallback SVGs eagerly so they're ready the moment the
  // WS path goes silent — the alternative is a noticeable flash of
  // background while the decoder catches up.
  const ready = (async function () {
    try {
      moodImgs.idle = await loadImage(MASCOTS.idle);
      moodImgs.thinking = await loadImage(MASCOTS.thinking);
      console.log(TAG, 'fallback mascots decoded');
    } catch (err) {
      console.warn(TAG, 'fallback mascot decode failed', err);
    }
  })();

  // ---- WS frame bus consumer ------------------------------------------
  // Exponential-ish reconnect on failure so a producer restart doesn't
  // require a full page reload to pick the camera back up.
  function connectWs() {
    if (!FRAME_BUS_PORT) {
      wsState = 'disabled';
      console.log(TAG, 'frame bus port=0, fallback-only mode');
      return;
    }
    const url = 'ws://127.0.0.1:' + FRAME_BUS_PORT;
    let ws;
    try {
      ws = new WebSocket(url);
    } catch (err) {
      console.warn(TAG, 'ws ctor failed', err);
      wsState = 'errored';
      setTimeout(connectWs, 1000);
      return;
    }
    ws.binaryType = 'arraybuffer';
    wsState = 'connecting';
    ws.onopen = function () {
      wsState = 'open';
      console.log(TAG, 'frame bus connected', url);
    };
    ws.onmessage = async function (ev) {
      if (!(ev.data instanceof ArrayBuffer)) return;
      const mySeq = ++nextRecvSeq;
      try {
        const blob = new Blob([ev.data], { type: 'image/jpeg' });
        // Decode off the main animation tick — createImageBitmap is
        // async and hands back a GPU-friendly handle for drawImage.
        const bitmap = await createImageBitmap(blob);
        // If a newer frame already won the race, drop this stale one.
        // Without this guard, bursty WS delivery + concurrent decodes
        // can cause the mascot to visibly rewind one or two frames at
        // a time — the "looks great then flickers" pattern.
        if (mySeq <= lastAcceptedSeq) {
          if (bitmap && bitmap.close) {
            try { bitmap.close(); } catch (_) {}
          }
          droppedOutOfOrder++;
          return;
        }
        if (latestRemoteBitmap && latestRemoteBitmap.close) {
          try { latestRemoteBitmap.close(); } catch (_) {}
        }
        latestRemoteBitmap = bitmap;
        latestRemoteAt = Date.now();
        lastRemoteBitmapInfo = {
          width: bitmap.width || null,
          height: bitmap.height || null,
          bytes: ev.data.byteLength || 0,
          seq: mySeq,
        };
        lastAcceptedSeq = mySeq;
        remoteFrameCount++;
      } catch (err) {
        console.warn(TAG, 'frame decode failed', err);
      }
    };
    ws.onclose = function () {
      wsState = 'closed';
      // Reconnect; the producer may simply have restarted.
      setTimeout(connectWs, 500);
    };
    ws.onerror = function (err) {
      // onclose fires after onerror — leave reconnect to onclose.
      console.warn(TAG, 'frame bus ws error', err && err.message);
    };
  }
  connectWs();

  // ---- render loop -----------------------------------------------------
  // setInterval, NOT requestAnimationFrame: Meet is frequently
  // backgrounded behind the main openhuman window during the agent
  // flow, and Chromium throttles rAF to ~0Hz in background tabs.
  // setInterval keeps firing regardless of focus, which is what we need
  // for the outbound camera to stay live.
  let frame = 0;
  function sampleCanvasPixels() {
    try {
      const cols = 7;
      const rows = 5;
      let sum = 0;
      let min = 255;
      let max = 0;
      let count = 0;
      let dark = 0;
      let bright = 0;
      for (let y = 0; y < rows; y++) {
        for (let x = 0; x < cols; x++) {
          const px = Math.max(0, Math.min(W - 1, Math.floor(((x + 0.5) * W) / cols)));
          const py = Math.max(0, Math.min(H - 1, Math.floor(((y + 0.5) * H) / rows)));
          const d = ctx.getImageData(px, py, 1, 1).data;
          const luma = Math.round((d[0] * 0.299) + (d[1] * 0.587) + (d[2] * 0.114));
          sum += luma;
          min = Math.min(min, luma);
          max = Math.max(max, luma);
          if (luma < 8) dark++;
          if (luma > 32) bright++;
          count++;
        }
      }
      lastCanvasProbe = {
        avgLuma: Math.round(sum / Math.max(1, count)),
        minLuma: min,
        maxLuma: max,
        darkSamples: dark,
        brightSamples: bright,
        sampleCount: count,
        source: lastDrawSource,
        frame: frame,
      };
    } catch (err) {
      lastCanvasProbe = { error: String((err && err.message) || err), source: lastDrawSource };
    }
  }

  function tick() {
    frame++;
    if (latestRemoteBitmap) {
      // Once any remote frame has arrived, we render only remote
      // bitmaps for the rest of the session — even if the producer
      // hiccups, holding the last bitmap is much less jarring than
      // morphing back to the static SVG. A 1px synthetic bob keeps
      // the WebRTC encoder from dropping the stream as "frozen" while
      // we're holding a stale frame.
      ctx.fillStyle = '#F7F4EE';
      ctx.fillRect(0, 0, W, H);
      const bw = latestRemoteBitmap.width || W;
      const bh = latestRemoteBitmap.height || H;
      const scale = Math.max(W / bw, H / bh);
      const dw = bw * scale;
      const dh = bh * scale;
      const dx = (W - dw) / 2;
      const dy = (H - dh) / 2 + (Math.sin(frame / (FPS * 2 / Math.PI)) * 0.5);
      ctx.drawImage(latestRemoteBitmap, dx, dy, dw, dh);
      lastDrawSource = 'remote';
      if (frame % FPS === 0) sampleCanvasPixels();
      return;
    }
    // Cold-start fallback: static SVG with a gentle bob so the camera
    // isn't black during the producer's WS handshake.
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
    lastDrawSource = 'fallback';
    if (frame % FPS === 0) sampleCanvasPixels();
  }
  setInterval(tick, Math.round(1000 / FPS));

  const stream = canvas.captureStream(FPS);
  const fakeVideoTrack = stream.getVideoTracks()[0];
  if (fakeVideoTrack) {
    try {
      Object.defineProperty(fakeVideoTrack, 'label', {
        value: 'OpenHuman Mascot',
        configurable: true,
      });
    } catch (_) {}
    try {
      fakeVideoTrack.contentHint = 'motion';
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

  function makeMascotTrack() {
    const ours = stream.getVideoTracks()[0];
    if (!ours) return null;
    const clone = ours.clone();
    try {
      Object.defineProperty(clone, 'label', {
        value: 'OpenHuman Mascot',
        configurable: true,
      });
    } catch (_) {}
    try {
      clone.contentHint = 'motion';
    } catch (_) {}
    return clone;
  }

  function isVideoTrack(track) {
    return !!track && track.kind === 'video';
  }

  function isVideoTransceiverInit(init) {
    if (!init || typeof init !== 'object') return false;
    if (Array.isArray(init.streams) && init.streams.some(function (s) {
      return s && typeof s.getVideoTracks === 'function' && s.getVideoTracks().length > 0;
    })) return true;
    return false;
  }

  function sanitizeVideoSenderInit(init) {
    if (!init || typeof init !== 'object' || !Array.isArray(init.sendEncodings)) return init;
    if (init.sendEncodings.length <= 1) return init;
    const next = Object.assign({}, init);
    const first = Object.assign({}, init.sendEncodings[0] || {});
    delete first.rid;
    delete first.scalabilityMode;
    first.scaleResolutionDownBy = 1;
    next.sendEncodings = [first];
    console.log(TAG, 'collapsed video sendEncodings to one layer for mascot');
    return next;
  }

  async function collectOutboundVideoStats() {
    const now = Date.now();
    if (now - lastOutboundStatsAt < 2000) return;
    lastOutboundStatsAt = now;
    try {
      for (const pc of peerConnections) {
        if (!pc || typeof pc.getSenders !== 'function') continue;
        const senders = pc.getSenders().filter(function (sender) {
          return sender && sender.track && sender.track.kind === 'video';
        });
        for (const sender of senders) {
          if (typeof sender.getStats !== 'function') continue;
          const report = await sender.getStats();
          report.forEach(function (stat) {
            if (stat && stat.type === 'outbound-rtp' && (stat.kind === 'video' || stat.mediaType === 'video')) {
              lastOutboundVideoStats = {
                framesEncoded: stat.framesEncoded ?? null,
                framesSent: stat.framesSent ?? null,
                bytesSent: stat.bytesSent ?? null,
                frameWidth: stat.frameWidth ?? null,
                frameHeight: stat.frameHeight ?? null,
                qualityLimitationReason: stat.qualityLimitationReason ?? null,
                timestamp: Math.round(stat.timestamp || 0),
              };
            }
          });
        }
      }
    } catch (err) {
      lastOutboundVideoStats = { error: String((err && err.message) || err) };
    }
  }

  function sampleVideoLuma(video) {
    try {
      if (!video || !video.videoWidth || !video.videoHeight || video.readyState < 2) return null;
      const c = document.createElement('canvas');
      c.width = 16;
      c.height = 9;
      const cctx = c.getContext('2d', { alpha: false });
      if (!cctx) return null;
      cctx.drawImage(video, 0, 0, c.width, c.height);
      const data = cctx.getImageData(0, 0, c.width, c.height).data;
      let sum = 0;
      let min = 255;
      let max = 0;
      for (let i = 0; i < data.length; i += 4) {
        const luma = Math.round((data[i] * 0.299) + (data[i + 1] * 0.587) + (data[i + 2] * 0.114));
        sum += luma;
        min = Math.min(min, luma);
        max = Math.max(max, luma);
      }
      const count = data.length / 4;
      return { avgLuma: Math.round(sum / Math.max(1, count)), minLuma: min, maxLuma: max };
    } catch (err) {
      return { error: String((err && err.message) || err) };
    }
  }

  function probeVideoElements() {
    try {
      return Array.prototype.slice.call(document.querySelectorAll('video'), 0, 12).map(function (video, idx) {
        const rect = video.getBoundingClientRect ? video.getBoundingClientRect() : null;
        const tracks = video.srcObject && typeof video.srcObject.getVideoTracks === 'function'
          ? video.srcObject.getVideoTracks().map(function (track) {
              let settings = {};
              try { settings = track.getSettings ? track.getSettings() : {}; } catch (_) {}
              return {
                // track.label is intentionally omitted — on real devices it
                // contains the camera/microphone device name, which is PII.
                enabled: !!track.enabled,
                muted: !!track.muted,
                readyState: track.readyState || '',
                width: settings.width || null,
                height: settings.height || null,
                frameRate: settings.frameRate || null,
              };
            })
          : [];
        return {
          idx: idx,
          videoWidth: video.videoWidth || 0,
          videoHeight: video.videoHeight || 0,
          readyState: video.readyState,
          paused: !!video.paused,
          currentTime: Math.round((video.currentTime || 0) * 1000) / 1000,
          visible: !!rect && rect.width > 0 && rect.height > 0,
          rect: rect ? {
            width: Math.round(rect.width),
            height: Math.round(rect.height),
            x: Math.round(rect.x),
            y: Math.round(rect.y),
          } : null,
          tracks: tracks,
          luma: sampleVideoLuma(video),
        };
      });
    } catch (err) {
      return [{ error: String((err && err.message) || err) }];
    }
  }

  md.getUserMedia = async function (constraints) {
    console.log(TAG, 'getUserMedia intercepted', JSON.stringify(constraints || {}));
    if (!wantsVideo(constraints)) {
      return origGetUserMedia(constraints);
    }
    await ready;
    const realStream = await origGetUserMedia(constraints);
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
      realStream.addTrack(makeMascotTrack());
    } else {
      console.warn(TAG, 'no canvas video track available — returning audio-only');
    }
    return realStream;
  };

  const NativeRTCPeerConnection = window.RTCPeerConnection || window.webkitRTCPeerConnection;
  if (NativeRTCPeerConnection && !NativeRTCPeerConnection.__openhumanCameraPatched) {
    const origAddTrack = NativeRTCPeerConnection.prototype.addTrack;
    const origAddTransceiver = NativeRTCPeerConnection.prototype.addTransceiver;
    const origGetSenders = NativeRTCPeerConnection.prototype.getSenders;

    if (origAddTrack) {
      NativeRTCPeerConnection.prototype.addTrack = function (track) {
        peerConnections.add(this);
        const args = Array.prototype.slice.call(arguments);
        if (isVideoTrack(track)) {
          const mascot = makeMascotTrack();
          if (mascot) {
            args[0] = mascot;
            console.log(TAG, 'RTCPeerConnection.addTrack video -> mascot');
          }
        }
        return origAddTrack.apply(this, args);
      };
    }

    if (origAddTransceiver) {
      NativeRTCPeerConnection.prototype.addTransceiver = function (trackOrKind, init) {
        peerConnections.add(this);
        let nextTrackOrKind = trackOrKind;
        let nextInit = init;
        const direction = init && init.direction;
        const willSend = !direction || direction === 'sendrecv' || direction === 'sendonly';
        if (willSend && (isVideoTrack(trackOrKind) || isVideoTransceiverInit(init))) {
          const mascot = makeMascotTrack();
          if (mascot) {
            nextTrackOrKind = mascot;
            nextInit = sanitizeVideoSenderInit(init);
            console.log(TAG, 'RTCPeerConnection.addTransceiver video -> mascot');
          }
        }
        return origAddTransceiver.call(this, nextTrackOrKind, nextInit);
      };
    }

    if (origGetSenders) {
      NativeRTCPeerConnection.prototype.getSenders = function () {
        peerConnections.add(this);
        return origGetSenders.apply(this, arguments);
      };
    }

    NativeRTCPeerConnection.__openhumanCameraPatched = true;
  }

  if (window.RTCRtpSender && window.RTCRtpSender.prototype && window.RTCRtpSender.prototype.replaceTrack) {
    const origReplaceTrack = window.RTCRtpSender.prototype.replaceTrack;
    if (!origReplaceTrack.__openhumanCameraPatched) {
      const patchedReplaceTrack = function (track) {
        const args = Array.prototype.slice.call(arguments);
        if (isVideoTrack(track)) {
          const mascot = makeMascotTrack();
          if (mascot) {
            args[0] = mascot;
            console.log(TAG, 'RTCRtpSender.replaceTrack video -> mascot');
          }
        }
        return origReplaceTrack.apply(this, args);
      };
      patchedReplaceTrack.__openhumanCameraPatched = true;
      window.RTCRtpSender.prototype.replaceTrack = patchedReplaceTrack;
    }
  }

  setInterval(function () {
    void collectOutboundVideoStats();
  }, 2000);

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
      frameBusPort: FRAME_BUS_PORT,
      wsState: wsState,
      remoteFrameCount: remoteFrameCount,
      droppedOutOfOrder: droppedOutOfOrder,
      remoteFreshMs: latestRemoteAt ? (Date.now() - latestRemoteAt) : null,
      lastRemoteBitmapInfo: lastRemoteBitmapInfo,
      lastDrawSource: lastDrawSource,
      canvasProbe: lastCanvasProbe,
      outboundVideoStats: lastOutboundVideoStats,
      videoTrack: fakeVideoTrack ? {
        label: fakeVideoTrack.label,
        enabled: fakeVideoTrack.enabled,
        muted: fakeVideoTrack.muted,
        readyState: fakeVideoTrack.readyState,
        settings: fakeVideoTrack.getSettings ? fakeVideoTrack.getSettings() : null,
      } : null,
      videoElements: probeVideoElements(),
    };
  };

  // Default fallback driver: toggle every 5s. Active only when the WS
  // path is silent (the tick loop ignores `currentMood` while remote
  // frames are fresh). Once the agent state machine wires real mood
  // calls we can drop this.
  setInterval(function () {
    window.__openhumanSetMood(currentMood === 'idle' ? 'thinking' : 'idle');
  }, TOGGLE_INTERVAL_MS);

  window.__openhumanCameraBridge = true;
  console.log(TAG, 'installed frame_bus_port=' + FRAME_BUS_PORT);
})();
