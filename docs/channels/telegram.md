# Telegram Channel

The Telegram channel allows OpenHuman to interact with users via a Telegram bot.

## Silent Streaming

While the bot is thinking or streaming a reply, updates are sent silently by default to minimize notification spam on the user's device. This means:
- The initial "thinking..." placeholder is sent without a notification sound.
- Intermediate streaming updates (edits to the message) do not trigger new notifications.
- Standalone messages and final fallback messages (if a message needs to be re-sent instead of edited) will still trigger a notification normally.

This behavior can be controlled via the `silent_streaming` option in the `[channels.telegram]` section of `config.toml`. It defaults to `true`.

```toml
[channels.telegram]
bot_token = "YOUR_BOT_TOKEN"
allowed_users = ["your_username"]
silent_streaming = true # Set to false to receive notifications for every update
```
