// Zoom recipe.
//
// Scope (V1 — passive shell):
//   * Load inside embedded webview, log bootstrap, reserve event-name
//     namespace for future meeting-lifecycle + caption capture.
//
// Event kinds reserved (not yet emitted — see follow-up #TBD):
//   zoom_call_started  { meetingId, url, startedAt }
//   zoom_captions      { meetingId, captions:[{speaker,text}], ts }
//   zoom_call_ended    { meetingId, endedAt, reason }
//
// Zoom WebClient DOM is not yet mapped. Caption DOM scraping requires
// live-call investigation; shipping as stub so account login works now and
// the extractor can land incrementally without churn on shared wiring.
(function (api) {
  if (!api) return;
  api.log('info', '[zoom-recipe] loaded (passive shell; lifecycle + captions TBD)');
})(window.__openhumanRecipe);
