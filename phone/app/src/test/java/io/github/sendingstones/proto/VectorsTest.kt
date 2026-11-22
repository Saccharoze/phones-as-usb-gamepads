package io.github.sendingstones.proto

import java.io.File
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** Same golden vectors as the Rust `proto` tests (board/proto/testdata/vectors.json). */
@Suppress("UNCHECKED_CAST")
class VectorsTest {
    private val vectors: Map<String, Any?> by lazy {
        val path = System.getProperty("vectors") ?: error("system property 'vectors' not set by Gradle")
        MiniJson.parse(File(path).readText()) as Map<String, Any?>
    }

    private fun list(key: String) = vectors[key] as List<Map<String, Any?>>

    private fun hex(s: String) = ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

    private fun Map<String, Any?>.int(key: String) = (this[key] as Long).toInt()

    private fun Map<String, Any?>.token() = java.lang.Long.parseUnsignedLong(this["token"] as String, 16)

    private fun state(m: Map<String, Any?>) = PadState(
        buttons = m.int("buttons"), hat = m.int("hat"),
        leftX = m.int("left_x"), leftY = m.int("left_y"),
        rightX = m.int("right_x"), rightY = m.int("right_y"),
        leftTrigger = m.int("left_trigger"), rightTrigger = m.int("right_trigger"),
    )

    private fun frame(m: Map<String, Any?>): Frame = when (m["type"]) {
        "hello" -> Frame.Hello(m.int("slot"), m.token(), (m["pin"] as Long?)?.toInt())
        "welcome" -> Frame.Welcome(m.int("slot"), m.token())
        "error" -> Frame.Error(ErrorCode.of(m.int("code"))!!)
        "state" -> Frame.State(m.token(), m.int("seq"), state(m["state"] as Map<String, Any?>))
        "heartbeat" -> Frame.Heartbeat(m.token(), m.int("seq"))
        "ping" -> Frame.Ping(m.token(), m["id"] as Long, m["phone_time_ms"] as Long)
        "pong" -> Frame.Pong(m.token(), m["id"] as Long, m["phone_time_ms"] as Long)
        "stats" -> Frame.Stats(m.token(), m.int("rtt_p50_ms"), m.int("rtt_p95_ms"))
        "release" -> Frame.Release(m.token())
        "status_req" -> Frame.StatusReq
        "status" -> Frame.Status(m["text"] as String)
        else -> error("unknown type ${m["type"]}")
    }

    @Test
    fun reportsMatchVectors() {
        for (case in list("reports")) {
            val expected = hex(case["hex"] as String)
            val s = state(case["state"] as Map<String, Any?>)
            assertArrayEquals("encode ${case["name"]}", expected, Report.encode(s))
            assertEquals("decode ${case["name"]}", s, Report.decode(expected))
        }
    }

    @Test
    fun framesMatchVectors() {
        for (case in list("frames")) {
            val expected = hex(case["hex"] as String)
            val f = frame(case["frame"] as Map<String, Any?>)
            assertArrayEquals("encode ${case["name"]}", expected, f.encode())
            assertEquals("decode ${case["name"]}", f, Frame.decode(expected))
        }
    }

    @Test
    fun invalidFramesAreRejectedForTheRightReason() {
        for (case in list("invalid_frames")) {
            try {
                Frame.decode(hex(case["hex"] as String))
                fail("${case["name"]} must be rejected")
            } catch (e: ProtoException) {
                assertEquals(case["name"].toString(), case["error"], e.kind)
            }
        }
    }

    @Test
    fun everyFrameTypeHasAVector() {
        val types = list("frames").map { (it["frame"] as Map<String, Any?>)["type"] }.toSet()
        assertEquals(11, types.size)
    }

    @Test
    fun sequenceOrderSurvivesWrap() {
        assertTrue(seqIsNewer(1, 0))
        assertTrue(seqIsNewer(0, 65535))
        assertTrue(seqIsNewer(10, 65530))
        assertFalse(seqIsNewer(5, 5))
        assertFalse(seqIsNewer(4, 5))
        assertFalse(seqIsNewer(65530, 10))
    }
}
