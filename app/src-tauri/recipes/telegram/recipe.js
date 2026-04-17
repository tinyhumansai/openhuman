// Telegram Web recipe. Scrapes the chat list in the left pane.
(function (api) {
  if (!api) return;
  api.log('info', '[telegram-recipe] starting');

  let last = '';

  function textOf(el) {
    return (el && el.textContent ? el.textContent : '').trim();
  }

  api.loop(function () {
    const rows = document.querySelectorAll('.chatlist .chatlist-chat, ul.chatlist > li');
    if (!rows || rows.length === 0) return;

    const messages = [];
    let totalUnread = 0;

    rows.forEach(function (row, idx) {
      const nameEl =
        row.querySelector('.user-title, .peer-title') || row.querySelector('.dialog-title span');
      const previewEl =
        row.querySelector('.dialog-subtitle') || row.querySelector('.user-last-message');
      const badgeEl = row.querySelector('.badge-unread, .dialog-subtitle-badge-unread');
      const name = textOf(nameEl);
      const preview = textOf(previewEl);
      const badgeNum = parseInt(textOf(badgeEl), 10);
      const unread = Number.isNaN(badgeNum) ? 0 : badgeNum;
      if (unread > 0) totalUnread += unread;
      if (name || preview) {
        messages.push({
          id: name ? 'tg:' + name : 'tg:row:' + idx,
          from: name || null,
          body: preview || null,
          unread: unread,
        });
      }
    });

    const key = JSON.stringify({
      n: messages.length,
      u: totalUnread,
      first: messages.slice(0, 5).map(function (m) { return m.from + '|' + m.body + '|' + m.unread; }),
    });
    if (key === last) return;
    last = key;

    api.ingest({ messages: messages, unread: totalUnread, snapshotKey: key });
  });
})(window.__openhumanRecipe);
