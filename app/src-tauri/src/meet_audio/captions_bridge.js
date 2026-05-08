// OpenHuman captions bridge for the embedded Google Meet webview.
//
// Companion to `audio_bridge.js`. Where the audio bridge handles the
// SPEAK direction (synthesized PCM → MediaStream the page hands to its
// RTCPeerConnection), this script handles the LISTEN direction by
// scraping Meet's built-in live captions instead of running our own
// STT pipeline:
//
//   - Auto-click the "Turn on captions" button so the user doesn't
//     have to remember.
//   - Watch the captions region with a MutationObserver and a 250 ms
//     poll fallback (Meet sometimes batches DOM updates outside the
//     observer's notify window).
//   - Maintain a queue of new caption lines, deduped by speaker+text.
//     Each entry: { speaker, text, ts }.
//   - Expose `window.__openhumanDrainCaptions()` and
//     `__openhumanCaptionsBridgeInfo()` for the Tauri shell to drive
//     over CDP `Runtime.evaluate`.
//
// Why scraping (and not getDisplayMedia, or Web Speech, or Meet's
// undocumented APIs)?
//   - getDisplayMedia would prompt the user for screen-share permission.
//   - Web Speech doesn't reach the remote participants' audio — only
//     local mic.
//   - Meet has no public caption API.
//   - The captions DOM is the simplest stable source. Class names
//     obfuscate often, so we lean on `aria-label="Captions"` (which
//     Meet keeps stable for accessibility).
//
// Wake-word handling lives in the core (`src/openhuman/meet_agent/`),
// not here — the page just streams every caption line out and core
// decides when to act.

(function () {
  if (window.__openhumanCaptionsBridgeInstalled) {
    return;
  }
  window.__openhumanCaptionsBridgeInstalled = true;

  var queue = [];
  // Per-speaker last-text fingerprint so a caption that grows in place
  // (Meet appends text mid-utterance) doesn't get queued multiple
  // times. We emit the *latest* text for each speaker only when it
  // changes; downstream wake-word logic dedupes on its own buffer.
  var lastBySpeaker = {};

  function findCaptionsRegion() {
    // Meet's captions region carries a stable accessibility label
    // even as class names churn between rollouts. Try the canonical
    // English first, then fall back to a fuzzy match for localized
    // builds ("Subtitles", "Sous-titres", etc.) that still embed
    // "captions" / "caption" in the aria-label.
    return (
      document.querySelector('[aria-label="Captions"]') ||
      document.querySelector('div[role="region"][aria-label*="aption"]') ||
      document.querySelector('div[role="region"][aria-label*="aption" i]') ||
      null
    );
  }

  function pollOnce() {
    var region = findCaptionsRegion();
    if (!region) return;

    // Each caption line is typically a flex row with the speaker name
    // at the top and the live transcript below. We don't depend on
    // exact class names; instead we walk direct children and treat
    // each as one caption "row".
    var rows = region.querySelectorAll(
      ':scope > div, :scope > section, :scope > [role="listitem"]'
    );
    if (!rows.length) {
      // Fall back to a single-block region: one big innerText blob.
      var blob = (region.innerText || "").trim();
      if (blob && blob !== lastBySpeaker.__blob__) {
        queue.push({ speaker: "", text: blob, ts: Date.now() });
        lastBySpeaker.__blob__ = blob;
      }
      return;
    }

    rows.forEach(function (row) {
      // The speaker name is usually the first text child; the
      // transcript is the larger one beneath. Heuristic: the line
      // with the most text wins as "transcript".
      var nodes = row.querySelectorAll("*");
      var bestText = "";
      var bestCandidate = null;
      var speakerGuess = "";
      nodes.forEach(function (n) {
        var t = (n.innerText || "").trim();
        if (!t) return;
        if (!speakerGuess && t.length < 40 && /^[A-Za-z][\w '\-\.]*$/.test(t)) {
          speakerGuess = t;
        }
        if (t.length > bestText.length) {
          bestText = t;
          bestCandidate = n;
        }
      });
      if (!bestText) return;
      // Strip the speaker name out of the body if it's the leading
      // chunk (Meet sometimes renders "Alice  the meeting starts at 3"
      // as one innerText blob).
      if (speakerGuess && bestText.startsWith(speakerGuess)) {
        bestText = bestText.slice(speakerGuess.length).trim();
      }
      if (!bestText) return;

      var key = speakerGuess || "_unknown";
      if (lastBySpeaker[key] === bestText) return;
      lastBySpeaker[key] = bestText;
      queue.push({ speaker: speakerGuess, text: bestText, ts: Date.now() });
    });
  }

  // Two layers, because Meet sometimes batches caption DOM updates
  // in ways that miss MutationObserver notifications:
  //
  //   1. MutationObserver — fires immediately on DOM mutation, picks
  //      up character-data changes that the poll might miss between
  //      ticks.
  //   2. 250 ms interval poll — safety net for batched updates and
  //      for the case where the captions region didn't exist at
  //      observer-attach time.
  function attachObserver() {
    var region = findCaptionsRegion();
    if (!region || region.__openhumanObserverAttached) return false;
    region.__openhumanObserverAttached = true;
    var obs = new MutationObserver(function () {
      pollOnce();
    });
    obs.observe(region, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    return true;
  }

  // Auto-enable captions: walk every button on the page and click any
  // that has an aria-label starting with "Turn on captions". Caps the
  // attempts so we don't fight a user who deliberately disables CC.
  var ENABLE_ATTEMPT_BUDGET = 30; // ~30 * 2s = 60s
  var enableAttempts = 0;
  function tryEnableCaptions() {
    if (enableAttempts >= ENABLE_ATTEMPT_BUDGET) return;
    enableAttempts++;
    var buttons = document.querySelectorAll("button[aria-label]");
    for (var i = 0; i < buttons.length; i++) {
      var lbl = (buttons[i].getAttribute("aria-label") || "").toLowerCase();
      // Match "Turn on captions" but NOT "Turn off captions".
      if (lbl.indexOf("turn on captions") === 0 || /^turn on captions/.test(lbl)) {
        try {
          buttons[i].click();
          enableAttempts = ENABLE_ATTEMPT_BUDGET; // success — stop trying.
          return true;
        } catch (_) {}
      }
    }
    return false;
  }

  setInterval(function () {
    attachObserver();
    pollOnce();
  }, 250);
  setInterval(tryEnableCaptions, 2000);

  // Public API consumed by the Tauri shell over CDP Runtime.evaluate.
  window.__openhumanDrainCaptions = function () {
    var out = queue.slice();
    queue.length = 0;
    return out;
  };

  window.__openhumanCaptionsBridgeInfo = function () {
    return {
      installed: true,
      region_found: !!findCaptionsRegion(),
      queue_depth: queue.length,
      tracked_speakers: Object.keys(lastBySpeaker).length,
      enable_attempts: enableAttempts,
    };
  };
})();
