# App Store Listing

This is the prepared product-page kit for the OpenHuman iOS companion app.

## App Store Connect Fields

- App name: `OpenHuman`
- Subtitle: `AI companion for your desktop`
- Bundle ID: `com.tinyhumansai.openhuman`
- SKU: `com.tinyhumansai.openhuman`
- Primary category: `Productivity`
- Secondary category: `Utilities`
- Copyright: `2026 Tiny Humans AI`
- Support URL: `https://tinyhumans.ai/openhuman`
- Marketing URL: `https://tinyhumans.ai/openhuman`
- Privacy Policy URL: `https://tinyhumans.gitbook.io/openhuman/legal/privacy-policy`

Metadata files live in `fastlane/metadata/en-US/`.

## Description

Use `fastlane/metadata/en-US/description.txt`.

## Keywords

Use `fastlane/metadata/en-US/keywords.txt`.

## App Icon

The App Store icon is already included in the iOS build:

- `app/src-tauri-mobile/icons/store/appstore.png`
- `app/src-tauri-mobile/icons/ios/AppIcon.appiconset/1024.png`

Both are 1024 x 1024 PNG assets.

## Screenshots

Generate the 6.9-inch iPhone screenshot set:

```bash
pnpm --dir app exec node ../scripts/ios-appstore-assets.mjs
```

Upload the generated files from:

```text
fastlane/screenshots/en-US/
```

Use these in App Store Connect under the iPhone 6.9-inch display screenshot slot.

You can also push the generated screenshots with Fastlane:

```bash
ASC_KEY_ID=9KD934428C \
ASC_ISSUER_ID=69a6de8b-cc07-47e3-e053-5b8c7c11a4d1 \
ASC_KEY_PATH=/Users/enamakel/Downloads/AuthKey_9KD934428C.p8 \
ASC_APP_VERSION=1.0 \
fastlane ios push_screenshots
```

Metadata can be pushed with the App Store Connect API helper:

```bash
ASC_KEY_ID=9KD934428C \
ASC_ISSUER_ID=69a6de8b-cc07-47e3-e053-5b8c7c11a4d1 \
ASC_KEY_PATH=/Users/enamakel/Downloads/AuthKey_9KD934428C.p8 \
ASC_APP_ID=6761229174 \
scripts/ios-appstore-metadata.mjs
```

## App Review Notes

Use `fastlane/metadata/review_information/notes.txt`.

Before submitting, replace the TODO contact fields in:

- `fastlane/metadata/review_information/email_address.txt`
- `fastlane/metadata/review_information/phone_number.txt`

## Privacy Answers Draft

Use this as the starting point for App Privacy in App Store Connect:

- Camera: used to scan the desktop pairing QR code.
- Microphone: used for push-to-talk voice messages.
- Speech recognition: used to transcribe voice messages.
- Identifiers/session data: pairing/session data may be used to connect the phone to the paired OpenHuman desktop runtime.
- User content: messages and voice transcripts are sent to the paired OpenHuman runtime to provide assistant responses.

Before submitting, make sure the App Privacy answers match the production backend/runtime behavior.

## Current Apple Requirements Checked

Apple currently requires one to ten screenshots for each platform localization, accepts `.jpeg`, `.jpg`, and `.png`, and allows high-resolution iPhone screenshots to scale down to smaller sizes. Apple lists the 6.9-inch portrait sizes as accepted high-resolution iPhone screenshot sizes.

Apple currently limits:

- App name: 30 characters
- Subtitle: 30 characters
- Promotional text: 170 characters
- Description: 4000 characters
- Keywords: 100 bytes
