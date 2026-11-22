package io.github.sendingstones.net

import io.github.sendingstones.proto.Frame
import io.github.sendingstones.proto.ProtoException
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetSocketAddress
import java.net.SocketTimeoutException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONException
import org.json.JSONObject

data class SlotInfo(val slot: Int, val state: String) {
    val free: Boolean get() = state == "free"
}

data class BoardStatus(val pinRequired: Boolean, val slots: List<SlotInfo>)

/** One STATUS_REQ round trip (retried); null if the board does not answer. */
suspend fun queryStatus(board: InetSocketAddress, tries: Int = 4): BoardStatus? = withContext(Dispatchers.IO) {
    try {
        DatagramSocket().use { socket ->
            socket.soTimeout = 400
            val request = Frame.StatusReq.encode()
            val buf = ByteArray(4096)
            repeat(tries) {
                try {
                    socket.send(DatagramPacket(request, request.size, board))
                    val packet = DatagramPacket(buf, buf.size)
                    socket.receive(packet)
                    val frame = Frame.decode(packet.data, packet.length)
                    if (frame is Frame.Status) return@withContext parseStatus(frame.json)
                } catch (_: SocketTimeoutException) {
                } catch (_: ProtoException) {
                } catch (_: JSONException) {
                }
            }
            null
        }
    } catch (_: java.io.IOException) {
        // No route (Wi-Fi off or switching networks): the same as no answer.
        null
    }
}

private fun parseStatus(json: String): BoardStatus {
    val root = JSONObject(json)
    val slots = root.getJSONArray("slots")
    return BoardStatus(
        pinRequired = root.optBoolean("pin_required"),
        slots = (0 until slots.length()).map {
            val s = slots.getJSONObject(it)
            SlotInfo(s.getInt("slot"), s.getString("state"))
        },
    )
}
