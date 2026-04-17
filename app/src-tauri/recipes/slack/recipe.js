// Slack recipe. Scrapes the channel sidebar for channel names + unread badges.
// Slack's app is a single-page workspace app; this recipe stays lightweight
// and just reports the channel list — we can add message-body scraping
// later once we decide what the agent should see.
(function (api) {
  if (!api) return;
  api.log('info', '[slack-recipe] starting');

  let last = '';

  function textOf(el) {
    return (el && el.textContent ? el.textContent : '').trim();
  }

  api.loop(function () {
    const rows = document.querySelectorAll(
      '[data-qa="virtual-list-item"], .p-channel_sidebar__channel'
    );
    if (!rows || rows.length === 0) return;

    const messages = [];
    let totalUnread = 0;

    rows.forEach(function (row, idx) {
      const nameEl =
        row.querySelector('[data-qa="channel_sidebar_name_button"]') ||
        row.querySelector('.p-channel_sidebar__name') ||
        row.querySelector('span');
      const badgeEl = row.querySelector('.p-channel_sidebar__badge, [data-qa="mention_badge"]');
      const name = textOf(nameEl);
      const badgeNum = parseInt(textOf(badgeEl), 10);
      const unread = Number.isNaN(badgeNum) ? 0 : badgeNum;
      if (unread > 0) totalUnread += unread;
      if (name) {
        messages.push({
          id: 'sl:' + name + ':' + idx,
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
