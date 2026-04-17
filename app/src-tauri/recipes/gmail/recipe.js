// Gmail recipe. Scrapes the inbox row list (tr.zA).
// Note: Google may block embedded WebViews with a "browser not supported"
// page. That's an auth problem, not a recipe problem — once the user is
// signed in, this scraper picks up the rows.
(function (api) {
  if (!api) return;
  api.log('info', '[gmail-recipe] starting');

  let last = '';

  function textOf(el) {
    return (el && el.textContent ? el.textContent : '').trim();
  }

  api.loop(function () {
    const rows = document.querySelectorAll('tr.zA');
    if (!rows || rows.length === 0) return;

    const messages = [];
    let totalUnread = 0;

    rows.forEach(function (row, idx) {
      const stableId =
        (row.getAttribute && row.getAttribute('data-legacy-message-id')) || null;
      const fromEl =
        row.querySelector('.yW span[email]') ||
        row.querySelector('.yW .yP') ||
        row.querySelector('.yW .zF') ||
        row.querySelector('.yW span');
      const subjEl = row.querySelector('.y6 span.bog') || row.querySelector('.y6 span');
      const snippetEl = row.querySelector('.y2');

      const from = fromEl
        ? (fromEl.getAttribute && fromEl.getAttribute('name')) || textOf(fromEl)
        : '';
      const subject = textOf(subjEl);
      const snippet = textOf(snippetEl);
      const isUnread = row.classList && row.classList.contains('zE');
      if (isUnread) totalUnread += 1;

      if (from || subject) {
        messages.push({
          id: 'gm:' + (stableId || (from + '|' + subject).slice(0, 120) || idx),
          from: from || null,
          body: subject + (snippet ? ' — ' + snippet : ''),
          unread: isUnread ? 1 : 0,
        });
      }
    });

    const key = JSON.stringify({
      n: messages.length,
      u: totalUnread,
      first: messages.slice(0, 5).map(function (m) { return m.from + '|' + m.body; }),
    });
    if (key === last) return;
    last = key;

    api.ingest({ messages: messages, unread: totalUnread, snapshotKey: key });
  });
})(window.__openhumanRecipe);
