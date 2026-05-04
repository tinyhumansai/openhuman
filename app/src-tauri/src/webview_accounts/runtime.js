// OpenHuman webview-accounts recipe runtime.
// Injected via WebviewBuilder.initialization_script BEFORE page JS runs.
// Exposes a small `window.__openhumanRecipe` API per-provider recipes use
// to scrape the DOM and pipe state back to Rust.
//
// Runs in the loaded service's origin (e.g. https://mail.google.com).
// IPC back to Rust uses Tauri's `window.__TAURI_INTERNALS__.invoke`,
// which Tauri auto-injects into every webview it controls (including
// child webviews on external origins).
//
// Event kinds emitted to Rust via `webview_recipe_event`:
//   log        { level, msg }
//   ingest     { messages, unread?, snapshotKey? }      (recipe-driven)
//   <custom>   arbitrary — recipes push via api.emit(kind, payload)
//
// NOTE: only injected for providers that still need a JS bridge (gmail,
// linkedin, google-meet). The migrated providers (whatsapp, telegram,
// slack, discord, browserscan) load with ZERO injected JS under cef —
// their scraping runs natively via CDP in the per-provider scanner
// modules. WebSocket interception lives in the Rust-side CDP Network
// listener (see `discord_scanner/mod.rs`), not here.
//
// Browser push notifications are intercepted natively in the CEF render
// process by `cef-helper`'s NotifyV8Handler, which replaces
// window.Notification + ServiceWorkerRegistration.prototype.showNotification
// with V8 native bindings (see the tauri-cef fork).
(function () {
  if (window.__openhumanRecipe) return;

  const ctx = window.__OPENHUMAN_RECIPE_CTX__ || { accountId: 'unknown', provider: 'unknown' };
  const POLL_MS = 2000;

  function rawInvoke(cmd, payload) {
    try {
      const inv = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
      if (typeof inv !== 'function') return Promise.resolve();
      return inv(cmd, payload || {});
    } catch (e) {
      // swallow — never let a bad invoke break the host page
      return Promise.resolve();
    }
  }

  function send(kind, payload) {
    return rawInvoke('webview_recipe_event', {
      args: {
        account_id: ctx.accountId,
        provider: ctx.provider,
        kind: kind,
        payload: payload || {},
        ts: Date.now(),
      },
    });
  }

  let loopFn = null;
  let pollTimer = null;

  function safeRunLoop() {
    if (!loopFn) return;
    try {
      loopFn(api);
    } catch (e) {
      send('log', { level: 'warn', msg: '[recipe] loop threw: ' + (e && e.message ? e.message : String(e)) });
    }
  }

  const api = {
    loop(fn) {
      loopFn = fn;
      if (pollTimer) clearInterval(pollTimer);
      pollTimer = setInterval(safeRunLoop, POLL_MS);
      // also kick once on next tick so we don't wait POLL_MS for the first call
      setTimeout(safeRunLoop, 250);
      send('log', { level: 'info', msg: '[recipe] loop registered, polling every ' + POLL_MS + 'ms' });
    },
    ingest(payload) {
      // payload: { messages: Array<{id?, from?, body, ts?}>, unread?, snapshotKey? }
      send('ingest', payload || {});
    },
    log(level, msg) {
      send('log', { level: level || 'info', msg: String(msg) });
    },
    /** Push an arbitrary event kind up to Rust. Recipe-specific events
     *  (e.g. `meet_call_started`) go through here — the host side just
     *  sees another `webview:event` envelope with the given `kind`. */
    emit(kind, payload) {
      if (!kind) return;
      send(String(kind), payload || {});
    },
    context() {
      return Object.assign({}, ctx);
    },
  };

  window.__openhumanRecipe = api;
  send('log', { level: 'info', msg: '[recipe-runtime] ready provider=' + ctx.provider + ' accountId=' + ctx.accountId });

  // --- #713 getDisplayMedia shim ---
  //
  // Background: embedded webviews run under CEF Alloy, which does not link
  // Chromium's DesktopMediaPicker. Without an interceptor, `getDisplayMedia`
  // gets auto-granted by our permission handler and Chromium silently picks
  // the primary display (issue #713 AC2: "OS screen/window picker appears").
  //
  // The picker UI is injected DIRECTLY into the child webview's own DOM
  // rather than rendered as a React modal in the main OpenHuman window.
  // Two reasons:
  //   (a) Works uniformly for every embedded provider — Meet, Slack
  //       Huddles, Discord, Zoom — without per-provider host-side glue.
  //   (b) Dodges the CEF native-view stacking problem: a React modal in
  //       the main window is always occluded by the child webview's
  //       NSView, forcing a hide/bounds dance that flickers the embedded
  //       site. An overlay inside the page is stacked in the page's own
  //       compositing context, so it sits above Meet/Slack UI naturally.
  //
  // Flow:
  //   1. Shim calls Tauri `screen_share_list_sources` to enumerate real
  //      screens (`screen:<CGDirectDisplayID>:0`) and windows
  //      (`window:<CGWindowID>:0`) natively.
  //   2. Shim builds a fixed-position picker overlay inside the page's
  //      document and awaits the user's choice.
  //   3. On Share, shim calls `getUserMedia` with a hand-crafted
  //      `chromeMediaSource: 'desktop' + chromeMediaSourceId` constraint.
  //      Stage 0 PoC proved Chromium honours the ID directly because our
  //      CEF permission callback grants `DESKTOP_VIDEO_CAPTURE` bits.
  //   4. On Cancel, shim throws `NotAllowedError` — same shape the real
  //      Chromium picker emits so page error handling is unchanged.
  (function installGetDisplayMediaShim() {
    if (!navigator.mediaDevices || typeof navigator.mediaDevices.getDisplayMedia !== 'function') {
      // Never had getDisplayMedia to begin with (non-WebRTC webview); skip.
      return;
    }
    if (navigator.mediaDevices.__ohGdmShimInstalled) return;

    // `navigator.mediaDevices.getDisplayMedia` is a WebIDL-defined prototype
    // method on `MediaDevices.prototype`. Chromium marks it
    // `writable: true, configurable: true` but *only* on the prototype —
    // plain `navigator.mediaDevices.getDisplayMedia = ...` on the instance
    // creates an own-property shadow that Chromium's IDL bindings bypass
    // when the page actually invokes the method. We override on the
    // prototype with `defineProperty` so the shim is what runs for every
    // MediaDevices instance in this page (including any iframes that
    // inherit from the same prototype).
    const proto = Object.getPrototypeOf(navigator.mediaDevices);
    const descriptor = Object.getOwnPropertyDescriptor(proto, 'getDisplayMedia');
    const origGetDisplayMedia = (descriptor && descriptor.value
      ? descriptor.value
      : navigator.mediaDevices.getDisplayMedia
    ).bind(navigator.mediaDevices);

    // Fire-and-forget session cleanup. Swallows errors because finalize
    // is a no-op on the host side for unknown/expired tokens and we don't
    // want a late IPC failure to leak into the getDisplayMedia rejection.
    function finalizeSessionQuiet(token, pickedId) {
      if (!token) return Promise.resolve();
      return rawInvoke('screen_share_finalize_session', {
        args: { token: token, pickedId: pickedId || null },
      }).catch(function () {});
    }

    // In-flight guard (graycyrus refactor #6). The host-side state already
    // evicts a stale session when begin_session fires twice, but without a
    // shim-side guard a second call would still append a second picker DOM
    // while the first is open — the user would see two stacked overlays.
    // Reject a concurrent call the same way the MediaStreams spec does
    // when an existing capture request is in progress.
    let pickerInFlight = false;

    const shim = async function (constraints) {
      constraints = constraints || {};
      if (pickerInFlight) {
        send('log', { level: 'warn', msg: '[gdm-shim] picker already open, rejecting concurrent call' });
        throw new DOMException(
          'A screen-share picker is already open',
          'InvalidStateError'
        );
      }
      pickerInFlight = true;
      try {
        return await runShim(constraints);
      } finally {
        pickerInFlight = false;
      }
    };

    const runShim = async function (constraints) {
      constraints = constraints || {};
      // User-activation gate (#812). `navigator.userActivation.isActive`
      // is transient — true only during the direct call stack of a real
      // gesture handler (click, key, touch). Third-party JS calling
      // getDisplayMedia from a timer or async continuation gets filtered
      // here, so our downstream commands (begin_session etc.) never open
      // a session without a gesture. Fall through to the original
      // implementation rather than throw so pages with legitimate
      // non-gesture flows (rare but possible) aren't hard-blocked.
      const hasActivation = !!(
        typeof navigator !== 'undefined' &&
        navigator.userActivation &&
        navigator.userActivation.isActive
      );
      send('log', {
        level: 'info',
        msg:
          '[gdm-shim] getDisplayMedia intercepted audio=' +
          !!constraints.audio +
          ' activation=' +
          hasActivation,
      });
      if (!hasActivation) {
        send('log', {
          level: 'warn',
          msg: '[gdm-shim] no user activation, falling through to native getDisplayMedia',
        });
        return origGetDisplayMedia(constraints);
      }

      let session;
      try {
        session = await rawInvoke('screen_share_begin_session', {
          args: {
            accountId: ctx.accountId,
            origin: (typeof location !== 'undefined' && location.origin) || 'unknown',
            hasUserActivation: hasActivation,
          },
        });
      } catch (e) {
        send('log', {
          level: 'error',
          msg: '[gdm-shim] begin_session IPC failed: ' + (e && e.message ? e.message : String(e)),
        });
        return origGetDisplayMedia(constraints);
      }
      if (!session || typeof session.token !== 'string' || !Array.isArray(session.sources)) {
        send('log', {
          level: 'warn',
          msg: '[gdm-shim] begin_session returned malformed payload, falling back',
        });
        return origGetDisplayMedia(constraints);
      }
      const sessionToken = session.token;
      const sources = session.sources;
      if (sources.length === 0) {
        send('log', { level: 'warn', msg: '[gdm-shim] no sources enumerated, falling back' });
        await finalizeSessionQuiet(sessionToken, null);
        return origGetDisplayMedia(constraints);
      }

      const pick = await showInPagePicker(sources, sessionToken);
      if (!pick) {
        send('log', { level: 'info', msg: '[gdm-shim] user cancelled picker' });
        await finalizeSessionQuiet(sessionToken, null);
        // Meet (and other video-conf sites) treat `NotAllowedError` on
        // getDisplayMedia as "the browser blocked us" and pop a
        // "needs permission" modal. Real Chrome ALSO throws
        // NotAllowedError on picker cancel, but Meet silently swallows
        // it there — presumably via a separate Permissions API check
        // that reports 'granted'. Since we can't easily signal that
        // state in CEF, throw `AbortError` instead: it's the MDN-blessed
        // "user interrupted a UI operation" error and most sites (Meet
        // included) dismiss it silently.
        throw new DOMException('User cancelled screen share picker', 'AbortError');
      }
      // Finalize the session BEFORE getUserMedia: the Chromium capture
      // path doesn't need the token, and leaving the session open past
      // this point would just hold the `active` slot for the account
      // until the 30s TTL fires.
      await finalizeSessionQuiet(sessionToken, pick.id);
      send('log', {
        level: 'info',
        msg: '[gdm-shim] picked id=' + pick.id + ' kind=' + pick.kind,
      });
      const videoMandatory = {
        chromeMediaSource: 'desktop',
        chromeMediaSourceId: pick.id,
        maxFrameRate: 30,
      };
      // System-audio capture via `chromeMediaSource: 'desktop'` needs a
      // loopback driver on macOS (no stock API). If the page requested
      // audio we try with audio first and fall back to video-only on
      // rejection so Meet/Slack/etc don't see a generic "Can't share"
      // error on every attempt. Chromium cleanly handles a missing audio
      // track in the SDP.
      const videoOnly = { video: { mandatory: videoMandatory }, audio: false };

      let stream;
      if (constraints.audio) {
        const audioMandatory = {
          chromeMediaSource: 'desktop',
          chromeMediaSourceId: pick.id,
        };
        try {
          stream = await navigator.mediaDevices.getUserMedia({
            video: { mandatory: videoMandatory },
            audio: { mandatory: audioMandatory },
          });
        } catch (e) {
          send('log', {
            level: 'warn',
            msg:
              '[gdm-shim] audio+video getUserMedia rejected (' +
              (e && e.name ? e.name : '?') +
              '), retrying video-only',
          });
          stream = await navigator.mediaDevices.getUserMedia(videoOnly);
        }
      } else {
        stream = await navigator.mediaDevices.getUserMedia(videoOnly);
      }

      // Stream returned by the legacy `chromeMediaSource: 'desktop'`
      // getUserMedia path is a real capture stream but its tracks lack
      // the display-media metadata the page expects from real
      // getDisplayMedia. Google Meet (and others) inspect
      // `track.getSettings().displaySurface` before they will route the
      // track over WebRTC — if the field is missing they throw "Can't
      // share your screen — Something went wrong".
      //
      // Patch each video track to expose the right displaySurface and
      // a `contentHint` of `detail` (standard WebRTC screen-capture
      // content hint). The underlying capture pipeline is unchanged;
      // we're only fixing the introspectable metadata the page relies
      // on to identify a display-media track.
      const displaySurface = pick.kind === 'screen' ? 'monitor' : 'window';
      stream.getVideoTracks().forEach(function (track) {
        try { track.contentHint = 'detail'; } catch (_) { /* ignore */ }
        try {
          const origGetSettings = track.getSettings.bind(track);
          Object.defineProperty(track, 'getSettings', {
            configurable: true,
            writable: true,
            value: function () {
              const base = origGetSettings() || {};
              return Object.assign({}, base, {
                displaySurface: displaySurface,
                logicalSurface: true,
                cursor: 'motion',
              });
            },
          });
        } catch (e) {
          send('log', {
            level: 'warn',
            msg: '[gdm-shim] patch getSettings failed: ' + (e && e.message ? e.message : e),
          });
        }
      });

      return stream;
    };

    // In-page picker. Renders straight into the host page's <body> so the
    // overlay stacks above the site's own compositor (Meet/Slack/Discord
    // UI) without any native-view gymnastics. All nodes are namespaced
    // under `__ohsp_*` class/ID prefixes and attached to a closed shadow
    // root where possible to avoid colliding with the host page's CSS.
    function showInPagePicker(sources, sessionToken) {
      return new Promise(function (resolveOuter, rejectOuter) {
        function host() { return (document.body || document.documentElement); }
        if (!host()) {
          // DOM hasn't parsed yet — wait for it and retry. Previously we
          // resolved null here, which the shim turned into an AbortError
          // even though no picker was ever shown (coderabbit #809).
          document.addEventListener(
            'DOMContentLoaded',
            function () {
              showInPagePicker(sources, sessionToken).then(resolveOuter, rejectOuter);
            },
            { once: true }
          );
          return;
        }

        const root = document.createElement('div');
        root.setAttribute('data-openhuman-screen-share-picker', '');
        root.style.cssText = [
          'all: initial',
          'position: fixed',
          'inset: 0',
          'z-index: 2147483647',
          'display: flex',
          'align-items: center',
          'justify-content: center',
          'background: rgba(0,0,0,0.55)',
          'backdrop-filter: blur(6px)',
          '-webkit-backdrop-filter: blur(6px)',
          'font-family: -apple-system, BlinkMacSystemFont, "Inter", "Segoe UI", sans-serif',
        ].join(';');

        const shadow = root.attachShadow ? root.attachShadow({ mode: 'closed' }) : root;

        const styleTag = document.createElement('style');
        styleTag.textContent = [
          '* { box-sizing: border-box; margin: 0; padding: 0; font-family: inherit; }',
          '.card { background: #fff; color: #1C1917; border-radius: 16px; width: min(640px, 92vw);',
          '        max-height: 86vh; box-shadow: 0 24px 64px rgba(0,0,0,0.35); overflow: hidden;',
          '        display: flex; flex-direction: column; }',
          '.head { padding: 20px 24px; border-bottom: 1px solid #E7E5E4; display: flex;',
          '        align-items: flex-start; justify-content: space-between; gap: 16px; }',
          '.title { font-size: 17px; font-weight: 600; color: #1C1917; }',
          '.origin { margin-top: 4px; font-size: 13px; color: #78716C; }',
          '.closebtn { width: 32px; height: 32px; border: none; background: transparent;',
          '            color: #78716C; cursor: pointer; border-radius: 8px; font-size: 18px;',
          '            display: flex; align-items: center; justify-content: center; }',
          '.closebtn:hover { background: #F5F5F4; color: #1C1917; }',
          '.tabs { display: flex; gap: 4px; padding: 0 24px; border-bottom: 1px solid #E7E5E4; }',
          '.tab { appearance: none; -webkit-appearance: none; background: transparent; border: 0;',
          '       padding: 12px 16px; font-size: 14px; font-weight: 500; color: #78716C;',
          '       cursor: pointer; border-bottom: 2px solid transparent; }',
          '.tab.active { color: #4A83DD; border-bottom-color: #4A83DD; }',
          '.body { padding: 20px 24px; overflow-y: auto; }',
          '.grid { display: grid; grid-template-columns: repeat(2, minmax(0,1fr)); gap: 12px; }',
          '.srcbtn { background: #FAFAF9; border: 2px solid #E7E5E4; border-radius: 10px;',
          '          padding: 0; cursor: pointer; text-align: left; overflow: hidden;',
          '          transition: border-color .15s, box-shadow .15s; }',
          '.srcbtn:hover { border-color: #D4D4D1; }',
          '.srcbtn.selected { border-color: #4A83DD;',
          '                   box-shadow: 0 0 0 3px rgba(74,131,221,0.18); }',
          '.srcthumb { aspect-ratio: 16/10; background: #F5F5F4; display: flex;',
          '            align-items: center; justify-content: center; color: #A8A29E;',
          '            font-size: 32px; }',
          '.srcname { padding: 8px 10px; font-size: 13px; color: #1C1917; font-weight: 500;',
          '           white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }',
          '.srcapp { padding: 0 10px 8px; font-size: 11px; color: #78716C;',
          '          white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }',
          '.empty { padding: 32px 0; text-align: center; color: #78716C; font-size: 13px; }',
          '.foot { padding: 12px 16px; border-top: 1px solid #E7E5E4; display: flex;',
          '        justify-content: flex-end; gap: 8px; }',
          '.btn { appearance: none; -webkit-appearance: none; border: 0; border-radius: 10px;',
          '       padding: 9px 16px; font-size: 14px; font-weight: 500; cursor: pointer; }',
          '.btn-secondary { background: transparent; color: #1C1917; }',
          '.btn-secondary:hover { background: #F5F5F4; }',
          '.btn-primary { background: #4A83DD; color: #fff; }',
          '.btn-primary:hover { background: #3D6DC4; }',
          '.btn-primary:disabled { background: #D4D4D1; cursor: not-allowed; }',
        ].join('\n');
        shadow.appendChild(styleTag);

        function hostnameOf(url) {
          try { return new URL(url).hostname || url; } catch (e) { return url; }
        }

        const origin = (typeof location !== 'undefined' && location.origin) || 'this site';
        let activeTab = sources.some(function (s) { return s.kind === 'screen'; })
          ? 'screen'
          : 'window';
        let selectedId = null;

        // DOM is constructed imperatively (no innerHTML) because hosts
        // like Google Meet ship strict Trusted Types CSP that rejects
        // string-based HTML assignment with a TypeError. `createElement`
        // and `appendChild` are policy-free and work everywhere.
        const card = document.createElement('div');
        card.className = 'card';

        function el(tag, attrs, text) {
          const node = document.createElement(tag);
          if (attrs) {
            Object.keys(attrs).forEach(function (k) {
              if (k === 'className') node.className = attrs[k];
              else node.setAttribute(k, attrs[k]);
            });
          }
          if (text != null) node.textContent = text;
          return node;
        }

        const head = el('div', { className: 'head' });
        const headLeft = el('div');
        headLeft.appendChild(el('div', { className: 'title' }, 'Choose what to share'));
        const originEl = el(
          'div',
          { className: 'origin' },
          hostnameOf(origin) + ' wants to share your screen.'
        );
        headLeft.appendChild(originEl);
        head.appendChild(headLeft);
        const closeBtn = el(
          'button',
          { className: 'closebtn', 'data-action': 'cancel', 'aria-label': 'Cancel' },
          '✕'
        );
        head.appendChild(closeBtn);
        card.appendChild(head);

        const tabs = el('div', { className: 'tabs' });
        const screenTab = el('button', { className: 'tab', 'data-tab': 'screen' }, 'Entire Screen');
        const windowTab = el('button', { className: 'tab', 'data-tab': 'window' }, 'Window');
        tabs.appendChild(screenTab);
        tabs.appendChild(windowTab);
        card.appendChild(tabs);

        const bodyEl = el('div', { className: 'body' });
        const gridEl = el('div', { className: 'grid' });
        bodyEl.appendChild(gridEl);
        card.appendChild(bodyEl);

        const foot = el('div', { className: 'foot' });
        const cancelBtn = el(
          'button',
          { className: 'btn btn-secondary', 'data-action': 'cancel' },
          'Cancel'
        );
        const shareBtn = el('button', { className: 'btn btn-primary' }, 'Share');
        shareBtn.disabled = true;
        foot.appendChild(cancelBtn);
        foot.appendChild(shareBtn);
        card.appendChild(foot);

        shadow.appendChild(card);

        const tabButtons = [screenTab, windowTab];

        function setTab(next) {
          activeTab = next;
          tabButtons.forEach(function (btn) {
            btn.classList.toggle('active', btn.getAttribute('data-tab') === activeTab);
          });
          render();
        }

        function render() {
          while (gridEl.firstChild) gridEl.removeChild(gridEl.firstChild);
          const filtered = sources.filter(function (s) { return s.kind === activeTab; });
          if (filtered.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent =
              'No ' + (activeTab === 'screen' ? 'screens' : 'windows') + ' available.';
            gridEl.appendChild(empty);
            shareBtn.disabled = true;
            return;
          }
          filtered.forEach(function (src) {
            const btn = document.createElement('button');
            btn.className = 'srcbtn' + (selectedId === src.id ? ' selected' : '');
            btn.setAttribute('data-source-id', src.id);
            const thumb = document.createElement('div');
            thumb.className = 'srcthumb';
            if (src.thumbnailPngBase64) {
              const img = document.createElement('img');
              img.src = 'data:image/png;base64,' + src.thumbnailPngBase64;
              img.alt = '';
              img.style.cssText =
                'width: 100%; height: 100%; object-fit: contain; display: block;';
              thumb.appendChild(img);
            } else {
              // Placeholder glyph until the lazy-loaded thumbnail arrives.
              thumb.textContent = activeTab === 'screen' ? '□' : '▣';
              // Dedup in-flight thumbnail IPCs: render() re-runs on every
              // selection change and tab switch, and without this cache
              // each pass would re-issue screen_share_thumbnail for every
              // source that hadn't yet returned (coderabbit #809).
              function paintThumb(b64) {
                if (!b64 || typeof b64 !== 'string') return;
                const liveBtn = gridEl.querySelector(
                  '[data-source-id="' + src.id.replace(/"/g, '\\"') + '"]'
                );
                if (!liveBtn) return;
                const liveThumb = liveBtn.querySelector('.srcthumb');
                if (!liveThumb) return;
                while (liveThumb.firstChild) liveThumb.removeChild(liveThumb.firstChild);
                const img = document.createElement('img');
                img.src = 'data:image/png;base64,' + b64;
                img.alt = '';
                img.style.cssText =
                  'width: 100%; height: 100%; object-fit: contain; display: block;';
                liveThumb.appendChild(img);
              }
              if (src.__thumbnailPromise) {
                src.__thumbnailPromise.then(paintThumb, function () {});
              } else {
                src.__thumbnailPromise = rawInvoke('screen_share_thumbnail', {
                  args: { token: sessionToken, id: src.id },
                }).then(
                  function (b64) {
                    if (b64 && typeof b64 === 'string') {
                      // Stash on the source so future re-renders keep
                      // the thumbnail without re-requesting it.
                      src.thumbnailPngBase64 = b64;
                    }
                    paintThumb(b64);
                    return b64;
                  },
                  function () {
                    /* thumbnail failures degrade gracefully to the glyph */
                  }
                );
              }
            }
            const name = document.createElement('div');
            name.className = 'srcname';
            name.textContent = src.name;
            btn.appendChild(thumb);
            btn.appendChild(name);
            if (src.appName) {
              const app = document.createElement('div');
              app.className = 'srcapp';
              app.textContent = src.appName;
              btn.appendChild(app);
            }
            btn.addEventListener('click', function () {
              selectedId = src.id;
              render();
            });
            btn.addEventListener('dblclick', function () {
              selectedId = src.id;
              finish(sources.find(function (s) { return s.id === selectedId; }) || null);
            });
            gridEl.appendChild(btn);
          });
          if (!selectedId || !filtered.some(function (s) { return s.id === selectedId; })) {
            selectedId = filtered[0].id;
            gridEl.firstChild && gridEl.firstChild.classList.add('selected');
          }
          shareBtn.disabled = !selectedId;
        }

        tabButtons.forEach(function (btn) {
          btn.addEventListener('click', function () { setTab(btn.getAttribute('data-tab')); });
        });

        let settled = false;
        function finish(pick) {
          if (settled) return;
          settled = true;
          window.removeEventListener('keydown', onKey, true);
          try { root.remove(); } catch (e) { /* ignore */ }
          resolveOuter(pick);
        }

        card.querySelectorAll('[data-action="cancel"]').forEach(function (btn) {
          btn.addEventListener('click', function () { finish(null); });
        });
        shareBtn.addEventListener('click', function () {
          const pick = sources.find(function (s) { return s.id === selectedId; }) || null;
          finish(pick);
        });
        // Clicks on the backdrop (outside the card) cancel. Clicks inside
        // the card bubble up to root too, but we stop them there.
        root.addEventListener('click', function (e) {
          if (e.target === root || e.composedPath()[0] === root) finish(null);
        });
        card.addEventListener('click', function (e) { e.stopPropagation(); });

        function onKey(e) {
          if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            finish(null);
          }
        }
        window.addEventListener('keydown', onKey, true);

        setTab(activeTab);
        host().appendChild(root);
      });
    }

    let installed = false;
    try {
      Object.defineProperty(proto, 'getDisplayMedia', {
        configurable: true,
        writable: true,
        value: shim,
      });
      installed = true;
    } catch (e) {
      send('log', {
        level: 'error',
        msg: '[gdm-shim] defineProperty(proto) failed: ' + (e && e.message ? e.message : String(e)),
      });
    }
    if (!installed) {
      try {
        Object.defineProperty(navigator.mediaDevices, 'getDisplayMedia', {
          configurable: true,
          writable: true,
          value: shim,
        });
        installed = true;
      } catch (e2) {
        send('log', {
          level: 'error',
          msg: '[gdm-shim] defineProperty(instance) failed: ' + (e2 && e2.message ? e2.message : String(e2)),
        });
      }
    }
    navigator.mediaDevices.__ohGdmShimInstalled = installed;

    // Some pages (Meet) also consult `navigator.permissions.query` and
    // branch on the reported state for `display-capture` /
    // `camera` / `microphone`. CEF Alloy's Permissions API does not
    // reflect what our OnRequestMediaAccessPermission callback will
    // grant dynamically, so it defaults to 'prompt' or even 'denied'
    // for `display-capture`. A page that sees 'denied' will assume
    // sharing is structurally blocked and refuse to call
    // getDisplayMedia — or show the "needs permission" modal on cancel.
    // We shadow the query for these names so the page sees 'granted'
    // and relies on our shim for the actual user decision.
    try {
      if (
        navigator.permissions &&
        typeof navigator.permissions.query === 'function' &&
        !navigator.permissions.__ohPermissionsShimInstalled
      ) {
        const permProto = Object.getPrototypeOf(navigator.permissions);
        const permDescriptor = Object.getOwnPropertyDescriptor(permProto, 'query');
        const origQuery = (permDescriptor && permDescriptor.value
          ? permDescriptor.value
          : navigator.permissions.query
        ).bind(navigator.permissions);
        // CEF Alloy's Permissions API doesn't reflect what our
        // OnRequestMediaAccessPermission callback will grant dynamically,
        // so it defaults to 'prompt' or 'denied' for the media permissions
        // we do handle. Pages that consult the Permissions API up front
        // (Meet for display-capture; some flows for camera/microphone)
        // refuse to try the actual getUserMedia call if they see 'denied'
        // here. Spoof all three to 'granted'; the real grant still goes
        // through our CEF permission handler where it's scoped per-call.
        const spoofed = {
          'display-capture': 'granted',
          camera: 'granted',
          microphone: 'granted',
        };
        const spoofedQuery = async function (descriptor) {
          const n = descriptor && descriptor.name;
          if (n && spoofed[n]) {
            return {
              state: spoofed[n],
              status: spoofed[n],
              name: n,
              onchange: null,
              addEventListener: function () {},
              removeEventListener: function () {},
              dispatchEvent: function () { return true; },
            };
          }
          return origQuery(descriptor);
        };
        try {
          Object.defineProperty(permProto, 'query', {
            configurable: true,
            writable: true,
            value: spoofedQuery,
          });
        } catch (e) {
          Object.defineProperty(navigator.permissions, 'query', {
            configurable: true,
            writable: true,
            value: spoofedQuery,
          });
        }
        navigator.permissions.__ohPermissionsShimInstalled = true;
        send('log', { level: 'info', msg: '[gdm-shim] permissions.query shim installed' });
      }
    } catch (e) {
      send('log', {
        level: 'warn',
        msg: '[gdm-shim] permissions.query shim failed: ' + (e && e.message ? e.message : e),
      });
    }

    send('log', {
      level: 'info',
      msg:
        '[gdm-shim] install=' + installed +
        ' on ' + ((typeof location !== 'undefined' && location.origin) || '?'),
    });
  })();
})();
