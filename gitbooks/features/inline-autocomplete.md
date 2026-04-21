---
icon: rotate
---

# Inline Autocomplete

Inline Autocomplete suggests text completions while you type in any application. Powered by your local AI model and your memory context.

<figure><img src="../.gitbook/assets/6. Inline Autocomplete@2x.png" alt=""><figcaption></figcaption></figure>

#### How It Works

As you type, OpenHuman watches for pauses (controlled by debounce). After you pause, it generates a completion based on what you have typed plus relevant context from memory. Press Tab to accept.

The more you accept suggestions, the more it learns your style through Personalization History.

#### Runtime Info

The settings page shows: platform support, enabled/running status, current phase, debounce timing, active model (e.g., gemma3:12b-it-q4\_K\_M), last error, and current suggestion. Start/Stop buttons available.

#### Settings

Navigate to Settings > Automation & Channels > Inline Autocomplete.

**Enabled:** Master toggle.

**Accept With Tab:** Accept suggestions by pressing Tab. On by default.

**Debounce (ms):** Wait time after typing stops before generating a suggestion. Default: 120ms.

**Max Chars:** Maximum characters per suggestion. Default: 384.

**Style Preset:** Controls tone and style. Default: "Balanced."

**Style Instructions:** Free-text field for your preferred writing style (e.g., "Write in short, direct sentences. Avoid formal language.").

**Disabled Apps:** One bundle/app token per line. Autocomplete won't activate in listed apps.

Click "Save Autocomplete Settings" after changes.

#### Personalization History

Tracks accepted completions to improve future suggestions. Clear History button resets personalization. Begins once you accept your first suggestion.

#### Testing

Test section with optional Context Override field. Buttons: Get Suggestion, Accept Suggestion, Debug Focus.

#### Live Logs

Real-time log showing autocomplete events with timestamps. Clear button to reset.
