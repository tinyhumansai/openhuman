// LinkedIn Messaging recipe. Scrapes the conversation list panel.
(function (api) {
  if (!api) return;
  api.log('info', '[linkedin-recipe] starting');

  let last = '';

  function textOf(el) {
    return (el && el.textContent ? el.textContent : '').trim();
  }

  api.loop(function () {
    const rows = document.querySelectorAll(
      'li.msg-conversation-listitem, .msg-conversations-container__pillar li'
    );
    if (!rows || rows.length === 0) return;

    const messages = [];
    rows.forEach(function (row, idx) {
      const nameEl =
        row.querySelector('.msg-conversation-listitem__participant-names') ||
        row.querySelector('.msg-conversation-card__participant-names') ||
        row.querySelector('h3');
      const previewEl =
        row.querySelector('.msg-conversation-card__message-snippet') ||
        row.querySelector('.msg-conversation-listitem__message-snippet');
      const unreadEl = row.querySelector('.notification-badge__count, .msg-conversation-card__unread-count');
      const name = textOf(nameEl);
      const preview = textOf(previewEl);
      const unreadNum = parseInt(textOf(unreadEl), 10);
      const unread = Number.isNaN(unreadNum) ? 0 : unreadNum;
      if (name || preview) {
        messages.push({
          id: name ? 'li:' + name : 'li:row:' + idx,
          from: name || null,
          body: preview || null,
          unread: unread,
        });
      }
    });

    const key = JSON.stringify({
      n: messages.length,
      first: messages.slice(0, 5).map(function (m) { return m.from + '|' + m.body; }),
    });
    if (key === last) return;
    last = key;

    api.ingest({ messages: messages, snapshotKey: key });
  });
})(window.__openhumanRecipe);
