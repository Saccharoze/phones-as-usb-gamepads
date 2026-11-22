package io.github.sendingstones.proto

import java.nio.ByteBuffer
import java.nio.ByteOrder

const val DEFAULT_PORT = 40400
const val SERVICE_TYPE = "_sendingstones._udp"
const val NO_PIN = 0xFFFF
const val MAX_PIN = 9999
private const val VERSION: Byte = 1
private const val HEADER_LEN = 4

enum class ErrorCode(val code: Int) {
    SlotTaken(1), BadPin(2), NoSuchSlot(3), BadFrame(4), UnknownToken(5);

    companion object {
        fun of(code: Int) = entries.firstOrNull { it.code == code }
    }
}

/**
 * UDP frames: header `S S <version> <type>`, integers little-endian.
 * Tokens are u64 carried in a Long's bits; u32 fields in a Long; u16 in an Int.
 */
sealed class Frame(val type: Int) {
    data class Hello(val slot: Int, val token: Long, val pin: Int?) : Frame(0x01)
    data class Welcome(val slot: Int, val token: Long) : Frame(0x02)
    data class Error(val code: ErrorCode) : Frame(0x03)
    data class State(val token: Long, val seq: Int, val state: PadState) : Frame(0x10)
    data class Heartbeat(val token: Long, val seq: Int) : Frame(0x11)
    data class Ping(val token: Long, val id: Long, val phoneTimeMs: Long) : Frame(0x20)
    data class Pong(val token: Long, val id: Long, val phoneTimeMs: Long) : Frame(0x21)
    data class Stats(val token: Long, val rttP50Ms: Int, val rttP95Ms: Int) : Frame(0x22)
    data class Release(val token: Long) : Frame(0x30)
    data object StatusReq : Frame(0x40)
    data class Status(val json: String) : Frame(0x41)

    fun encode(): ByteArray {
        val payload = payload()
        val buf = ByteBuffer.allocate(HEADER_LEN + payload.size).order(ByteOrder.LITTLE_ENDIAN)
        buf.put('S'.code.toByte()).put('S'.code.toByte()).put(VERSION).put(type.toByte()).put(payload)
        return buf.array()
    }

    private fun payload(): ByteArray = when (this) {
        is Hello -> le(11) { put(slot.toByte()).putLong(token).putShort((pin ?: NO_PIN).toShort()) }
        is Welcome -> le(9) { put(slot.toByte()).putLong(token) }
        is Error -> byteArrayOf(code.code.toByte())
        is State -> le(10 + Report.LENGTH) { putLong(token).putShort(seq.toShort()).put(Report.encode(state)) }
        is Heartbeat -> le(10) { putLong(token).putShort(seq.toShort()) }
        is Ping -> le(16) { putLong(token).putInt(id.toInt()).putInt(phoneTimeMs.toInt()) }
        is Pong -> le(16) { putLong(token).putInt(id.toInt()).putInt(phoneTimeMs.toInt()) }
        is Stats -> le(12) { putLong(token).putShort(rttP50Ms.toShort()).putShort(rttP95Ms.toShort()) }
        is Release -> le(8) { putLong(token) }
        StatusReq -> ByteArray(0)
        is Status -> json.toByteArray(Charsets.UTF_8)
    }

    companion object {
        private val PAYLOAD_LEN = mapOf(
            0x01 to 11, 0x02 to 9, 0x03 to 1, 0x10 to 10 + Report.LENGTH, 0x11 to 10,
            0x20 to 16, 0x21 to 16, 0x22 to 12, 0x30 to 8, 0x40 to 0,
        )

        /** Throws [ProtoException] whose `kind` matches the names in vectors.json. */
        fun decode(bytes: ByteArray, length: Int = bytes.size): Frame {
            if (length < HEADER_LEN) throw ProtoException("too_short", "frame shorter than header")
            if (bytes[0] != 'S'.code.toByte() || bytes[1] != 'S'.code.toByte()) throw ProtoException("bad_magic", "bad magic")
            if (bytes[2] != VERSION) throw ProtoException("bad_version", "version ${bytes[2]}")
            val type = bytes[3].toInt() and 0xFF
            val payloadLen = length - HEADER_LEN
            if (type == 0x41) return Status(utf8(bytes, payloadLen))
            val expected = PAYLOAD_LEN[type] ?: throw ProtoException("unknown_type", "type $type")
            if (payloadLen != expected) throw ProtoException("bad_length", "type $type: $payloadLen != $expected")
            val r = ByteBuffer.wrap(bytes, HEADER_LEN, payloadLen).order(ByteOrder.LITTLE_ENDIAN)
            return decodeFixed(type, r, bytes)
        }

        private fun decodeFixed(type: Int, r: ByteBuffer, bytes: ByteArray): Frame = when (type) {
            0x01 -> Hello(r.u8(), r.long, decodePin(r.u16()))
            0x02 -> Welcome(r.u8(), r.long)
            0x03 -> r.u8().let { Error(ErrorCode.of(it) ?: throw ProtoException("bad_error_code", "code $it")) }
            0x10 -> State(r.long, r.u16(), Report.decode(bytes, HEADER_LEN + 10))
            0x11 -> Heartbeat(r.long, r.u16())
            0x20 -> Ping(r.long, r.u32(), r.u32())
            0x21 -> Pong(r.long, r.u32(), r.u32())
            0x22 -> Stats(r.long, r.u16(), r.u16())
            0x30 -> Release(r.long)
            else -> StatusReq
        }

        private fun decodePin(raw: Int): Int? = when {
            raw == NO_PIN -> null
            raw <= MAX_PIN -> raw
            else -> throw ProtoException("bad_pin", "PIN $raw")
        }

        private fun utf8(bytes: ByteArray, payloadLen: Int): String {
            val decoder = Charsets.UTF_8.newDecoder()
            return try {
                decoder.decode(ByteBuffer.wrap(bytes, HEADER_LEN, payloadLen)).toString()
            } catch (e: java.nio.charset.CharacterCodingException) {
                throw ProtoException("bad_status_text", "STATUS is not UTF-8")
            }
        }
    }
}

/** True when [seq] is newer than [last] on the u16 ring (half the ring ahead counts). */
fun seqIsNewer(seq: Int, last: Int): Boolean {
    val ahead = (seq - last) and 0xFFFF
    return ahead != 0 && ahead < 0x8000
}

private inline fun le(size: Int, fill: ByteBuffer.() -> Unit): ByteArray =
    ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN).apply(fill).array()

private fun ByteBuffer.u8() = get().toInt() and 0xFF
private fun ByteBuffer.u16() = short.toInt() and 0xFFFF
private fun ByteBuffer.u32() = int.toLong() and 0xFFFFFFFFL
