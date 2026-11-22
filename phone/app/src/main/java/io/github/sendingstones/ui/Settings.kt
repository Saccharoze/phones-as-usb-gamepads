package io.github.sendingstones.ui

import android.content.Context
import androidx.core.content.edit
import io.github.sendingstones.controller.StickSettings

data class Settings(
    val sensitivity: Float = 1.0f,
    val deadZone: Float = 0.10f,
    val swapSticks: Boolean = false,
    val vibration: Boolean = true,
    val showLatency: Boolean = true,
    /** host:port typed by hand or the last board used; empty when none. */
    val boardAddress: String = "",
) {
    val stick get() = StickSettings(deadZone = deadZone, sensitivity = sensitivity, swapSticks = swapSticks)
}

class SettingsStore(context: Context) {
    private val prefs = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    fun load() = Settings(
        sensitivity = prefs.getFloat("sensitivity", 1.0f),
        deadZone = prefs.getFloat("dead_zone", 0.10f),
        swapSticks = prefs.getBoolean("swap_sticks", false),
        vibration = prefs.getBoolean("vibration", true),
        showLatency = prefs.getBoolean("show_latency", true),
        boardAddress = prefs.getString("board_address", "") ?: "",
    )

    fun save(s: Settings) {
        prefs.edit {
            putFloat("sensitivity", s.sensitivity)
            putFloat("dead_zone", s.deadZone)
            putBoolean("swap_sticks", s.swapSticks)
            putBoolean("vibration", s.vibration)
            putBoolean("show_latency", s.showLatency)
            putString("board_address", s.boardAddress)
        }
    }
}
