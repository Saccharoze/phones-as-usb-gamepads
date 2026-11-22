package io.github.sendingstones.ui

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.lifecycleScope
import io.github.sendingstones.net.Board
import io.github.sendingstones.net.BoardClient
import io.github.sendingstones.net.TokenStore

private sealed interface Screen {
    data object Find : Screen
    data class Slots(val board: Board) : Screen
    data class Controller(val board: Board, val client: BoardClient) : Screen
    data class SettingsPage(val back: Screen) : Screen
}

class MainActivity : ComponentActivity() {
    private var screen by mutableStateOf<Screen>(Screen.Find)
    private lateinit var store: SettingsStore
    private lateinit var tokens: TokenStore
    private var settings by mutableStateOf(Settings())
    private var wifiLock: WifiManager.WifiLock? = null

    private val client: BoardClient? get() = (screen as? Screen.Controller)?.client

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        store = SettingsStore(this)
        tokens = TokenStore(this)
        settings = store.load()
        setContent {
            MaterialTheme(colorScheme = darkColorScheme()) {
                Surface { Content() }
            }
        }
    }

    @androidx.compose.runtime.Composable
    private fun Content() {
        when (val s = screen) {
            Screen.Find -> FindBoardScreen(
                settings,
                onPick = { board ->
                    updateSettings(settings.copy(boardAddress = board.label))
                    screen = Screen.Slots(board)
                },
                onSettings = { screen = Screen.SettingsPage(Screen.Find) },
            )
            is Screen.Slots -> {
                BackHandler { screen = Screen.Find }
                SlotPickerScreen(
                    s.board,
                    holds = { slot -> tokens.holds(s.board, slot) },
                    onPick = { slot, pin -> startController(s.board, slot, pin) },
                    onBack = { screen = Screen.Find },
                )
            }
            is Screen.Controller -> {
                BackHandler { leaveController(s) }
                DisposableEffect(s) {
                    enterFullScreen(true)
                    onDispose { enterFullScreen(false) }
                }
                ControllerScreen(s.client, settings)
            }
            is Screen.SettingsPage -> {
                BackHandler { screen = s.back }
                SettingsScreen(settings, onChange = ::updateSettings, onBack = { screen = s.back })
            }
        }
    }

    private fun startController(board: Board, slot: Int, pin: Int?) {
        val client = BoardClient(board.address, slot, pin, frameMs(), lifecycleScope, tokens.forSlot(board, slot))
        screen = Screen.Controller(board, client)
        client.start()
    }

    private fun leaveController(s: Screen.Controller) {
        s.client.close()
        screen = Screen.Slots(s.board)
    }

    private fun updateSettings(new: Settings) {
        settings = new
        store.save(new)
    }

    /** Immersive full screen, screen kept on, low-latency Wi-Fi while playing. */
    private fun enterFullScreen(on: Boolean) {
        val insets = WindowCompat.getInsetsController(window, window.decorView)
        if (on) {
            insets.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            insets.hide(WindowInsetsCompat.Type.systemBars())
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            holdWifiLock(true)
        } else {
            insets.show(WindowInsetsCompat.Type.systemBars())
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            holdWifiLock(false)
        }
    }

    private fun holdWifiLock(on: Boolean) {
        if (!on) {
            wifiLock?.takeIf { it.isHeld }?.release()
            return
        }
        val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        @Suppress("DEPRECATION") // HIGH_PERF is the best available before Android 10
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            WifiManager.WIFI_MODE_FULL_LOW_LATENCY
        } else {
            WifiManager.WIFI_MODE_FULL_HIGH_PERF
        }
        wifiLock = (wifiLock ?: wifi.createWifiLock(mode, "sending-stones").apply { setReferenceCounted(false) })
            .also { it.acquire() }
    }

    @Suppress("DEPRECATION") // Display.getRefreshRate via the window's display, valid since API 1
    private fun frameMs(): Long {
        val hz = windowManager.defaultDisplay.refreshRate.takeIf { it > 1f } ?: 60f
        return (1000f / hz).toLong().coerceAtLeast(1)
    }

    override fun onPause() {
        super.onPause()
        client?.pause()
        if (screen is Screen.Controller) holdWifiLock(false)
    }

    override fun onResume() {
        super.onResume()
        client?.start()
        if (screen is Screen.Controller) holdWifiLock(true)
    }

    override fun onDestroy() {
        if (isFinishing) client?.close()
        super.onDestroy()
    }
}
