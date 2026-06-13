// PTTSpeaker.swift
// AVSpeechSynthesizer wrapper for TTS.

import AVFoundation
import os.log

private let log = Logger(subsystem: "ai.openhuman.ptt", category: "PTTSpeaker")

// AVSpeechSynthesizer is not Sendable; PTT operations are serialized by the
// plugin's command actor above, so the unchecked conformance is sound here.
final class PTTSpeaker: NSObject, AVSpeechSynthesizerDelegate, @unchecked Sendable {
    // MARK: - State

    private let synthesizer = AVSpeechSynthesizer()
    private var currentUtteranceId: String?

    /// Called when synthesis starts for an utterance.
    var onStarted: ((String) -> Void)?
    /// Called when synthesis finishes or is cancelled.
    /// `finished` is false when cancelled.
    var onEnded: ((String, Bool) -> Void)?

    override init() {
        super.init()
        synthesizer.delegate = self
    }

    // MARK: - Public API

    /// Enqueue text for synthesis.
    /// - Parameters:
    ///   - text: The text to speak.
    ///   - voiceId: Optional `AVSpeechSynthesisVoice.identifier`. Defaults to the
    ///     system's current language voice if nil.
    ///   - rate: Speech rate in [AVSpeechUtteranceMinimumSpeechRate,
    ///     AVSpeechUtteranceMaximumSpeechRate]. 0.5 = default rate.
    func speak(text: String, voiceId: String?, rate: Float?) {
        log.info("[ptt] PTTSpeaker: speak text_len=\(text.count) voiceId=\(voiceId ?? "default")")

        let utterance = AVSpeechUtterance(string: text)

        if let voiceId {
            utterance.voice = AVSpeechSynthesisVoice(identifier: voiceId)
        } else {
            // Default: use the voice matching the device's current locale.
            utterance.voice = AVSpeechSynthesisVoice(language: Locale.current.language.languageCode?.identifier ?? "en")
        }

        // Map the caller's normalized rate (0.5–2.0) to AVFoundation's scale.
        // AVSpeechUtteranceDefaultSpeechRate == 0.5 on the [0,1] AVFoundation scale.
        if let rate {
            let clamped = min(max(rate, 0.1), 2.0)
            // Rough mapping: caller's 1.0 → AVFoundation 0.5 (default)
            utterance.rate = AVSpeechUtteranceDefaultSpeechRate * clamped
        } else {
            utterance.rate = AVSpeechUtteranceDefaultSpeechRate
        }

        let uid = UUID().uuidString
        currentUtteranceId = uid
        // Store id on the utterance so the delegate can recover it.
        // AVSpeechUtterance doesn't have a built-in id field so we embed it
        // in the speech string's associated object via objc runtime — or,
        // simpler: since we track currentUtteranceId and replace it per
        // utterance, and we only queue one at a time, the delegate receives
        // the most-recently-set id.
        synthesizer.speak(utterance)
        log.debug("[ptt] PTTSpeaker: utterance enqueued uid=\(uid)")
    }

    /// Immediately stop synthesis at the word boundary.
    func cancel() {
        log.info("[ptt] PTTSpeaker: cancel")
        synthesizer.stopSpeaking(at: .immediate)
    }

    /// Return all on-device voices.
    func listVoices() -> [[String: String]] {
        return AVSpeechSynthesisVoice.speechVoices().map { voice in
            [
                "id": voice.identifier,
                "name": voice.name,
                "lang": voice.language,
            ]
        }
    }

    // MARK: - AVSpeechSynthesizerDelegate

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didStart utterance: AVSpeechUtterance) {
        let uid = currentUtteranceId ?? "unknown"
        log.info("[ptt] PTTSpeaker: synthesis started uid=\(uid)")
        onStarted?(uid)
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        let uid = currentUtteranceId ?? "unknown"
        log.info("[ptt] PTTSpeaker: synthesis finished uid=\(uid)")
        currentUtteranceId = nil
        onEnded?(uid, true)
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        let uid = currentUtteranceId ?? "unknown"
        log.info("[ptt] PTTSpeaker: synthesis cancelled uid=\(uid)")
        currentUtteranceId = nil
        onEnded?(uid, false)
    }
}
