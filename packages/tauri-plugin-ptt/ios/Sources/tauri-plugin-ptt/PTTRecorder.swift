// PTTRecorder.swift
// AVAudioEngine + SFSpeechRecognizer pipeline for push-to-talk recording.
//
// Mic permission pattern from chat4000/Sources/Services/VoiceNotes.swift:
//   AVAudioApplication.requestRecordPermission (iOS 17+)
//   AVAudioSession.sharedInstance().requestRecordPermission (older)

import AVFoundation
import os.log
import Speech

private let log = Logger(subsystem: "ai.openhuman.ptt", category: "PTTRecorder")

/// Single-session AVAudioEngine + SFSpeechRecognizer recorder.
/// One `startListening` call creates one recognition task; `stopListening`
/// tears it down. Never keeps a task running between sessions.
final class PTTRecorder {
    // MARK: - Types

    enum RecorderError: Error, LocalizedError {
        case microphonePermissionDenied
        case speechPermissionDenied
        case alreadyRecording
        case notRecording
        case audioEngineError(String)
        case recognizerUnavailable

        var errorDescription: String? {
            switch self {
            case .microphonePermissionDenied: return "Microphone permission denied"
            case .speechPermissionDenied: return "Speech recognition permission denied"
            case .alreadyRecording: return "Recording already active"
            case .notRecording: return "No active recording session"
            case .audioEngineError(let msg): return "Audio engine error: \(msg)"
            case .recognizerUnavailable: return "Speech recognizer unavailable for current locale"
            }
        }
    }

    // MARK: - State

    private let engine = AVAudioEngine()
    private var recognitionRequest: SFSpeechAudioBufferRecognitionRequest?
    private var recognitionTask: SFSpeechRecognitionTask?
    private var recognizer: SFSpeechRecognizer?

    // Latest partial transcript captured inside the recognitionTask result
    // handler. SFSpeechRecognitionTask exposes no `result` property, so we
    // mirror it here for stopListening() to read on tear-down.
    private var latestTranscript = ""

    private var isRecording = false

    /// Emitted for each partial result while the user speaks.
    var onPartialTranscript: ((String) -> Void)?
    /// Emitted on any async error (permission denial, interruption, etc.).
    var onError: ((String, String) -> Void)?

    // MARK: - Permissions

    /// Returns true if both microphone and speech recognition are authorized.
    func requestPermissions() async -> Result<Void, RecorderError> {
        log.debug("[ptt] PTTRecorder: requesting microphone permission")
        let micGranted = await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            if #available(iOS 17.0, *) {
                AVAudioApplication.requestRecordPermission { granted in
                    continuation.resume(returning: granted)
                }
            } else {
                AVAudioSession.sharedInstance().requestRecordPermission { granted in
                    continuation.resume(returning: granted)
                }
            }
        }

        guard micGranted else {
            log.error("[ptt] PTTRecorder: microphone permission denied")
            return .failure(.microphonePermissionDenied)
        }

        log.debug("[ptt] PTTRecorder: requesting speech recognition permission")
        let speechStatus = await withCheckedContinuation { (continuation: CheckedContinuation<SFSpeechRecognizerAuthorizationStatus, Never>) in
            SFSpeechRecognizer.requestAuthorization { status in
                continuation.resume(returning: status)
            }
        }

        guard speechStatus == .authorized else {
            log.error("[ptt] PTTRecorder: speech permission denied status=\(speechStatus.rawValue)")
            return .failure(.speechPermissionDenied)
        }

        log.info("[ptt] PTTRecorder: all permissions granted")
        return .success(())
    }

    // MARK: - Recording lifecycle

    /// Start a new recording + recognition session.
    /// Partial transcripts are delivered via `onPartialTranscript`.
    func startListening() async throws {
        guard !isRecording else {
            log.warning("[ptt] PTTRecorder: startListening called while already recording")
            throw RecorderError.alreadyRecording
        }

        log.info("[ptt] PTTRecorder: startListening — requesting permissions")
        let permResult = await requestPermissions()
        switch permResult {
        case .failure(let err):
            throw err
        case .success:
            break
        }

        let locale = Locale.current
        recognizer = SFSpeechRecognizer(locale: locale)
        guard let recognizer, recognizer.isAvailable else {
            log.error("[ptt] PTTRecorder: SFSpeechRecognizer unavailable locale=\(locale.identifier)")
            throw RecorderError.recognizerUnavailable
        }

        log.debug("[ptt] PTTRecorder: activating audio session")
        try AudioSessionManager.shared.activateForRecording()

        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        // Keep audio only for recognition — do not save to disk.
        request.requiresOnDeviceRecognition = false
        recognitionRequest = request

        let inputNode = engine.inputNode
        let format = inputNode.outputFormat(forBus: 0)

        inputNode.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
            self?.recognitionRequest?.append(buffer)
        }

        engine.prepare()
        do {
            try engine.start()
        } catch {
            log.error("[ptt] PTTRecorder: engine start failed: \(error.localizedDescription)")
            cleanupEngine()
            throw RecorderError.audioEngineError(error.localizedDescription)
        }

        recognitionTask = recognizer.recognitionTask(with: request) { [weak self] result, error in
            guard let self else { return }

            if let result {
                let text = result.bestTranscription.formattedString
                self.latestTranscript = text
                log.debug("[ptt] PTTRecorder: partial text_len=\(text.count)")
                self.onPartialTranscript?(text)
            }

            if let error {
                // Cancellation is not an error — the task ends when we
                // call stopListening or when the user stops speaking.
                let nsErr = error as NSError
                let isCancelled = nsErr.domain == "kAFAssistantErrorDomain" && nsErr.code == 209
                let isNoSpeech = nsErr.domain == "kAFAssistantErrorDomain" && nsErr.code == 1110
                if !isCancelled && !isNoSpeech {
                    log.error("[ptt] PTTRecorder: recognition error: \(error.localizedDescription)")
                    self.onError?("recognition_error", error.localizedDescription)
                }
            }
        }

        isRecording = true
        log.info("[ptt] PTTRecorder: recording started")
    }

    /// Stop the active session and return the final transcript text.
    /// Tears down the engine and recognition task regardless of outcome.
    func stopListening() -> String {
        guard isRecording else {
            log.warning("[ptt] PTTRecorder: stopListening called with no active session")
            return ""
        }

        log.info("[ptt] PTTRecorder: stopListening")

        // Signal end-of-audio to the recognizer before stopping the engine
        // so the recognizer can finalize with what it has already buffered.
        recognitionRequest?.endAudio()
        recognitionTask?.finish()

        let finalText = latestTranscript
        latestTranscript = ""
        log.debug("[ptt] PTTRecorder: final text_len=\(finalText.count)")

        cleanupEngine()
        AudioSessionManager.shared.deactivate()
        isRecording = false

        return finalText
    }

    // MARK: - Cleanup

    private func cleanupEngine() {
        engine.inputNode.removeTap(onBus: 0)
        if engine.isRunning {
            engine.stop()
        }
        recognitionRequest = nil
        recognitionTask = nil
        recognizer = nil
        log.debug("[ptt] PTTRecorder: engine and task cleaned up")
    }

    /// Force-stop without waiting for a final result. Called on app backgrounding
    /// or audio session interruption.
    func forceStop() {
        guard isRecording else { return }
        log.info("[ptt] PTTRecorder: forceStop")
        recognitionRequest?.endAudio()
        recognitionTask?.cancel()
        cleanupEngine()
        latestTranscript = ""
        AudioSessionManager.shared.deactivate()
        isRecording = false
    }

    var active: Bool { isRecording }
}
