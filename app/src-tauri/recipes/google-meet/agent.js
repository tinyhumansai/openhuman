// Google Meet Agent — Stage 1: auto-join as a headless attendee.
//
// Role gating:
//   This script runs inside every google-meet webview (injected via
//   initialization_script). It bails immediately if __OPENHUMAN_RECIPE_CTX__.role
//   is not "agent" so the user-facing recipe.js path is unaffected.
//
// Lifecycle events emitted (via window.__openhumanRecipe.emit):
//   meet_agent_joined  { code: string, joinedAt: number }
//   meet_agent_left    { reason: 'leave-button-gone' | 'navigated-away' }
//   meet_agent_failed  { reason: 'timeout' | 'meeting-not-found' | 'permission-denied' | 'sign-in-required' }
//
// DOM selector contract:
//   All selectors below are "stable-ish" — they've been accurate as of 2026.
//   Meet ships continuous quiet DOM renames; expect periodic maintenance
//   in sync with recipe.js. When a selector stops working, check DevTools
//   on the lobby page and update the relevant helper.
//
//   Key selectors:
//     button[jsname="Qx7uuf"]           — primary Join/Ask-to-join action
//     [role="button"][aria-label*="microphone" i] — pre-join mic toggle
//     [role="button"][aria-label*="camera" i]     — pre-join camera toggle
//     [aria-label="Leave call"]                   — in-call leave button
//     [jsname="CQylAd"]                           — leave button jsname fallback
//     [data-self-name], [data-participant-id]     — in-call presence signals
//
// Public API:
//   window.__openhumanMeetAgent.leave()           — best-effort leave click
//   window.__openhumanMeetAgent.pure.*            — testable pure helpers
//
// Out of scope for Stage 1:
//   No avatar rendering, no audio capture, no TTS/STT, no LLM loop.
//   Those come in stages 2-5.

(function () {
  var ctx = window.__OPENHUMAN_RECIPE_CTX__;
  var api = window.__openhumanRecipe;

  // Role gate — bail out immediately if we're not the agent webview.
  if (!ctx || ctx.role !== 'agent') {
    return;
  }

  var meetingUrl = ctx.meetingUrl || '';
  var accountId = ctx.accountId || '';

  if (api) {
    api.log('info', '[meet-agent] starting accountId=' + accountId + ' meetingUrl=' + meetingUrl);
  }

  // ─── Pure helpers (exported for Vitest) ──────────────────────────────────

  /**
   * Extract the meeting code (e.g. "abc-defg-hij") from a full URL string.
   * Returns null when the pathname does not match a Meet room pattern.
   */
  function extractMeetingCode(href) {
    try {
      var pathname = new URL(href).pathname;
      var m = /^\/([a-z]{3,4}-[a-z]{3,4}-[a-z]{3,4})(?:$|\/|\?)/i.exec(pathname);
      return m ? m[1] : null;
    } catch (_) {
      return null;
    }
  }

  /**
   * Locate the pre-join "Join now" / "Ask to join" button.
   * Strategy (in priority order):
   *   1. button[jsname="Qx7uuf"]
   *   2. button[aria-label="Join now" | "Ask to join" | "Join"]
   *   3. Any visible <button> with matching textContent
   * Returns null if not found or if the button is disabled.
   */
  function findJoinButton(doc) {
    try {
      // 1. Stable jsname — most reliable when present.
      var byJsname = doc.querySelector('button[jsname="Qx7uuf"]');
      if (byJsname && !byJsname.disabled) return byJsname;

      // 2. aria-label variants.
      var ariaLabels = ['Join now', 'Ask to join', 'Join'];
      for (var i = 0; i < ariaLabels.length; i++) {
        var btn = doc.querySelector('button[aria-label="' + ariaLabels[i] + '"]');
        if (btn && !btn.disabled) return btn;
      }

      // 3. Text-content fallback.
      var buttons = doc.querySelectorAll('button');
      for (var j = 0; j < buttons.length; j++) {
        var b = buttons[j];
        if (b.disabled) continue;
        var text = (b.textContent || '').trim();
        if (/^(Join now|Ask to join|Join)$/i.test(text)) return b;
      }
    } catch (_) {}
    return null;
  }

  /**
   * Locate the pre-join microphone toggle.
   * Returns null if not found.
   */
  function findMicButton(doc) {
    try {
      var btn = doc.querySelector('[role="button"][aria-label*="microphone" i]');
      if (btn) return btn;
      btn = doc.querySelector('[data-is-muted]');
      if (btn) return btn;
    } catch (_) {}
    return null;
  }

  /**
   * Locate the pre-join camera toggle.
   * Returns null if not found.
   */
  function findCamButton(doc) {
    try {
      var btn = doc.querySelector('[role="button"][aria-label*="camera" i]');
      if (btn) return btn;
    } catch (_) {}
    return null;
  }

  /**
   * Returns true if the mic button appears to be ON (unmuted).
   * Defaults to false (assume off) when the state is ambiguous — safer to
   * skip clicking an already-off button than to accidentally turn it on.
   */
  function isMicOn(btn) {
    if (!btn) return false;
    try {
      if (btn.getAttribute('aria-pressed') === 'true') return true;
      if (btn.getAttribute('data-is-muted') === 'false') return true;
    } catch (_) {}
    return false;
  }

  /**
   * Returns true if the camera button appears to be ON.
   * Same conservative default as isMicOn.
   */
  function isCamOn(btn) {
    if (!btn) return false;
    try {
      if (btn.getAttribute('aria-pressed') === 'true') return true;
      if (btn.getAttribute('data-is-muted') === 'false') return true;
    } catch (_) {}
    return false;
  }

  /**
   * Returns true when the document shows in-call participant signals.
   * [data-self-name] appears on the user's own tile; [data-participant-id]
   * appears on every participant tile. Either is sufficient.
   */
  function isInCall(doc) {
    try {
      if (doc.querySelector('[data-self-name]')) return true;
      if (doc.querySelector('[data-participant-id]')) return true;
    } catch (_) {}
    return false;
  }

  /**
   * Locate the in-call "Leave call" button.
   * Returns null if not found.
   */
  function findLeaveButton(doc) {
    try {
      var btn = doc.querySelector('[aria-label="Leave call"]');
      if (btn) return btn;
      btn = doc.querySelector('[jsname="CQylAd"]');
      if (btn) return btn;
      var buttons = doc.querySelectorAll('button');
      for (var i = 0; i < buttons.length; i++) {
        var text = (buttons[i].textContent || '').trim();
        if (/^leave(\s+call)?$/i.test(text)) return buttons[i];
      }
    } catch (_) {}
    return null;
  }

  /**
   * Detect screens where joining is impossible.
   * Returns a reason string or null (let the timeout handle ambiguous cases).
   *
   * Conservative — unknown screens return null rather than false-positive.
   */
  function isUnjoinableScreen(doc) {
    try {
      var bodyText = (doc.body && doc.body.textContent) ? doc.body.textContent : '';
      if (/check your meeting code/i.test(bodyText)) return 'meeting-not-found';
      if (/you can't join this video call/i.test(bodyText)) return 'permission-denied';
      if (/switch account/i.test(bodyText) && /sign in/i.test(bodyText)) return 'sign-in-required';
    } catch (_) {}
    return null;
  }

  // ─── Main agent loop ──────────────────────────────────────────────────────

  var JOIN_TIMEOUT_MS = 60000;
  var POLL_INTERVAL_MS = 1000;

  var startedAt = Date.now();
  var joinedCode = null;   // non-null once we've emitted meet_agent_joined
  var failedEmitted = false;
  var pollTimer = null;
  var stopped = false;

  function emitOnce(kind, payload) {
    if (!api) return;
    try {
      api.emit(kind, payload);
    } catch (_) {}
  }

  function stopPolling() {
    stopped = true;
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  function poll() {
    if (stopped) return;

    var elapsed = Date.now() - startedAt;
    var doc = document;
    var currentHref = window.location.href;

    // If we're not on the target meeting URL, navigate there.
    var targetCode = extractMeetingCode(meetingUrl);
    var currentCode = extractMeetingCode(currentHref);

    if (targetCode && currentCode !== targetCode) {
      if (api) api.log('debug', '[meet-agent] navigating to meeting url=' + meetingUrl);
      try {
        window.location.replace(meetingUrl);
      } catch (_) {}
      return; // Wait for next poll after navigation.
    }

    // Check for unjoinable screens first.
    var unjoinable = isUnjoinableScreen(doc);
    if (unjoinable && !failedEmitted) {
      failedEmitted = true;
      if (api) api.log('warn', '[meet-agent] unjoinable screen reason=' + unjoinable);
      emitOnce('meet_agent_failed', { accountId: accountId, reason: unjoinable });
      stopPolling();
      return;
    }

    var inCall = isInCall(doc);

    // Transition: we were in the call and now we're not.
    if (joinedCode && !inCall) {
      var navigatedAway = currentCode && currentCode !== joinedCode;
      var reason = navigatedAway ? 'navigated-away' : 'leave-button-gone';
      if (api) api.log('info', '[meet-agent] left call code=' + joinedCode + ' reason=' + reason);
      emitOnce('meet_agent_left', { accountId: accountId, reason: reason });
      stopPolling();
      return;
    }

    // Transition: we just joined.
    if (inCall && !joinedCode) {
      joinedCode = currentCode || targetCode || 'unknown';
      if (api) api.log('info', '[meet-agent] joined call code=' + joinedCode);
      emitOnce('meet_agent_joined', {
        accountId: accountId,
        code: joinedCode,
        joinedAt: Date.now(),
      });
      return; // Continue polling to detect leave.
    }

    // Already in call — nothing to do this tick.
    if (inCall && joinedCode) return;

    // Not yet in call — try to join if we find the join button.
    var joinBtn = findJoinButton(doc);
    if (joinBtn) {
      // Ensure mic is off.
      var micBtn = findMicButton(doc);
      if (micBtn && isMicOn(micBtn)) {
        if (api) api.log('debug', '[meet-agent] muting mic before join');
        try { micBtn.click(); } catch (_) {}
      }
      // Ensure cam is off.
      var camBtn = findCamButton(doc);
      if (camBtn && isCamOn(camBtn)) {
        if (api) api.log('debug', '[meet-agent] disabling camera before join');
        try { camBtn.click(); } catch (_) {}
      }
      if (api) api.log('info', '[meet-agent] clicking join button');
      try { joinBtn.click(); } catch (_) {}
      return;
    }

    // Timeout check — only applies before we've joined.
    if (!joinedCode && elapsed >= JOIN_TIMEOUT_MS && !failedEmitted) {
      failedEmitted = true;
      if (api) api.log('warn', '[meet-agent] join timeout after ' + Math.round(elapsed / 1000) + 's');
      emitOnce('meet_agent_failed', { accountId: accountId, reason: 'timeout' });
      stopPolling();
    }
  }

  pollTimer = setInterval(poll, POLL_INTERVAL_MS);

  // Run one tick immediately (don't wait for first interval).
  try { poll(); } catch (_) {}

  // ─── Public API ───────────────────────────────────────────────────────────

  window.__openhumanMeetAgent = {
    /**
     * Best-effort: click the Leave call button.
     * The host Tauri command calls this before closing the webview, but
     * closing the webview is the authoritative teardown — this is just
     * graceful cleanup.
     */
    leave: function () {
      try {
        var btn = findLeaveButton(document);
        if (btn) {
          if (api) api.log('info', '[meet-agent] leave() clicked leave button');
          btn.click();
        } else {
          if (api) api.log('debug', '[meet-agent] leave() leave button not found (host will close webview)');
        }
      } catch (_) {}
      stopPolling();
    },

    /** Pure helpers — exposed for Vitest (no side effects). */
    pure: {
      extractMeetingCode: extractMeetingCode,
      findJoinButton: findJoinButton,
      findMicButton: findMicButton,
      findCamButton: findCamButton,
      isMicOn: isMicOn,
      isCamOn: isCamOn,
      isInCall: isInCall,
      findLeaveButton: findLeaveButton,
      isUnjoinableScreen: isUnjoinableScreen,
    },
  };
})();
