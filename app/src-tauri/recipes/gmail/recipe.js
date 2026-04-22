// Gmail recipe — lightweight inbox-change signal emitter.
//
// This recipe's only job is to detect when the Gmail inbox list mutates
// (new message appears, read-state changes, label changes) and emit a
// minimal signal to the Rust side so it knows to run a targeted
// network / IDB scan pass. All heavy data extraction happens in the
// Rust-side `gmail_scanner` over CDP — this file deliberately contains
// no scraping logic.
(function (api) {
  if (!api) return;
  api.log('info', '[gmail-recipe] starting inbox signal emitter');

  var inboxNode = null;
  var observer = null;

  function startObserver() {
    // Gmail renders its inbox list as <tbody> or <div role="grid"> rows
    // inside a container with class 'ae4 UI'. We observe the closest
    // stable ancestor that wraps the thread-list. If the DOM structure
    // changes across Gmail redesigns this selector falls back to the body
    // so we still get *some* mutation signal.
    var target =
      document.querySelector('[role="main"]') ||
      document.querySelector('.ae4') ||
      document.body;

    if (!target || target === inboxNode) return; // already observing same node

    if (observer) observer.disconnect();
    inboxNode = target;

    observer = new MutationObserver(function () {
      api.log('debug', '[gmail-recipe] inbox mutation detected — emitting signal');
      api.ingest({ kind: 'signal', event: 'inbox_changed' });
    });

    observer.observe(target, { childList: true, subtree: true });
    api.log('info', '[gmail-recipe] observer attached to inbox container');
  }

  // Attempt to attach on first load and re-attach on SPA navigations.
  function tryAttach() {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', startObserver, { once: true });
    } else {
      startObserver();
    }
  }

  tryAttach();

  // Gmail is a SPA — re-check the target node every 5 s so the observer
  // stays valid across soft navigations (compose → inbox → label).
  setInterval(startObserver, 5000);
})(window.__openhumanRecipe);
