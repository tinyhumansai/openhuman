package com.alphahuman.app

import android.util.Log

/**
 * TDLib Bridge Stub for Android
 *
 * TDLib native library is not available on Android through Maven Central.
 * Telegram integration on mobile uses MTProto via the frontend JavaScript.
 * This stub ensures the build compiles while TDLib features return errors.
 */
object TdLibBridge {
    private const val TAG = "TdLibBridge"

    /**
     * Stub - TDLib is not available on Android.
     */
    @JvmStatic
    fun createClient(): Int {
        Log.w(TAG, "TDLib is not available on Android")
        return -1
    }

    /**
     * Stub - TDLib is not available on Android.
     */
    @JvmStatic
    fun send(clientId: Int, requestJson: String): String {
        Log.w(TAG, "TDLib is not available on Android")
        return """{"@type":"error","code":501,"message":"TDLib is not available on Android"}"""
    }

    /**
     * Stub - TDLib is not available on Android.
     */
    @JvmStatic
    fun receive(timeout: Double): String? {
        return null
    }

    /**
     * Stub - TDLib is not available on Android.
     */
    @JvmStatic
    fun destroyClient(clientId: Int) {
        Log.w(TAG, "TDLib is not available on Android")
    }

    /**
     * TDLib is not available on Android via Maven.
     */
    @JvmStatic
    fun isAvailable(): Boolean {
        return false
    }
}
