package io.github.sendingstones.controller

import io.github.sendingstones.proto.Button

/**
 * Every on-screen position lives here, so the layout can be tuned without
 * touching input logic. A control sits at an anchor (fractions of the screen
 * width and height) plus an offset and a radius in units of screen height,
 * which keeps buttons round and thumb-sized on any aspect ratio.
 */
data class Spot(val ax: Float, val ay: Float, val dx: Float = 0f, val dy: Float = 0f, val r: Float) {
    fun center(w: Float, h: Float) = Vec(ax * w + dx * h, ay * h + dy * h)
    fun radius(h: Float) = r * h
}

sealed interface Control {
    val spot: Spot

    data class Press(val button: Button, override val spot: Spot) : Control
    data class Trigger(val left: Boolean, override val spot: Spot) : Control
    data class Dpad(override val spot: Spot) : Control
}

object Layout {
    private const val FACE = 0.075f
    private const val SHOULDER = 0.07f
    private const val SMALL = 0.055f

    /** ABXY diamond, lower right; centre offset kept in height units. */
    private val faceCentre = Spot(ax = 1f, ay = 0.72f, dx = -0.30f, r = 0f)
    private fun face(dx: Float, dy: Float) =
        faceCentre.copy(dx = faceCentre.dx + dx, dy = faceCentre.dy + dy, r = FACE)

    val controls: List<Control> = listOf(
        Control.Trigger(left = true, Spot(ax = 0f, ay = 0.11f, dx = 0.12f, r = SHOULDER)),
        Control.Press(Button.LB, Spot(ax = 0f, ay = 0.11f, dx = 0.30f, r = SHOULDER)),
        Control.Trigger(left = false, Spot(ax = 1f, ay = 0.11f, dx = -0.12f, r = SHOULDER)),
        Control.Press(Button.RB, Spot(ax = 1f, ay = 0.11f, dx = -0.30f, r = SHOULDER)),
        Control.Press(Button.View, Spot(ax = 0.5f, ay = 0.13f, dx = -0.16f, r = SMALL)),
        Control.Press(Button.Menu, Spot(ax = 0.5f, ay = 0.13f, dx = 0.16f, r = SMALL)),
        Control.Press(Button.L3, Spot(ax = 0.5f, ay = 0.42f, dx = -0.20f, r = SMALL)),
        Control.Press(Button.R3, Spot(ax = 0.5f, ay = 0.42f, dx = 0.20f, r = SMALL)),
        // Inboard of the left thumb's resting place, which belongs to the stick.
        Control.Dpad(Spot(ax = 0.5f, ay = 0.78f, dx = -0.42f, r = 0.15f)),
        Control.Press(Button.A, face(0f, 0.15f)),
        Control.Press(Button.B, face(0.15f, 0f)),
        Control.Press(Button.X, face(-0.15f, 0f)),
        Control.Press(Button.Y, face(0f, -0.15f)),
    )

    /** Touch target is a bit larger than the drawn circle: thumbs miss. */
    const val HIT_SLOP = 1.25f

    /** Floating sticks start below this line (fraction of height), above it is the top bar. */
    const val STICK_ZONE_TOP = 0.24f

    /** Thumb travel for full deflection, in units of screen height. */
    const val STICK_TRAVEL = 0.16f

    /** Inside this fraction of the D-pad radius no direction is pressed. */
    const val DPAD_DEAD = 0.25f
}
