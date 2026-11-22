package io.github.sendingstones.controller

import io.github.sendingstones.proto.AXIS_MAX
import io.github.sendingstones.proto.Button
import io.github.sendingstones.proto.Hat
import io.github.sendingstones.proto.PadState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class InputModelTest {
    private val w = 2400f
    private val h = 1080f
    private val travel = Layout.STICK_TRAVEL * h

    private fun centerOf(button: Button) =
        Layout.controls.filterIsInstance<Control.Press>().first { it.button == button }.spot.center(w, h)

    private val dpad = Layout.controls.filterIsInstance<Control.Dpad>().first().spot

    @Test
    fun deadZoneAndFullDeflection() {
        val s = StickSettings()
        assertEquals(0 to 0, StickMath.axes(Vec(0.09f * travel, 0f), travel, s))
        assertEquals(AXIS_MAX to 0, StickMath.axes(Vec(travel, 0f), travel, s))
        assertEquals(0 to -AXIS_MAX, StickMath.axes(Vec(0f, -2 * travel), travel, s))
    }

    @Test
    fun deflectionRampsFromTheEdgeOfTheDeadZone() {
        val (x, _) = StickMath.axes(Vec(0.55f * travel, 0f), travel, StickSettings())
        assertEquals(AXIS_MAX / 2.0, x.toDouble(), 30.0)
    }

    @Test
    fun hatDirectionsGoClockwiseFromUp() {
        val r = 100f
        assertEquals(Hat.UP, StickMath.hat(Vec(0f, -r), r))
        assertEquals(Hat.UP_RIGHT, StickMath.hat(Vec(r, -r), r))
        assertEquals(Hat.RIGHT, StickMath.hat(Vec(r, 0f), r))
        assertEquals(Hat.DOWN, StickMath.hat(Vec(0f, r), r))
        assertEquals(Hat.UP_LEFT, StickMath.hat(Vec(-r, -r), r))
        assertEquals(Hat.NEUTRAL, StickMath.hat(Vec(10f, 0f), r))
    }

    @Test
    fun stickPlusTwoButtonsAtOnce() {
        val m = InputModel(w, h)
        val thumb = Vec(400f, 700f)
        m.down(1, thumb)
        m.move(1, Vec(thumb.x, thumb.y - travel))
        assertTrue(m.down(2, centerOf(Button.A)))
        assertTrue(m.down(3, centerOf(Button.B)))
        val s = m.state()
        assertEquals(-AXIS_MAX, s.leftY)
        assertTrue(s.isPressed(Button.A) && s.isPressed(Button.B))
        m.up(1)
        assertEquals(0, m.state().leftY)
        assertTrue(m.state().isPressed(Button.A))
    }

    @Test
    fun aFingerKeepsItsButtonEvenWhenItSlidesOff() {
        val m = InputModel(w, h)
        m.down(1, centerOf(Button.X))
        m.move(1, centerOf(Button.Y))
        assertEquals(PadState.NEUTRAL.with(Button.X, true), m.state())
    }

    @Test
    fun dpadAllowsSlidingBetweenDirections() {
        val m = InputModel(w, h)
        val c = dpad.center(w, h)
        val r = dpad.radius(h)
        m.down(1, Vec(c.x, c.y - r * 0.8f))
        assertEquals(Hat.UP, m.state().hat)
        assertTrue(m.move(1, Vec(c.x + r * 0.8f, c.y)))
        assertEquals(Hat.RIGHT, m.state().hat)
    }

    @Test
    fun oneFingerPerStickAndSwapForLeftHanders() {
        val m = InputModel(w, h, StickSettings(swapSticks = true))
        m.down(1, Vec(400f, 700f))
        m.move(1, Vec(400f + travel, 700f))
        m.down(2, Vec(500f, 650f))
        m.move(2, Vec(500f, 650f + travel))
        val s = m.state()
        assertEquals("left half drives the right stick", AXIS_MAX, s.rightX)
        assertEquals("second finger on the same half is ignored", 0, s.rightY)
        assertEquals(0, s.leftX)
    }

    @Test
    fun triggersAreDigital() {
        val m = InputModel(w, h)
        val lt = Layout.controls.filterIsInstance<Control.Trigger>().first { it.left }.spot.center(w, h)
        m.down(1, lt)
        assertEquals(255, m.state().leftTrigger)
        m.releaseAll()
        assertEquals(PadState.NEUTRAL, m.state())
    }

    @Test
    fun topBarIsNotAStickZone() {
        val m = InputModel(w, h)
        assertFalse(m.down(1, Vec(w / 2, 10f)))
        assertTrue(m.active.isEmpty())
    }

    @Test
    fun controlsDoNotOverlapOnCommonScreens() {
        for ((sw, sh) in listOf(2400f to 1080f, 1920f to 1080f, 2340f to 1080f, 1600f to 720f)) {
            val spots = Layout.controls.map { it.spot.center(sw, sh) to it.spot.radius(sh) }
            for (i in spots.indices) for (j in i + 1 until spots.size) {
                val (a, ra) = spots[i]
                val (b, rb) = spots[j]
                assertTrue("controls $i and $j overlap on ${sw}x$sh", (a - b).length >= ra + rb)
            }
            for ((c, r) in spots) {
                assertTrue("off screen on ${sw}x$sh", c.x - r >= 0 && c.x + r <= sw && c.y - r >= 0 && c.y + r <= sh)
            }
        }
    }
}
