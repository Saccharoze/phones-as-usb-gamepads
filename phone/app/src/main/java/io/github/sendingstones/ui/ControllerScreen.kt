package io.github.sendingstones.ui

import android.view.HapticFeedbackConstants
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.sp
import io.github.sendingstones.controller.Binding
import io.github.sendingstones.controller.Control
import io.github.sendingstones.controller.InputModel
import io.github.sendingstones.controller.Layout
import io.github.sendingstones.controller.Vec
import io.github.sendingstones.net.BoardClient
import io.github.sendingstones.net.Link
import io.github.sendingstones.proto.Hat

private val Background = Color(0xFF121214)
private val Idle = Color(0xFF3A3A40)
private val LabelColor = Color(0xFFE8E8EC)
/** Player 1 to 4: blue, orange, green, purple. A board with more pads reuses them in turn. */
val PlayerColors = listOf(Color(0xFF4F8EF7), Color(0xFFF2784B), Color(0xFF3DBE7A), Color(0xFFB57CF2))

/**
 * One Canvas with raw multi-pointer handling: every pointer is bound to the
 * control it went down on until it lifts (see [InputModel]).
 */
@Composable
fun ControllerScreen(client: BoardClient, settings: Settings) {
    val model = remember { InputModel(1f, 1f, settings.stick) }
    // Bumped on every input event so the Canvas redraws pressed states.
    var frame by remember { mutableIntStateOf(0) }
    val link by client.link.collectAsState()
    val view = LocalView.current
    val text = rememberTextMeasurer()
    val accent = PlayerColors[(client.slot - 1).mod(PlayerColors.size)]

    LaunchedEffect(settings) { model.settings = settings.stick }

    Box(Modifier.fillMaxSize().background(Background)) {
        Canvas(
            Modifier.fillMaxSize().pointerInput(Unit) {
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent()
                        model.resize(size.width.toFloat(), size.height.toFloat())
                        var tick = false
                        for (change in event.changes) {
                            val id = change.id.value
                            val at = Vec(change.position.x, change.position.y)
                            when {
                                change.pressed && !change.previousPressed -> tick = model.down(id, at) || tick
                                !change.pressed && change.previousPressed -> model.up(id)
                                change.pressed -> tick = model.move(id, at) || tick
                            }
                            change.consume()
                        }
                        if (tick && settings.vibration) view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
                        client.update(model.state())
                        frame++
                    }
                }
            },
        ) {
            @Suppress("UNUSED_EXPRESSION") frame // read so the Canvas redraws on input
            model.resize(size.width, size.height)
            drawControls(model, accent, text)
            drawSticks(model, accent)
            drawTopBar(client.slot, link, settings.showLatency, accent, text)
        }
    }
}

private fun DrawScope.drawControls(model: InputModel, accent: Color, text: TextMeasurer) {
    val (w, h) = size.width to size.height
    val held = model.active.values
    for (control in Layout.controls) {
        val c = control.spot.center(w, h).offset()
        val r = control.spot.radius(h)
        when (control) {
            is Control.Press -> {
                val on = held.any { it is Binding.Press && it.button == control.button }
                drawCircle(if (on) accent else Idle, r, c)
                label(text, control.button.label, c, r)
            }
            is Control.Trigger -> {
                val on = held.any { it is Binding.Trigger && it.left == control.left }
                drawCircle(if (on) accent else Idle, r, c)
                label(text, if (control.left) "LT" else "RT", c, r)
            }
            is Control.Dpad -> drawDpad(c, r, held.filterIsInstance<Binding.Dpad>().firstOrNull()?.hat, accent)
        }
    }
}

private fun DrawScope.drawDpad(c: Offset, r: Float, hat: Int?, accent: Color) {
    drawCircle(Idle, r, c, style = Stroke(width = r * 0.08f))
    // Up, right, down, left pads; diagonals light the two neighbours.
    val pads = listOf(Hat.UP to Offset(0f, -1f), Hat.RIGHT to Offset(1f, 0f), Hat.DOWN to Offset(0f, 1f), Hat.LEFT to Offset(-1f, 0f))
    for ((dir, unit) in pads) {
        val on = hat != null && hat != Hat.NEUTRAL && ((hat - dir + 8) % 8).let { it == 0 || it == 1 || it == 7 }
        drawCircle(if (on) accent else Idle, r * 0.28f, c + unit * (r * 0.6f))
    }
}

private fun DrawScope.drawSticks(model: InputModel, accent: Color) {
    val travel = Layout.STICK_TRAVEL * size.height
    for (b in model.active.values.filterIsInstance<Binding.Stick>()) {
        val origin = b.origin.offset()
        drawCircle(Idle, travel, origin, style = Stroke(width = travel * 0.06f))
        val delta = b.current - b.origin
        val k = if (delta.length > travel) travel / delta.length else 1f
        drawCircle(accent, travel * 0.45f, origin + Offset(delta.x * k, delta.y * k))
    }
}

private fun DrawScope.drawTopBar(slot: Int, link: Link, showLatency: Boolean, accent: Color, text: TextMeasurer) {
    val h = size.height
    val c = Offset(size.width / 2, h * 0.05f)
    drawCircle(accent, h * 0.035f, c)
    label(text, "P$slot", c, h * 0.035f)
    val (dot, status) = when (link) {
        is Link.Connected -> Color(0xFF3CCB7F) to if (showLatency) link.rttP50Ms?.let { "$it ms" } ?: "…" else ""
        Link.Connecting -> Color(0xFFE5B83B) to "connecting"
        Link.Paused -> Color(0xFFE5B83B) to "paused"
        Link.NoReply -> Color(0xFFE5484D) to "no reply"
        is Link.Refused -> Color(0xFFE5484D) to link.code.name
    }
    drawCircle(dot, h * 0.012f, c + Offset(h * 0.08f, 0f))
    if (status.isNotEmpty()) {
        val layout = text.measure(status, TextStyle(color = LabelColor, fontSize = 12.sp))
        drawText(layout, topLeft = c + Offset(h * 0.11f, -layout.size.height / 2f))
    }
}

private fun DrawScope.label(text: TextMeasurer, s: String, c: Offset, r: Float) {
    val layout = text.measure(s, TextStyle(color = LabelColor, fontSize = (r / density * 0.55f).coerceIn(9f, 22f).sp))
    drawText(layout, topLeft = c - Offset(layout.size.width / 2f, layout.size.height / 2f))
}

private fun Vec.offset() = Offset(x, y)
