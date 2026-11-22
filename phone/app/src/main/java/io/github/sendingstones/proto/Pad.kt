package io.github.sendingstones.proto

/** Kotlin port of the Rust `proto` crate; both are checked against vectors.json. */

/**
 * Buttons and their report bit. Bit n is HID button n + 1, which a Linux PC reports as key code
 * 0x130 + n, so each button sits on the bit whose code carries its name (BTN_A, BTN_X, BTN_TL,
 * BTN_SELECT...). Bits 2, 5, 8, 9 and 15 are reserved.
 */
enum class Button(val bit: Int, val label: String) {
    A(0, "A"), B(1, "B"), X(3, "X"), Y(4, "Y"),
    LB(6, "LB"), RB(7, "RB"), View(10, "View"), Menu(11, "Menu"),
    Guide(12, "Guide"), L3(13, "L3"), R3(14, "R3");

    val mask: Int get() = 1 shl bit
}

/** D-pad as a HID hat: 0 = up, clockwise in 45° steps, 8 = released. */
object Hat {
    const val UP = 0
    const val UP_RIGHT = 1
    const val RIGHT = 2
    const val DOWN_RIGHT = 3
    const val DOWN = 4
    const val DOWN_LEFT = 5
    const val LEFT = 6
    const val UP_LEFT = 7
    const val NEUTRAL = 8
}

const val AXIS_MAX = 32767
const val AXIS_MIN = -32767

/** One controller's full state. Stick Y is down-positive (HID convention). */
data class PadState(
    val buttons: Int = 0,
    val hat: Int = Hat.NEUTRAL,
    val leftX: Int = 0,
    val leftY: Int = 0,
    val rightX: Int = 0,
    val rightY: Int = 0,
    val leftTrigger: Int = 0,
    val rightTrigger: Int = 0,
) {
    fun isPressed(button: Button) = buttons and button.mask != 0

    fun with(button: Button, pressed: Boolean) =
        copy(buttons = if (pressed) buttons or button.mask else buttons and button.mask.inv())

    companion object {
        val NEUTRAL = PadState()
    }
}

class ProtoException(val kind: String, message: String) : Exception(message)

object Report {
    const val LENGTH = 13
    private val RESERVED_BUTTONS = Button.entries.fold(0xFFFF) { bits, b -> bits and b.mask.inv() }

    fun encode(s: PadState): ByteArray {
        val out = ByteArray(LENGTH)
        putU16(out, 0, s.buttons and RESERVED_BUTTONS.inv() and 0xFFFF)
        out[2] = s.hat.coerceIn(0, Hat.NEUTRAL).toByte()
        putAxis(out, 3, s.leftX)
        putAxis(out, 5, s.leftY)
        putAxis(out, 7, s.rightX)
        putAxis(out, 9, s.rightY)
        out[11] = s.leftTrigger.coerceIn(0, 255).toByte()
        out[12] = s.rightTrigger.coerceIn(0, 255).toByte()
        return out
    }

    /** Strict, like the Rust side: rejects anything [encode] would never produce. */
    fun decode(b: ByteArray, offset: Int = 0): PadState {
        val buttons = u16(b, offset)
        if (buttons and RESERVED_BUTTONS != 0) throw ProtoException("bad_state", "reserved button bits")
        val hatByte = b[offset + 2].toInt() and 0xFF
        if (hatByte > Hat.NEUTRAL) throw ProtoException("bad_state", "hat $hatByte")
        return PadState(
            buttons = buttons,
            hat = hatByte,
            leftX = axis(b, offset + 3),
            leftY = axis(b, offset + 5),
            rightX = axis(b, offset + 7),
            rightY = axis(b, offset + 9),
            leftTrigger = b[offset + 11].toInt() and 0xFF,
            rightTrigger = b[offset + 12].toInt() and 0xFF,
        )
    }

    private fun putAxis(out: ByteArray, at: Int, v: Int) = putU16(out, at, v.coerceIn(AXIS_MIN, AXIS_MAX) and 0xFFFF)

    private fun axis(b: ByteArray, at: Int): Int {
        val v = u16(b, at).toShort().toInt()
        if (v < AXIS_MIN) throw ProtoException("bad_state", "axis -32768")
        return v
    }

    private fun putU16(out: ByteArray, at: Int, v: Int) {
        out[at] = v.toByte()
        out[at + 1] = (v shr 8).toByte()
    }

    private fun u16(b: ByteArray, at: Int) = (b[at].toInt() and 0xFF) or ((b[at + 1].toInt() and 0xFF) shl 8)
}
