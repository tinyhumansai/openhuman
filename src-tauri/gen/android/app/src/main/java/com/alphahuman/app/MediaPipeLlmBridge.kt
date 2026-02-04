package com.alphahuman.app

import android.content.Context
import android.util.Log
import com.google.mediapipe.tasks.genai.llminference.LlmInference
import com.google.mediapipe.tasks.genai.llminference.LlmInference.LlmInferenceOptions
import org.json.JSONObject
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

/**
 * MediaPipe LLM Inference Bridge for Android
 *
 * Provides a JNI-accessible interface to MediaPipe's LLM Inference API for the Rust backend.
 * Enables on-device LLM inference using Google's MediaPipe framework.
 *
 * Supported models: Gemma 3n, Gemma 2, Phi-2, Falcon, StableLM
 * See: https://ai.google.dev/edge/mediapipe/solutions/genai/llm_inference/android
 */
object MediaPipeLlmBridge {
    private const val TAG = "MediaPipeLlmBridge"

    // LLM Inference instance (singleton)
    private var llmInference: LlmInference? = null

    // Application context reference
    private var appContext: Context? = null

    // Current model path
    private var currentModelPath: String? = null

    // Loading state
    private val isLoading = AtomicBoolean(false)

    // Streaming callback
    private var streamingCallback: ((String, Boolean) -> Unit)? = null

    /**
     * Initialize the bridge with application context.
     * Must be called from MainActivity before using other methods.
     */
    @JvmStatic
    fun initialize(context: Context) {
        appContext = context.applicationContext
        Log.i(TAG, "MediaPipe LLM Bridge initialized")
    }

    /**
     * Check if MediaPipe LLM is available on this device.
     * @return JSON with availability status and device info
     */
    @JvmStatic
    fun isAvailable(): String {
        return try {
            val json = JSONObject()
            json.put("available", true)
            json.put("initialized", llmInference != null)
            json.put("model_loaded", currentModelPath != null)
            json.put("current_model", currentModelPath ?: "")
            json.toString()
        } catch (e: Exception) {
            Log.e(TAG, "Error checking availability", e)
            """{"available":false,"error":"${e.message?.replace("\"", "\\\"")}"}"""
        }
    }

    /**
     * Load a model from the specified path.
     * @param modelPath Path to the .task model file (e.g., /data/local/tmp/llm/gemma-3-1b-it-int4.task)
     * @param maxTokens Maximum number of tokens to generate (default: 1024)
     * @param topK Top-K sampling parameter (default: 40)
     * @param temperature Sampling temperature (default: 0.8)
     * @param randomSeed Random seed for reproducibility (default: 0 = random)
     * @return JSON with success status or error
     */
    @JvmStatic
    fun loadModel(
        modelPath: String,
        maxTokens: Int = 1024,
        topK: Int = 40,
        temperature: Float = 0.8f,
        randomSeed: Int = 0
    ): String {
        val context = appContext
        if (context == null) {
            return """{"success":false,"error":"Bridge not initialized. Call initialize() first."}"""
        }

        if (isLoading.get()) {
            return """{"success":false,"error":"Model is already loading"}"""
        }

        return try {
            isLoading.set(true)
            Log.i(TAG, "Loading model from: $modelPath")

            // Check if model file exists
            val modelFile = File(modelPath)
            if (!modelFile.exists()) {
                isLoading.set(false)
                return """{"success":false,"error":"Model file not found: $modelPath"}"""
            }

            // Close existing model if any
            llmInference?.close()
            llmInference = null
            currentModelPath = null

            // Build options
            // Note: Temperature is not available in MediaPipe LLM Inference API 0.10.x
            // Only setModelPath, setMaxTokens, setMaxTopK, and setRandomSeed are supported
            val optionsBuilder = LlmInferenceOptions.builder()
                .setModelPath(modelPath)
                .setMaxTokens(maxTokens)
                .setMaxTopK(topK)

            if (randomSeed > 0) {
                optionsBuilder.setRandomSeed(randomSeed)
            }

            // Temperature parameter is accepted but not used in current API version
            @Suppress("UNUSED_VARIABLE")
            val unusedTemp = temperature

            val options = optionsBuilder.build()

            // Create LLM inference instance
            llmInference = LlmInference.createFromOptions(context, options)
            currentModelPath = modelPath

            isLoading.set(false)
            Log.i(TAG, "Model loaded successfully")

            val json = JSONObject()
            json.put("success", true)
            json.put("model_path", modelPath)
            json.toString()
        } catch (e: Exception) {
            isLoading.set(false)
            Log.e(TAG, "Error loading model", e)
            """{"success":false,"error":"${e.message?.replace("\"", "\\\"")}"}"""
        }
    }

    /**
     * Generate a response synchronously.
     * @param prompt The input prompt
     * @return JSON with generated text or error
     */
    @JvmStatic
    fun generateResponse(prompt: String): String {
        val inference = llmInference
        if (inference == null) {
            return """{"success":false,"error":"No model loaded. Call loadModel() first."}"""
        }

        return try {
            Log.d(TAG, "Generating response for prompt: ${prompt.take(100)}...")

            val response = inference.generateResponse(prompt)

            val json = JSONObject()
            json.put("success", true)
            json.put("response", response)
            json.put("prompt", prompt)
            json.toString()
        } catch (e: Exception) {
            Log.e(TAG, "Error generating response", e)
            """{"success":false,"error":"${e.message?.replace("\"", "\\\"")}"}"""
        }
    }

    /**
     * Generate a response asynchronously with streaming.
     * Results are sent via the streaming callback.
     * @param prompt The input prompt
     * @return JSON with status
     */
    @JvmStatic
    fun generateResponseAsync(prompt: String): String {
        val inference = llmInference
        if (inference == null) {
            return """{"success":false,"error":"No model loaded. Call loadModel() first."}"""
        }

        return try {
            Log.d(TAG, "Starting async generation for prompt: ${prompt.take(100)}...")

            inference.generateResponseAsync(prompt) { partialResult, done ->
                streamingCallback?.invoke(partialResult, done)
            }

            val json = JSONObject()
            json.put("success", true)
            json.put("status", "streaming")
            json.toString()
        } catch (e: Exception) {
            Log.e(TAG, "Error starting async generation", e)
            """{"success":false,"error":"${e.message?.replace("\"", "\\\"")}"}"""
        }
    }

    /**
     * Set the streaming callback for async generation.
     * @param callback Function that receives (partialResult: String, isDone: Boolean)
     */
    @JvmStatic
    fun setStreamingCallback(callback: (String, Boolean) -> Unit) {
        streamingCallback = callback
    }

    /**
     * Unload the current model and free resources.
     * @return JSON with status
     */
    @JvmStatic
    fun unloadModel(): String {
        return try {
            llmInference?.close()
            llmInference = null
            currentModelPath = null

            Log.i(TAG, "Model unloaded")
            """{"success":true}"""
        } catch (e: Exception) {
            Log.e(TAG, "Error unloading model", e)
            """{"success":false,"error":"${e.message?.replace("\"", "\\\"")}"}"""
        }
    }

    /**
     * Get the default model storage directory.
     * @return Path to the models directory
     */
    @JvmStatic
    fun getModelsDirectory(): String {
        val context = appContext ?: return "/data/local/tmp/llm"

        // Use app's files directory for model storage
        val modelsDir = File(context.filesDir, "models")
        if (!modelsDir.exists()) {
            modelsDir.mkdirs()
        }
        return modelsDir.absolutePath
    }

    /**
     * List available models in the models directory.
     * @return JSON array of model files
     */
    @JvmStatic
    fun listModels(): String {
        return try {
            val modelsDir = File(getModelsDirectory())
            val models = modelsDir.listFiles { file ->
                file.isFile && (file.name.endsWith(".task") || file.name.endsWith(".bin"))
            } ?: emptyArray()

            val json = JSONObject()
            json.put("success", true)
            json.put("models_dir", modelsDir.absolutePath)

            val modelsList = models.map { file ->
                JSONObject().apply {
                    put("name", file.name)
                    put("path", file.absolutePath)
                    put("size_mb", file.length() / (1024 * 1024))
                }
            }
            json.put("models", modelsList)
            json.toString()
        } catch (e: Exception) {
            Log.e(TAG, "Error listing models", e)
            """{"success":false,"error":"${e.message?.replace("\"", "\\\"")}"}"""
        }
    }

    /**
     * Get recommended models for download.
     * @return JSON with model recommendations and download URLs
     */
    @JvmStatic
    fun getRecommendedModels(): String {
        val json = JSONObject()
        json.put("success", true)
        json.put("models", listOf(
            JSONObject().apply {
                put("name", "Gemma 3 1B (4-bit)")
                put("id", "gemma-3-1b-it-int4")
                put("size_mb", 550)
                put("description", "Compact, fast model suitable for most devices")
                put("url", "https://huggingface.co/litert-community/Gemma3-1B-IT/resolve/main/gemma3-1b-it-int4.task")
            },
            JSONObject().apply {
                put("name", "Gemma 3n E2B (4-bit)")
                put("id", "gemma-3n-e2b-it-int4")
                put("size_mb", 1400)
                put("description", "Effective 2B model with multimodal support")
                put("url", "https://huggingface.co/litert-community/Gemma3n-E2B-IT/resolve/main/gemma3n-e2b-it-int4.task")
            },
            JSONObject().apply {
                put("name", "Gemma 3n E4B (4-bit)")
                put("id", "gemma-3n-e4b-it-int4")
                put("size_mb", 2800)
                put("description", "Effective 4B model, best quality, requires high-end device")
                put("url", "https://huggingface.co/litert-community/Gemma3n-E4B-IT/resolve/main/gemma3n-e4b-it-int4.task")
            }
        ))
        return json.toString()
    }
}
