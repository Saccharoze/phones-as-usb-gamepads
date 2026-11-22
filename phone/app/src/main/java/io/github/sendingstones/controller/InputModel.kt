package io.github.sendingstones.controller

import io.github.sendingstones.proto.AXIS_MAX
import io.github.sendingstones.proto.Button
import io.github.sendingstones.proto.Hat
import io.github.sendingstones.proto.PadState
import kotlin.math.atan2
import kotlin.math.hypot
import kotlin.math.pow
import kotlin.math.roundToInt

data class Vec(val x: Float, val y: Float) {
    operator fun minus(o: Vec) = Vec(x - o.x, y - o.y)
    val length get() = hypot(x, y)
}

data class StickSettings(
    val deadZone: Float = 0.10f,
    val sensitivity: Float = 1.0f,
    /** 1 = linear; above 1 gives finer control near the centre. */
    val curve: Float = 1.0f,
    /** Left-handed: the left half drives the right stick and vice versa. */
    val swapSticks: Boolean = false,
)

object StickMath {
    /** Thumb offset -> axis pair in -32767..32767, Y down-positive like HID. */
    fun axes(offset: Vec, travel: Float, s: StickSettings): Pair<Int, Int> {
        val magnitude = offset.length / travel
        if (magnitude <= s.deadZone || travel <= 0f) return 0 to 0
        val beyondDead = ((magnitude - s.deadZone) / (1f - s.deadZone)).coerceIn(0f, 1f)
        val scaled = (beyondDead.pow(s.curve) * s.sensitivity).coerceIn(0f, 1f)
        val unit = scaled / offset.length
        return axis(offset.x * unit) to axis(offset.y * unit)
    }

    private fun axis(v: Float) = (v * AXIS_MAX).roundToInt().coerceIn(-AXIS_MAX, AXIS_MAX)

    /** Screen-space offset from the D-pad centre -> hat value (8 = none). */
    fun hat(offset: Vec, radius: Float): Int {
        if (offset.length < radius * Layout.DPAD_DEAD) return Hat.NEUTRAL
        // atan2(x, -y): 0° points up, clockwise positive, as the hat expects.
        val degrees = (Math.toDegrees(atan2(offset.x, -offset.y).toDouble()) + 360.0) % 360.0
        return ((degrees + 22.5) / 45.0).toInt() % 8
    }
}

/** What one finger is holding, from touch-down until it lifts. */
sealed interface Binding {
    data class Press(val button: Button) : Binding
    data class Trigger(val left: Boolean) : Binding
    data class Dpad(val hat: Int) : Binding
    data class Stick(val left: Boolean, val origin: Vec, val current: Vec) : Binding
}

/**
 * Multi-touch to [PadState]. Each pointer binds to the control it went down
 * on and keeps it until it lifts; only the D-pad lets a finger slide between
 * directions. Pure: positions in pixels in, state out.
 */
class InputModel(private var width: Float, private var height: Float, var settings: StickSettings = StickSettings()) {
    private val bindings = LinkedHashMap<Long, Binding>()

    val active: Map<Long, Binding> get() = bindings

    fun resize(w: Float, h: Float) {
        width = w
        height = h
    }

    /** Returns true when the touch pressed something worth a haptic tick. */
    fun down(pointer: Long, at: Vec): Boolean {
        val binding = bind(at) ?: return false
        bindings[pointer] = binding
        return binding !is Binding.Stick && !(binding is Binding.Dpad && binding.hat == Hat.NEUTRAL)
    }

    /** Returns true when a D-pad slide changed direction (another haptic tick). */
    fun move(pointer: Long, at: Vec): Boolean {
        when (val b = bindings[pointer]) {
            is Binding.Stick -> bindings[pointer] = b.copy(current = at)
            is Binding.Dpad -> {
                val hat = StickMath.hat(at - dpadCenter(), dpadRadius())
                bindings[pointer] = Binding.Dpad(hat)
                return hat != b.hat && hat != Hat.NEUTRAL
            }
            else -> {}
        }
        return false
    }

    fun up(pointer: Long) {
        bindings.remove(pointer)
    }

    fun releaseAll() = bindings.clear()

    fun state(): PadState {
        var s = PadState.NEUTRAL
        for (b in bindings.values) {
            s = when (b) {
                is Binding.Press -> s.with(b.button, true)
                is Binding.Trigger -> if (b.left) s.copy(leftTrigger = 255) else s.copy(rightTrigger = 255)
                is Binding.Dpad -> if (b.hat != Hat.NEUTRAL) s.copy(hat = b.hat) else s
                is Binding.Stick -> withStick(s, b)
            }
        }
        return s
    }

    private fun withStick(s: PadState, b: Binding.Stick): PadState {
        val (x, y) = StickMath.axes(b.current - b.origin, Layout.STICK_TRAVEL * height, settings)
        val drivesLeft = b.left != settings.swapSticks
        return if (drivesLeft) s.copy(leftX = x, leftY = y) else s.copy(rightX = x, rightY = y)
    }

    private fun bind(at: Vec): Binding? {
        hit(at)?.let { control ->
            return when (control) {
                is Control.Press -> Binding.Press(control.button)
                is Control.Trigger -> Binding.Trigger(control.left)
                is Control.Dpad -> Binding.Dpad(StickMath.hat(at - dpadCenter(), dpadRadius()))
            }
        }
        if (at.y < Layout.STICK_ZONE_TOP * height) return null
        val left = at.x < width / 2
        // One finger per stick: a second thumb on the same half is ignored.
        if (bindings.values.any { it is Binding.Stick && it.left == left }) return null
        return Binding.Stick(left, origin = at, current = at)
    }

    /** Nearest control whose enlarged circle contains the touch. */
    private fun hit(at: Vec): Control? = Layout.controls
        .map { it to (at - it.spot.center(width, height)).length / it.spot.radius(height) }
        .filter { (_, d) -> d <= Layout.HIT_SLOP }
        .minByOrNull { (_, d) -> d }
        ?.first

    private val dpad get() = Layout.controls.filterIsInstance<Control.Dpad>().first()
    private fun dpadCenter() = dpad.spot.center(width, height)
    private fun dpadRadius() = dpad.spot.radius(height)
}
