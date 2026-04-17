// Google Meet recipe.
//
// Scope:
//   * Track the meeting lifecycle (joined call → left call / navigated
//     away) driven off the URL path.
//   * Stream Meet's own live-caption text back to Rust so the host can
//     accumulate a transcript. We do NOT run Whisper here — Meet's
//     built-in captions are the source of truth. User must have
//     "Turn on captions" enabled in Meet for this to yield anything.
//
// Event kinds emitted (on top of the runtime's standard set):
//   meet_call_started  { code, url, startedAt }
//   meet_captions      { code, captions:[{speaker,text}], ts }
//   meet_call_ended    { code, endedAt, reason }
//
// DOM anchors used — all are "stable-ish", meaning they've held for
// months at a time but are not contractual. Expect periodic maintenance
// when Meet ships a big redesign.
//   * URL path `/xxx-xxxx-xxx`  → meeting code / "am I in a call"
//   * `[jsname="tgaKEf"]`       → caption region container
//   * within a caption row: first `img[alt]` or `[data-self-name]`
//     for the speaker's display name; the rest of the text nodes for
//     the rolling transcript line
(function (api) {
  if (!api) return;
  api.log('info', '[google-meet-recipe] starting');

  const MEETING_CODE_RE = /^\/([a-z]{3,4}-[a-z]{3,4}-[a-z]{3,4})(?:$|\/|\?)/i;

  // Current-call state, owned by the recipe. Transitions are the trigger
  // for the started/ended lifecycle events.
  let currentCode = null;
  let startedAt = 0;

  // Meet SPA-navigates you off the meeting URL when you leave a call,
  // which destroys this JS context before emitEnded can run. Persist the
  // in-progress code to sessionStorage so the recipe on the next page
  // can emit a synthetic ended event for the previous session. Keyed by
  // origin (same-origin nav is guaranteed within meet.google.com).
  const SS_CODE = 'openhuman_gmeet_currentCode';
  const SS_STARTED_AT = 'openhuman_gmeet_startedAt';

  function ssGet(k) {
    try { return window.sessionStorage.getItem(k); } catch (_) { return null; }
  }
  function ssSet(k, v) {
    try { window.sessionStorage.setItem(k, v); } catch (_) {}
  }
  function ssDel(k) {
    try { window.sessionStorage.removeItem(k); } catch (_) {}
  }
  // Last caption snapshot we sent up — compared each tick so we only
  // emit when the on-screen captions actually changed.
  let lastCaptionsKey = '';

  function textOf(el) {
    return (el && el.textContent ? el.textContent : '').trim();
  }

  function meetingCode() {
    try {
      const m = MEETING_CODE_RE.exec(window.location.pathname || '');
      return m ? m[1] : null;
    } catch (_) {
      return null;
    }
  }

  // Pull the speaker name out of one caption row. Meet renders an avatar
  // image whose `alt` is the speaker's display name; own-user rows carry
  // a `data-self-name` attribute instead.
  function rowSpeaker(row) {
    try {
      const img = row.querySelector('img[alt]');
      if (img) {
        const alt = (img.getAttribute('alt') || '').trim();
        // Skip icon alts ("arrow_downward", "avatar", etc).
        if (alt && alt.length > 1 && !looksLikeIconLigature(alt) &&
            !/^avatar$/i.test(alt)) {
          return alt;
        }
      }
      const self = row.querySelector('[data-self-name]');
      if (self) {
        const name = (self.getAttribute('data-self-name') || '').trim();
        if (name) return name;
      }
      // Current Meet layout: speaker display name is the first non-empty
      // <span> inside the row (e.g. "You", "Alice"). Use it as a fallback
      // as long as it doesn't look like icon/chrome text.
      const spans = row.querySelectorAll('span');
      for (let i = 0; i < spans.length; i++) {
        const t = (spans[i].textContent || '').replace(/\s+/g, ' ').trim();
        if (!t) continue;
        if (looksLikeIconLigature(t)) continue;
        if (t.length > 40) continue; // too long to be a display name
        return t;
      }
    } catch (_) {}
    return 'Unknown';
  }

  // Pull the rolling transcript line for one caption row. We want the
  // caption text only, not the speaker's name / timestamp chrome, so we
  // collect text from nodes that DON'T live inside an img's parent block
  // and aren't the `[data-self-name]` node.
  function rowText(row) {
    try {
      // Current Meet layout (2026-04): the row's textContent concatenates
      // the speaker display name (inside a <span>) and the live caption
      // text (a sibling text node), with no separator — e.g.
      // "YouMake a massive improvement...". Picking the longest span
      // returns only "You"; we want the text AFTER the speaker span.
      const full = (row.textContent || '').replace(/\s+/g, ' ').trim();
      if (!full) return '';
      // Prefer stripping the first non-empty span's text from the front.
      const spans = row.querySelectorAll('span');
      let prefix = '';
      for (let i = 0; i < spans.length; i++) {
        const t = (spans[i].textContent || '').replace(/\s+/g, ' ').trim();
        if (t) {
          prefix = t;
          break;
        }
      }
      let stripped = full;
      if (prefix && full.toLowerCase().startsWith(prefix.toLowerCase())) {
        stripped = full.slice(prefix.length).trim();
      }
      // Drop the "Jump to bottom" chrome if it trails the caption.
      stripped = stripped.replace(/\s*arrow_downward\s*Jump to bottom\s*$/i, '').trim();
      return stripped;
    } catch (_) {
      return textOf(row);
    }
  }

  // Reject text that's clearly a Material Icon ligature rather than real
  // caption content. Meet's toolbar buttons (e.g. "closed_caption_off",
  // "settings", "mic_off") render the icon name as textContent because the
  // Material Symbols font turns ligatures into glyphs. Real captions are
  // natural language, so anything that's a single snake_case token is noise.
  function looksLikeIconLigature(text) {
    if (!text) return true;
    const t = text.trim();
    if (!t) return true;
    // Single token, all lowercase letters / digits / underscores: icon name.
    if (/^[a-z0-9_]+$/.test(t) && t.length < 40) return true;
    return false;
  }

  // Heuristic: real caption lines look like natural language — they contain
  // at least one whitespace separator once they're more than a word or two
  // long, and they rarely contain the same token concatenated with itself
  // (which is what Meet's IconButton tooltip + label fusion produces, e.g.
  // "closeClose", "settingsSettings", "micMic off").
  function looksLikeCaptionLine(text) {
    if (!text) return false;
    const t = text.trim();
    if (t.length < 3) return false;
    if (looksLikeIconLigature(t)) return false;
    // Toast/snackbar patterns: short, no whitespace, PascalCase-ish.
    if (t.length < 20 && !/\s/.test(t)) return false;
    // Repeated-token pattern like "closeClose" or "settingsSettings".
    if (/^([a-z]+)([A-Z][a-z]*)$/.test(t)) {
      const m = /^([a-z]+)([A-Z][a-z]*)$/.exec(t);
      if (m && m[2].toLowerCase() === m[1]) return false;
    }
    // Embedded Material icon ligature inside the text (e.g.
    // "Your meeting is safe: content_copyCopy link", "mic_offMic off").
    // Real caption lines never contain `foo_bar` tokens.
    if (/\b[a-z]+_[a-z]+\b/.test(t)) return false;
    // Known Meet snackbar / modal strings that slip through otherwise.
    if (/Your meeting is safe|Your meeting's ready|Copy link|Meeting details|Add people|Add others|Jump to bottom|Jump to most recent/i.test(t)) return false;
    // Repeated-token suffix like "closeClose" appearing anywhere.
    if (/([a-z]{3,})\1/i.test(t)) return false;
    return true;
  }

  // Heuristic: a real caption region contains multiple speaker rows, each
  // with an `img[alt]` avatar (or a `[data-self-name]` for the local user).
  // A toolbar button labelled "caption" does not. Score a candidate region
  // by how many plausible speaker rows live inside.
  function scoreCaptionRegion(el) {
    if (!el) return 0;
    try {
      const imgs = el.querySelectorAll('img[alt]');
      const selves = el.querySelectorAll('[data-self-name]');
      const spans = el.querySelectorAll('span');
      let plausible = 0;
      for (let i = 0; i < imgs.length; i++) {
        const alt = (imgs[i].getAttribute('alt') || '').trim();
        if (!alt || alt.length < 2) continue;
        // Exclude Material icon alts (content_copy, mic_off, etc) — those
        // are icons, not participant avatars.
        if (looksLikeIconLigature(alt)) continue;
        // Exclude generic labels like "Avatar" that some toasts use.
        if (/^avatar$/i.test(alt)) continue;
        plausible++;
      }
      for (let i = 0; i < selves.length; i++) {
        const name = (selves[i].getAttribute('data-self-name') || '').trim();
        if (name) plausible++;
      }
      // Needs speakers AND enough spans to host transcript text.
      if (plausible === 0) return 0;
      if (spans.length < 2) return 0;
      return plausible * 10 + spans.length;
    } catch (_) {
      return 0;
    }
  }

  // Locate the live captions container. Try the stable jsname first, then
  // fall back to *scored* candidates matching "captions" in aria-label or
  // an aria-live polite region. We only accept a candidate that actually
  // looks like a caption surface (see `scoreCaptionRegion`).
  function findCaptionRegion() {
    try {
      const primary = document.querySelector('[jsname="tgaKEf"]');
      if (primary && scoreCaptionRegion(primary) > 0) {
        return { region: primary, how: 'jsname=tgaKEf' };
      }
      if (primary) {
        api.log(
          'warn',
          '[google-meet-recipe] primary [jsname=tgaKEf] matched but scored 0 — Meet DOM may have changed'
        );
      }
    } catch (_) {}

    // Strong signal: Meet exposes the live captions container with
    // role="region" and a localized "Captions" aria-label. Current DOM
    // (as of 2026-04) no longer keeps participant avatars inside this
    // container, so scoring by `img[alt]` speakers rejects it. Accept it
    // directly as a high-confidence match.
    try {
      const labelled = document.querySelectorAll(
        '[role="region"][aria-label],[aria-label]'
      );
      for (let i = 0; i < labelled.length; i++) {
        const lbl = (labelled[i].getAttribute('aria-label') || '').trim();
        if (/^(captions|sous-titres|untertitel|leyendas|字幕)$/i.test(lbl)) {
          return {
            region: labelled[i],
            how: 'aria-label="' + lbl + '"',
          };
        }
      }
    } catch (_) {}

    // Candidate pool: aria-label containing "caption" (localized) OR
    // aria-live="polite" regions (Meet marks the captions container as a
    // live region so screen readers announce new lines).
    const candidates = [];
    try {
      const labelled = document.querySelectorAll('[aria-label]');
      for (let i = 0; i < labelled.length; i++) {
        const label = labelled[i].getAttribute('aria-label') || '';
        if (/caption|sous-titre|untertitel|leyenda|字幕/i.test(label)) {
          candidates.push(labelled[i]);
        }
      }
      const live = document.querySelectorAll('[aria-live="polite"]');
      for (let i = 0; i < live.length; i++) {
        candidates.push(live[i]);
      }
    } catch (_) {}

    let best = null;
    let bestScore = 0;
    let bestHow = '';
    for (let i = 0; i < candidates.length; i++) {
      const s = scoreCaptionRegion(candidates[i]);
      if (s > bestScore) {
        bestScore = s;
        best = candidates[i];
        const lbl = (candidates[i].getAttribute('aria-label') || '').slice(0, 40);
        const live = candidates[i].getAttribute('aria-live') || '';
        bestHow = 'fallback(aria-label="' + lbl + '",aria-live="' + live + '",score=' + s + ')';
      }
    }
    if (best) return { region: best, how: bestHow };
    return null;
  }

  // Throttled diagnostic so we can see what the recipe is actually looking
  // at inside a live call without spamming the log every tick.
  let lastDiagAt = 0;
  function maybeLogDiag(found, rows) {
    const now = Date.now();
    if (now - lastDiagAt < 5000) return;
    lastDiagAt = now;
    if (!found) {
      // Verbose dump: describe candidate regions across several selector
      // strategies so we can identify the renamed captions container.
      let dump = '';
      try {
        const seen = new Set();
        const regions = [];
        const addAll = (nodes, tag) => {
          for (let i = 0; i < nodes.length; i++) {
            if (seen.has(nodes[i])) continue;
            seen.add(nodes[i]);
            regions.push({ el: nodes[i], tag: tag });
          }
        };
        addAll(document.querySelectorAll('[aria-live="polite"]'), 'polite');
        addAll(document.querySelectorAll('[aria-live="assertive"]'), 'assertive');
        addAll(document.querySelectorAll('[role="log"]'), 'role=log');
        addAll(document.querySelectorAll('[role="region"]'), 'role=region');
        // aria-label containing "caption" in multiple locales
        const labelled = document.querySelectorAll('[aria-label]');
        for (let i = 0; i < labelled.length; i++) {
          const lbl = labelled[i].getAttribute('aria-label') || '';
          if (/caption|sous-titre|untertitel|leyenda|字幕/i.test(lbl)) {
            if (!seen.has(labelled[i])) {
              seen.add(labelled[i]);
              regions.push({ el: labelled[i], tag: 'label~caption' });
            }
          }
        }
        for (let i = 0; i < regions.length; i++) {
          const r = regions[i].el;
          const tag = regions[i].tag;
          const label = (r.getAttribute('aria-label') || '').slice(0, 40);
          const role = r.getAttribute('role') || '';
          const jsname = r.getAttribute('jsname') || '';
          const cls = (r.className || '').toString().slice(0, 40);
          const imgs = r.querySelectorAll('img[alt]').length;
          const selves = r.querySelectorAll('[data-self-name]').length;
          const spans = r.querySelectorAll('span').length;
          const txt = (r.textContent || '').trim().slice(0, 60);
          dump +=
            ' || [' + i + ' ' + tag + '] label="' + label + '" role="' + role +
            '" jsname="' + jsname + '" class="' + cls +
            '" imgs=' + imgs + ' selves=' + selves + ' spans=' + spans +
            ' text="' + txt.replace(/\n/g, ' ') + '"';
        }
      } catch (_) {}
      api.log(
        'info',
        '[google-meet-recipe] diag: no caption region found. jsname=' +
          (document.querySelector('[jsname="tgaKEf"]') ? 'present' : 'absent') +
          ' aria-live-polite=' +
          document.querySelectorAll('[aria-live="polite"]').length +
          dump
      );
      return;
    }
    let extra = '';
    if (!rows.length && found.region) {
      // No rows came through the filter — dump the region's child tree so
      // we can see how Meet is laying out captions now.
      try {
        const region = found.region;
        extra += ' children=' + region.children.length;
        for (let i = 0; i < Math.min(region.children.length, 3); i++) {
          const child = region.children[i];
          const txt = (child.textContent || '').trim().slice(0, 60).replace(/\n/g, ' ');
          const spans = child.querySelectorAll('span').length;
          const imgs = child.querySelectorAll('img[alt]').length;
          extra += ' ch[' + i + '](tag=' + child.tagName +
            ' spans=' + spans + ' imgs=' + imgs + ' text="' + txt + '")';
        }
      } catch (_) {}
    }
    api.log(
      'info',
      '[google-meet-recipe] diag: region found via ' +
        found.how +
        ' rows=' +
        rows.length +
        (rows.length
          ? ' sample="' +
            (rows[0].speaker + ': ' + rows[0].text).slice(0, 80) +
            '"'
          : '') +
        extra
    );
  }

  function captionRows() {
    const found = findCaptionRegion();
    if (!found) {
      maybeLogDiag(null, []);
      return [];
    }
    const region = found.region;
    const rows = [];
    try {
      const children = region.children;
      for (let i = 0; i < children.length; i++) {
        const row = children[i];
        const speaker = rowSpeaker(row);
        const text = rowText(row);
        if (!text) continue;
        // Reject toolbar icon ligatures, snackbar "closeClose" duplications,
        // and other non-caption chrome.
        if (!looksLikeCaptionLine(text)) continue;
        // A row without a real speaker avatar AND short text is almost
        // certainly chrome (tooltip, icon label). Keep it only if it has
        // enough length to plausibly be a caption line.
        if (speaker === 'Unknown' && text.length < 12) continue;
        rows.push({ speaker: speaker, text: text });
      }
    } catch (_) {}
    maybeLogDiag(found, rows);
    return rows;
  }

  function emitStarted(code) {
    startedAt = Date.now();
    api.log('info', '[google-meet-recipe] call started: ' + code);
    ssSet(SS_CODE, code);
    ssSet(SS_STARTED_AT, String(startedAt));
    try {
      api.emit('meet_call_started', {
        code: code,
        url: window.location.href,
        startedAt: startedAt,
      });
    } catch (_) {}
  }

  function emitEnded(code, reason) {
    const endedAt = Date.now();
    api.log(
      'info',
      '[google-meet-recipe] call ended: ' +
        code +
        ' reason=' +
        reason +
        ' duration_s=' +
        Math.round((endedAt - startedAt) / 1000)
    );
    ssDel(SS_CODE);
    ssDel(SS_STARTED_AT);
    try {
      api.emit('meet_call_ended', {
        code: code,
        endedAt: endedAt,
        reason: reason,
      });
    } catch (_) {}
  }

  // Recovery path: if Meet destroyed the previous recipe context before
  // we could emit call_ended (leave-call navigates the SPA), sessionStorage
  // still has the code. On bootstrap, if we find a stale code AND the
  // current page has no meeting code, flush the previous session.
  (function recoverStaleSession() {
    const staleCode = ssGet(SS_CODE);
    if (!staleCode) return;
    const liveCode = meetingCode();
    if (liveCode === staleCode) {
      // Page reload inside the same call — resume, don't flush.
      const staleStarted = parseInt(ssGet(SS_STARTED_AT) || '0', 10);
      startedAt = staleStarted || Date.now();
      currentCode = staleCode;
      return;
    }
    // Either the URL has no code (left the call) or a different code
    // (switched meetings). Either way, close out the previous one.
    const staleStarted = parseInt(ssGet(SS_STARTED_AT) || '0', 10);
    if (staleStarted) startedAt = staleStarted;
    emitEnded(staleCode, liveCode ? 'switched-on-reload' : 'navigated-away');
  })();

  function emitCaptionsIfChanged(code, captions) {
    const key = JSON.stringify(captions);
    if (key === lastCaptionsKey) return;
    lastCaptionsKey = key;
    try {
      api.emit('meet_captions', {
        code: code,
        captions: captions,
        ts: Date.now(),
      });
    } catch (_) {}
  }

  // Positive "we are in the call" signal. The URL keeps the meeting code
  // in the lobby and on the post-leave screen too, so URL alone is not
  // enough. Once you actually enter the meeting room, Meet renders one
  // participant tile per attendee (including your own), marked with
  // `[data-participant-id]` on the tile wrapper and `[data-self-name]`
  // on the own-user tile. Neither attribute is present in the lobby or
  // on the post-leave screen — their presence is the cleanest signal
  // that we're fully joined.
  function sawParticipantBubbles() {
    try {
      if (document.querySelector('[data-self-name]')) return true;
      if (document.querySelector('[data-participant-id]')) return true;
    } catch (_) {}
    return false;
  }

  function inCallNow() {
    const code = meetingCode();
    if (!code) return null;
    return sawParticipantBubbles() ? code : null;
  }

  api.loop(function () {
    const activeCode = inCallNow();

    // End: we had an active call, and the "Leave call" button is gone
    // (lobby page, post-leave screen, or SPA-nav to another route).
    if (currentCode && activeCode !== currentCode) {
      emitEnded(
        currentCode,
        activeCode ? 'switched' : 'leave-call-button-gone'
      );
      // If we jumped straight to a different meeting, fall through to
      // emit the new start on the same tick.
      currentCode = null;
      lastCaptionsKey = '';
    }

    // Start: we're in a call (URL matches AND Leave-call button visible)
    // and we hadn't marked ourselves in-call yet.
    if (activeCode && !currentCode) {
      currentCode = activeCode;
      lastCaptionsKey = '';
      emitStarted(activeCode);
    }

    if (!currentCode) return;
    const captions = captionRows();
    if (captions.length > 0) {
      emitCaptionsIfChanged(currentCode, captions);
    }
  });
})(window.__openhumanRecipe);
