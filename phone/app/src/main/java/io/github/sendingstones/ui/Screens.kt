package io.github.sendingstones.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import io.github.sendingstones.net.Board
import io.github.sendingstones.net.BoardStatus
import io.github.sendingstones.net.discoverBoards
import io.github.sendingstones.net.queryStatus
import io.github.sendingstones.proto.DEFAULT_PORT
import java.net.InetSocketAddress
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.withContext

/** Screen 1: boards found over mDNS, plus a typed address as fallback. */
@Composable
fun FindBoardScreen(settings: Settings, onPick: (Board) -> Unit, onSettings: () -> Unit) {
    val context = LocalContext.current
    val boards by remember {
        discoverBoards(context).catch { emit(emptyList()) }.flowOn(Dispatchers.Main)
    }.collectAsState(initial = emptyList())
    var typed by remember { mutableStateOf(settings.boardAddress) }
    var error by remember { mutableStateOf<String?>(null) }

    Page("Sending Stones", action = "Settings", onAction = onSettings) {
        Text("Boards on this Wi-Fi", style = MaterialTheme.typography.titleMedium)
        if (boards.isEmpty()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(12.dp))
                Text("Searching…")
            }
        }
        for (board in boards) {
            Card(Modifier.fillMaxWidth().clickable { onPick(board) }) {
                Column(Modifier.padding(16.dp)) {
                    Text(board.name, style = MaterialTheme.typography.titleSmall)
                    Text(board.label, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        Spacer(Modifier.size(8.dp))
        Text("Or type its address", style = MaterialTheme.typography.titleMedium)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(
                value = typed,
                onValueChange = { typed = it.trim(); error = null },
                label = { Text("Board IP or host:port") },
                singleLine = true,
                isError = error != null,
                supportingText = error?.let { { Text(it) } },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                modifier = Modifier.weight(1f),
            )
            var resolving by remember { mutableStateOf(false) }
            Button(enabled = typed.isNotEmpty() && !resolving, onClick = { resolving = true }) { Text("Connect") }
            if (resolving) {
                LaunchedEffect(typed) {
                    val address = withContext(Dispatchers.IO) { parseAddress(typed) }
                    resolving = false
                    if (address == null) error = "Not an address" else onPick(Board(typed, address))
                }
            }
        }
    }
}

/** Accepts `host` or `host:port`; resolves on the caller's (IO) thread. */
fun parseAddress(text: String): InetSocketAddress? {
    val host = text.substringBeforeLast(':', text)
    val port = text.substringAfterLast(':', "").toIntOrNull() ?: DEFAULT_PORT
    if (host.isBlank() || port !in 1..65535) return null
    return runCatching { InetSocketAddress(java.net.InetAddress.getByName(host), port) }.getOrNull()
}

/** Screen 2: one button per player, two per row, free or taken; PIN when the board asks for one. */
@Composable
fun SlotPickerScreen(board: Board, holds: (slot: Int) -> Boolean, onPick: (slot: Int, pin: Int?) -> Unit, onBack: () -> Unit) {
    var status by remember { mutableStateOf<BoardStatus?>(null) }
    var noReply by remember { mutableStateOf(false) }
    var pin by remember { mutableStateOf("") }

    LaunchedEffect(board) {
        while (true) {
            val s = queryStatus(board.address)
            noReply = s == null
            if (s != null) status = s
            delay(1500)
        }
    }

    Page(board.name, action = "Back", onAction = onBack) {
        val s = status
        when {
            s == null && noReply -> Text("The board does not answer at ${board.label}. Same Wi-Fi? Is stonesd running?")
            s == null -> Text("Asking the board…")
            else -> {
                if (s.pinRequired) {
                    OutlinedTextField(
                        value = pin,
                        onValueChange = { v -> pin = v.filter(Char::isDigit).take(4) },
                        label = { Text("PIN") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
                    )
                }
                for (row in s.slots.chunked(2)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        for (slot in row) {
                            val color = PlayerColors[(slot.slot - 1).mod(PlayerColors.size)]
                            // This phone's own slot from an earlier run: resuming needs no PIN.
                            val mine = !slot.free && holds(slot.slot)
                            Button(
                                onClick = { onPick(slot.slot, pin.toIntOrNull()) },
                                enabled = mine || (slot.free && (!s.pinRequired || pin.length == 4)),
                                colors = ButtonDefaults.buttonColors(containerColor = color),
                                modifier = Modifier.size(width = 160.dp, height = 88.dp),
                            ) {
                                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                                    Text("Player ${slot.slot}", style = MaterialTheme.typography.titleLarge)
                                    Text(if (slot.free) "free" else if (mine) "yours" else slot.state)
                                }
                            }
                        }
                    }
                }
                if (s.slots.none { it.free }) {
                    Text("Every slot is held. A dropped phone frees its slot after 2 minutes.")
                }
            }
        }
    }
}

@Composable
fun SettingsScreen(settings: Settings, onChange: (Settings) -> Unit, onBack: () -> Unit) {
    Page("Settings", action = "Done", onAction = onBack) {
        LabeledSlider("Stick sensitivity", settings.sensitivity, 0.5f..2f, "%.2f×") { onChange(settings.copy(sensitivity = it)) }
        LabeledSlider("Dead zone", settings.deadZone, 0f..0.3f, null) { onChange(settings.copy(deadZone = it)) }
        LabeledSwitch("Swap sticks (left-handed)", settings.swapSticks) { onChange(settings.copy(swapSticks = it)) }
        LabeledSwitch("Vibrate on press", settings.vibration) { onChange(settings.copy(vibration = it)) }
        LabeledSwitch("Show latency", settings.showLatency) { onChange(settings.copy(showLatency = it)) }
        OutlinedTextField(
            value = settings.boardAddress,
            onValueChange = { onChange(settings.copy(boardAddress = it.trim())) },
            label = { Text("Board address (fallback)") },
            singleLine = true,
        )
    }
}

@Composable
private fun LabeledSlider(label: String, value: Float, range: ClosedFloatingPointRange<Float>, format: String?, onChange: (Float) -> Unit) {
    val shown = format?.format(value) ?: "${(value * 100).toInt()} %"
    Text("$label: $shown")
    Slider(value = value, onValueChange = onChange, valueRange = range)
}

@Composable
private fun LabeledSwitch(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onChange)
    }
}

@Composable
private fun Page(title: String, action: String, onAction: () -> Unit, content: @Composable () -> Unit) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(title, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
            TextButton(onClick = onAction) { Text(action) }
        }
        content()
    }
}
