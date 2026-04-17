// Discord recipe. Scrapes the channel / DM sidebar for names and unread badges.
// Discord's web app uses hashed CSS class names (e.g. `link-abcd`) so we lean
// on stable ARIA roles and `data-list-item-id` attributes where available.
(function (api) {
  if (!api) return;
  api.log('info', '[discord-recipe] starting');

  let last = '';

  function textOf(el) {
    return (el && el.textContent ? el.textContent : '').trim();
  }

  api.loop(function () {
    const rows = document.querySelectorAll(
      '[role="treeitem"][data-list-item-id], [data-list-item-id^="channels"], [data-list-item-id^="private-channels"]'
    );
    if (!rows || rows.length === 0) return;

    const messages = [];
    let totalUnread = 0;

    rows.forEach(function (row, idx) {
      const nameEl =
        row.querySelector('[class*="name_"]') ||
        row.querySelector('[class*="channelName_"]') ||
        row.querySelector('a [class*="overflow"]') ||
        row.querySelector('a');
      const badgeEl =
        row.querySelector('[class*="numberBadge_"]') ||
        row.querySelector('[class*="unread_"]') ||
        row.querySelector('[aria-label*="unread" i]');
      const name = textOf(nameEl);
      const badgeNum = parseInt(textOf(badgeEl), 10);
      const unread = Number.isNaN(badgeNum) ? 0 : badgeNum;
      if (unread > 0) totalUnread += unread;
      if (name) {
        messages.push({
          id: 'dc:' + name + ':' + idx,
          from: name,
          body: null,
          unread: unread,
        });
      }
    });

    const key = JSON.stringify({
      n: messages.length,
      u: totalUnread,
      first: messages.slice(0, 8).map(function (m) { return m.from + '|' + m.unread; }),
    });
    if (key === last) return;
    last = key;

    api.ingest({ messages: messages, unread: totalUnread, snapshotKey: key });
  });
})(window.__openhumanRecipe);
