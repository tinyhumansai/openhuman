#!/usr/bin/env bash

create-dmg \
    --volname "OpenHuman installer" \
    --volicon "./src-tauri/icons/icon.icns" \
    --background "./src-tauri/images/background-dmg.svg" \
    --window-size 540 380 \
    --icon-size 100 \
    --icon "OpenHuman.app" 138 225 \
    --hide-extension "OpenHuman.app" \
    --app-drop-link 402 225 \
    --no-internet-enable \
    "$1" \
    "$2"
