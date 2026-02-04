package com.alphahuman.app

import android.util.Log
import org.drinkless.tdlib.Client
import org.drinkless.tdlib.TdApi
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/**
 * TDLib Bridge for Android
 *
 * Provides a JNI-accessible interface to TDLib for the Rust backend.
 * Manages TDLib client lifecycle and provides JSON-based request/response interface.
 */
object TdLibBridge {
    private const val TAG = "TdLibBridge"

    // Client instance (singleton)
    private var client: Client? = null

    // Request ID counter for correlation
    private val requestIdCounter = AtomicLong(1)

    // Pending requests waiting for responses
    private val pendingRequests = ConcurrentHashMap<Long, (TdApi.Object) -> Unit>()

    // Update handler callback
    private var updateHandler: ((String) -> Unit)? = null

    // Client ID (always 1 for singleton)
    private const val CLIENT_ID = 1

    /**
     * Create a TDLib client.
     * @return Client ID (always 1)
     */
    @JvmStatic
    fun createClient(): Int {
        Log.i(TAG, "Creating TDLib client")

        if (client != null) {
            Log.w(TAG, "Client already exists, returning existing client ID")
            return CLIENT_ID
        }

        // Create result handler that processes responses and updates
        val resultHandler = Client.ResultHandler { result ->
            handleResult(result)
        }

        // Create exception handler
        val exceptionHandler = Client.ExceptionHandler { e ->
            Log.e(TAG, "TDLib exception", e)
        }

        // Create the client
        client = Client.create(resultHandler, exceptionHandler, exceptionHandler)

        Log.i(TAG, "TDLib client created successfully")
        return CLIENT_ID
    }

    /**
     * Handle a TDLib result (response or update).
     */
    private fun handleResult(result: TdApi.Object) {
        // Convert to JSON for the Rust side
        val json = tdObjectToJson(result)

        // Check if this is a response to a pending request (has @extra)
        // Note: TDLib Java API doesn't expose @extra directly, so we handle responses
        // through the synchronous send pattern instead

        // For updates, call the update handler
        if (result is TdApi.Update) {
            updateHandler?.invoke(json)
        }
    }

    /**
     * Send a synchronous request to TDLib.
     * @param requestJson JSON string of the TDLib API request
     * @return JSON string of the response
     */
    @JvmStatic
    fun send(clientId: Int, requestJson: String): String {
        val currentClient = client
        if (currentClient == null) {
            Log.e(TAG, "Client not initialized")
            return """{"@type":"error","code":400,"message":"Client not initialized"}"""
        }

        try {
            Log.d(TAG, "Sending request: $requestJson")

            // Parse the JSON request
            val function = jsonToTdFunction(requestJson)
            if (function == null) {
                return """{"@type":"error","code":400,"message":"Invalid request format"}"""
            }

            // Execute synchronously
            val result = currentClient.send(function)

            // Convert result to JSON
            val responseJson = tdObjectToJson(result)
            Log.d(TAG, "Received response: $responseJson")

            return responseJson
        } catch (e: Exception) {
            Log.e(TAG, "Error sending request", e)
            return """{"@type":"error","code":500,"message":"${e.message?.replace("\"", "\\\"")}"}"""
        }
    }

    /**
     * Receive updates from TDLib (with timeout).
     * @param timeout Timeout in seconds
     * @return JSON string of the update, or null if timeout
     */
    @JvmStatic
    fun receive(timeout: Double): String? {
        val currentClient = client ?: return null

        try {
            val result = Client.execute(TdApi.GetOption("version"))
            // The actual receiving is done via the result handler callback
            // This method is mainly for polling pattern support
            return null
        } catch (e: Exception) {
            Log.e(TAG, "Error receiving", e)
            return null
        }
    }

    /**
     * Set the update handler callback.
     * @param handler Function that receives update JSON strings
     */
    @JvmStatic
    fun setUpdateHandler(handler: (String) -> Unit) {
        updateHandler = handler
    }

    /**
     * Destroy the TDLib client.
     */
    @JvmStatic
    fun destroyClient(clientId: Int) {
        Log.i(TAG, "Destroying TDLib client")

        client?.close()
        client = null
        pendingRequests.clear()
        updateHandler = null

        Log.i(TAG, "TDLib client destroyed")
    }

    /**
     * Check if TDLib is available.
     */
    @JvmStatic
    fun isAvailable(): Boolean {
        return try {
            // Try to load the TDLib native library
            System.loadLibrary("tdjni")
            true
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "TDLib native library not found", e)
            false
        }
    }

    /**
     * Convert a TDLib object to JSON string.
     * Note: This is a simplified implementation. TDLib Java API provides toString()
     * which returns a debug representation, not proper JSON.
     */
    private fun tdObjectToJson(obj: TdApi.Object): String {
        // Use TDLib's built-in serialization
        // The toString() method provides a debug format, we need proper JSON
        return try {
            // For now, return a simple JSON representation
            // In production, use TDLib's JSON serialization or implement proper conversion
            val json = JSONObject()
            json.put("@type", obj.javaClass.simpleName.replaceFirstChar { it.lowercase() })

            // Handle common types
            when (obj) {
                is TdApi.Error -> {
                    json.put("code", obj.code)
                    json.put("message", obj.message)
                }
                is TdApi.Ok -> {
                    // Empty ok response
                }
                is TdApi.User -> {
                    json.put("id", obj.id)
                    json.put("first_name", obj.firstName)
                    json.put("last_name", obj.lastName)
                    json.put("username", obj.usernames?.activeUsernames?.firstOrNull() ?: "")
                }
                is TdApi.AuthorizationStateWaitTdlibParameters -> {
                    json.put("@type", "authorizationStateWaitTdlibParameters")
                }
                is TdApi.AuthorizationStateWaitPhoneNumber -> {
                    json.put("@type", "authorizationStateWaitPhoneNumber")
                }
                is TdApi.AuthorizationStateWaitCode -> {
                    json.put("@type", "authorizationStateWaitCode")
                }
                is TdApi.AuthorizationStateReady -> {
                    json.put("@type", "authorizationStateReady")
                }
                is TdApi.UpdateAuthorizationState -> {
                    json.put("@type", "updateAuthorizationState")
                    json.put("authorization_state", tdObjectToJson(obj.authorizationState))
                }
                else -> {
                    // Generic handling - just use toString for now
                    json.put("raw", obj.toString())
                }
            }

            json.toString()
        } catch (e: Exception) {
            Log.e(TAG, "Error converting TdObject to JSON", e)
            """{"@type":"error","code":500,"message":"JSON conversion failed"}"""
        }
    }

    /**
     * Convert a JSON string to a TDLib function.
     * Note: This is a simplified implementation. Full implementation would parse
     * all TDLib API types.
     */
    private fun jsonToTdFunction(json: String): TdApi.Function<*>? {
        return try {
            val obj = JSONObject(json)
            val type = obj.optString("@type", "")

            when (type) {
                "setTdlibParameters" -> TdApi.SetTdlibParameters().apply {
                    databaseDirectory = obj.optString("database_directory", "")
                    useMessageDatabase = obj.optBoolean("use_message_database", true)
                    useSecretChats = obj.optBoolean("use_secret_chats", false)
                    apiId = obj.optInt("api_id", 0)
                    apiHash = obj.optString("api_hash", "")
                    systemLanguageCode = obj.optString("system_language_code", "en")
                    deviceModel = obj.optString("device_model", "Android")
                    applicationVersion = obj.optString("application_version", "1.0")
                }
                "setAuthenticationPhoneNumber" -> TdApi.SetAuthenticationPhoneNumber(
                    obj.optString("phone_number", ""),
                    null
                )
                "checkAuthenticationCode" -> TdApi.CheckAuthenticationCode(
                    obj.optString("code", "")
                )
                "getMe" -> TdApi.GetMe()
                "getChats" -> TdApi.GetChats(
                    null,
                    obj.optInt("limit", 100)
                )
                "getChat" -> TdApi.GetChat(
                    obj.optLong("chat_id", 0)
                )
                "sendMessage" -> {
                    val chatId = obj.optLong("chat_id", 0)
                    val text = obj.optString("text", "")
                    val inputContent = TdApi.InputMessageText(
                        TdApi.FormattedText(text, emptyArray()),
                        null,
                        false
                    )
                    TdApi.SendMessage(chatId, 0, null, null, null, inputContent)
                }
                "close" -> TdApi.Close()
                "logOut" -> TdApi.LogOut()
                "getOption" -> TdApi.GetOption(obj.optString("name", ""))
                else -> {
                    Log.w(TAG, "Unknown function type: $type")
                    null
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error parsing JSON to TdFunction", e)
            null
        }
    }
}
