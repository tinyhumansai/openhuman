// PTTPlugin.swift
// Tauri v2 plugin class. Bridges Rust commands to PTTRecorder / PTTSpeaker.
//
// Command names must match the Rust command names in commands.rs
// (Tauri converts snake_case to camelCase for the Swift @objc method).

import AVFoundation
import os.log
import Tauri
import UIKit
import WebKit

private let log = Logger(subsystem: "ai.openhuman.ptt", category: "PTTPlugin")

// MARK: - Codable payload types (mirror models.rs)

private struct TranscriptResult: Encodable {
    let text: String
    let isFinal: Bool
}

private struct VoiceInfoPayload: Encodable {
    let id: String
    let name: String
    let lang: String
}

private struct SpeakArgs: Decodable {
    let text: String
    let voiceId: String?
    let rate: Float?
}

// MARK: - PTTPlugin

class PTTPlugin: Plugin {
    private let recorder = PTTRecorder()
    private let speaker = PTTSpeaker()

    override func load(webview: WKWebView) {
        super.load(webview: webview)
        log.info("[ptt] PTTPlugin: load — wiring audio session observers")

        AudioSessionManager.shared.startObserving(
            onInterrupted: { [weak self] in
                self?.handleInterruption()
            },
            onRouteChange: { [weak self] reason in
                self?.handleRouteChange(reason: reason)
            }
        )

        recorder.onPartialTranscript = { [weak self] text in
            log.debug("[ptt] PTTPlugin: partial transcript text_len=\(text.count)")
            self?.trigger("ptt://transcript-partial", data: ["text": text])
        }

        recorder.onError = { [weak self] code, message in
            log.error("[ptt] PTTPlugin: async error code=\(code) message=\(message)")
            self?.trigger("ptt://error", data: ["code": code, "message": message])
        }

        speaker.onStarted = { [weak self] uid in
            log.debug("[ptt] PTTPlugin: tts started uid=\(uid)")
            self?.trigger("ptt://tts-started", data: ["utteranceId": uid])
        }

        speaker.onEnded = { [weak self] uid, finished in
            log.debug("[ptt] PTTPlugin: tts ended uid=\(uid) finished=\(finished)")
            self?.trigger("ptt://tts-ended", data: ["utteranceId": uid, "finished": finished])
        }

        // Cancel active speech when the app backgrounds so the OS audio
        // session can be released cleanly.
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(appDidBackground),
            name: UIApplication.didEnterBackgroundNotification,
            object: nil
        )
    }

    // MARK: - Commands (called by Tauri runtime via @objc)

    @objc func startListening(_ invoke: Invoke) {
        log.info("[ptt] PTTPlugin: startListening command received")
        Task {
            do {
                try await self.recorder.startListening()
                log.debug("[ptt] PTTPlugin: startListening succeeded")
                invoke.resolve()
            } catch {
                log.error("[ptt] PTTPlugin: startListening error: \(error.localizedDescription)")
                invoke.reject(error.localizedDescription)
                self.emitPermissionOrAudioError(from: error)
            }
        }
    }

    @objc func stopListening(_ invoke: Invoke) {
        log.info("[ptt] PTTPlugin: stopListening command received")
        let finalText = recorder.stopListening()
        log.debug("[ptt] PTTPlugin: stopListening final text_len=\(finalText.count)")
        trigger("ptt://transcript-final", data: ["text": finalText])
        let result = TranscriptResult(text: finalText, isFinal: true)
        invoke.resolve(result)
    }

    @objc func speak(_ invoke: Invoke) {
        log.info("[ptt] PTTPlugin: speak command received")
        do {
            let args = try invoke.parseArgs(SpeakArgs.self)
            log.debug("[ptt] PTTPlugin: speak text_len=\(args.text.count)")
            speaker.speak(text: args.text, voiceId: args.voiceId, rate: args.rate)
            invoke.resolve()
        } catch {
            log.error("[ptt] PTTPlugin: speak parse error: \(error.localizedDescription)")
            invoke.reject(error.localizedDescription)
        }
    }

    @objc func cancelSpeech(_ invoke: Invoke) {
        log.info("[ptt] PTTPlugin: cancelSpeech command received")
        speaker.cancel()
        invoke.resolve()
    }

    @objc func listVoices(_ invoke: Invoke) {
        log.info("[ptt] PTTPlugin: listVoices command received")
        let voices = speaker.listVoices().map { v in
            VoiceInfoPayload(
                id: v["id"] ?? "",
                name: v["name"] ?? "",
                lang: v["lang"] ?? ""
            )
        }
        log.debug("[ptt] PTTPlugin: listVoices count=\(voices.count)")
        invoke.resolve(voices)
    }

    // MARK: - Internal event helpers

    private func emitPermissionOrAudioError(from error: Error) {
        let (code, message): (String, String)
        switch error {
        case PTTRecorder.RecorderError.microphonePermissionDenied:
            code = "permission_denied"
            message = "Microphone access was denied. Enable it in Settings."
        case PTTRecorder.RecorderError.speechPermissionDenied:
            code = "permission_denied"
            message = "Speech recognition was denied. Enable it in Settings."
        default:
            code = "audio_error"
            message = error.localizedDescription
        }
        trigger("ptt://error", data: ["code": code, "message": message])
    }

    // MARK: - Session interruption / route change

    private func handleInterruption() {
        log.warning("[ptt] PTTPlugin: audio interrupted — stopping recorder")
        if recorder.active {
            let finalText = recorder.stopListening()
            trigger("ptt://transcript-final", data: ["text": finalText])
        }
        trigger("ptt://error", data: [
            "code": "interrupted",
            "message": "Audio session was interrupted by another app or call.",
        ])
    }

    private func handleRouteChange(reason: AVAudioSession.RouteChangeReason) {
        // BT device unplugged mid-recording — stop gracefully.
        if reason == .oldDeviceUnavailable && recorder.active {
            log.warning("[ptt] PTTPlugin: route changed (device unavailable) — stopping recorder")
            let finalText = recorder.stopListening()
            trigger("ptt://transcript-final", data: ["text": finalText])
            trigger("ptt://error", data: [
                "code": "route_changed",
                "message": "Audio output device disconnected.",
            ])
        }
    }

    // MARK: - Background handling

    @objc private func appDidBackground() {
        log.info("[ptt] PTTPlugin: app backgrounded — stopping recorder if active")
        if recorder.active {
            let finalText = recorder.stopListening()
            trigger("ptt://transcript-final", data: ["text": finalText])
        }
        speaker.cancel()
    }
}

// MARK: - Plugin factory

/// Entry point called by `tauri::ios_plugin_binding!(init_plugin_ptt)`.
@_cdecl("init_plugin_ptt")
func initPlugin() -> Plugin {
    log.debug("[ptt] init_plugin_ptt — returning PTTPlugin instance")
    return PTTPlugin()
}
