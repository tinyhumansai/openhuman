// AudioSessionManager.swift
// Manages AVAudioSession category, activation, and notification handling.
//
// Pattern adapted from chat4000/Sources/Services/VoiceNotes.swift:
//   session.setCategory(.playAndRecord, mode: .spokenAudio,
//                       options: [.defaultToSpeaker, .allowBluetoothA2DP])

import AVFoundation
import os.log

private let log = Logger(subsystem: "ai.openhuman.ptt", category: "AudioSessionManager")

/// Centralises AVAudioSession lifecycle so PTTRecorder and PTTSpeaker
/// share a single category configuration. Activating the session once
/// for both recording and playback avoids category-flip glitches on BT.
final class AudioSessionManager {
    static let shared = AudioSessionManager()
    private init() {}

    private var interruptionObserver: NSObjectProtocol?
    private var routeChangeObserver: NSObjectProtocol?

    /// Called once by PTTPlugin to wire up system notifications.
    /// `onInterrupted` fires when a phone call or system audio takes over.
    /// `onRouteChange` fires when BT headset connects / disconnects.
    func startObserving(
        onInterrupted: @escaping () -> Void,
        onRouteChange: @escaping (AVAudioSession.RouteChangeReason) -> Void
    ) {
        let nc = NotificationCenter.default

        interruptionObserver = nc.addObserver(
            forName: AVAudioSession.interruptionNotification,
            object: nil,
            queue: .main
        ) { notification in
            guard
                let info = notification.userInfo,
                let typeValue = info[AVAudioSessionInterruptionTypeKey] as? UInt,
                let type = AVAudioSession.InterruptionType(rawValue: typeValue)
            else { return }

            if type == .began {
                log.info("[ptt] audio session interrupted — began")
                onInterrupted()
            } else {
                log.debug("[ptt] audio session interruption ended")
            }
        }

        routeChangeObserver = nc.addObserver(
            forName: AVAudioSession.routeChangeNotification,
            object: nil,
            queue: .main
        ) { notification in
            guard
                let info = notification.userInfo,
                let reasonValue = info[AVAudioSessionRouteChangeReasonKey] as? UInt,
                let reason = AVAudioSession.RouteChangeReason(rawValue: reasonValue)
            else { return }

            log.info("[ptt] audio route changed reason=\(reason.rawValue)")
            onRouteChange(reason)
        }

        log.debug("[ptt] AudioSessionManager: observers registered")
    }

    func stopObserving() {
        let nc = NotificationCenter.default
        if let obs = interruptionObserver { nc.removeObserver(obs) }
        if let obs = routeChangeObserver { nc.removeObserver(obs) }
        interruptionObserver = nil
        routeChangeObserver = nil
        log.debug("[ptt] AudioSessionManager: observers removed")
    }

    // MARK: - Session activation

    /// Activate the shared session for recording + playback.
    /// Category: .playAndRecord, mode: .spokenAudio
    /// Options: .defaultToSpeaker, .allowBluetooth, .allowBluetoothA2DP
    func activateForRecording() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(
            .playAndRecord,
            mode: .spokenAudio,
            options: [.defaultToSpeaker, .allowBluetooth, .allowBluetoothA2DP]
        )
        try session.setActive(true)
        log.info("[ptt] audio session activated for recording")
    }

    /// Deactivate the session, notifying other apps they can resume.
    func deactivate() {
        do {
            try AVAudioSession.sharedInstance().setActive(
                false,
                options: .notifyOthersOnDeactivation
            )
            log.info("[ptt] audio session deactivated")
        } catch {
            log.error("[ptt] audio session deactivate error: \(error.localizedDescription)")
        }
    }
}
