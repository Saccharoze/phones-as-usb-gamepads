package io.github.sendingstones.net

import android.os.SystemClock
import io.github.sendingstones.proto.ErrorCode
import io.github.sendingstones.proto.Frame
import io.github.sendingstones.proto.PadState
import io.github.sendingstones.proto.ProtoException
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetSocketAddress
import java.net.SocketTimeoutException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

sealed interface Link {
    data object Connecting : Link
    data class Connected(val rttP50Ms: Int?, val rttP95Ms: Int?) : Link
    data class Refused(val code: ErrorCode) : Link
    data object NoReply : Link
    data object Paused : Link
}

/**
 * One phone's session with the board. The UI pushes the latest [PadState]
 * with [update]; a send loop turns it into STATE frames (on change, at most
 * once per display frame), keepalives, PINGs and STATS.
 */
class BoardClient(
    private val board: InetSocketAddress,
    val slot: Int,
    private val pin: Int?,
    private val frameMs: Long,
    private val scope: CoroutineScope,
    private val memory: TokenMemory,
) {
    private val socket = DatagramSocket().apply { soTimeout = RECEIVE_POLL_MS }
    private val desired = MutableStateFlow(PadState.NEUTRAL)
    private val replies = Channel<Frame>(Channel.CONFLATED)
    private val _link = MutableStateFlow<Link>(Link.Connecting)
    private val rtt = RttWindow()
    // Touched by the send and receive coroutines, which run on IO threads.
    // A token kept from an earlier run resumes the slot; 0 asks for a new one.
    @Volatile private var token = memory.load()
    @Volatile private var seq = 0
    @Volatile private var session: Job? = null
    private var receiver: Job? = null

    val link: StateFlow<Link> = _link

    fun update(state: PadState) {
        desired.value = state
    }

    /** Claims the slot (or resumes it with the kept token) and starts sending. */
    fun start() {
        if (session?.isActive == true) return
        if (receiver?.isActive != true) receiver = scope.launch(Dispatchers.IO) { receiveLoop() }
        session = scope.launch(Dispatchers.IO) {
            _link.value = Link.Connecting
            if (claim()) sendLoop()
        }
    }

    /** App paused or phone locked: neutral now, then silence; the token is kept. */
    fun pause() {
        session?.cancel()
        session = null
        desired.value = PadState.NEUTRAL
        scope.launch(Dispatchers.IO) { if (token != 0L) sendState(PadState.NEUTRAL) }
        _link.value = Link.Paused
    }

    /** Leaves for good: neutral, RELEASE, close. */
    fun close() {
        session?.cancel()
        receiver?.cancel()
        val held = token
        memory.save(0)
        // Not on [scope]: when the activity is finishing, its scope is already
        // cancelled and the RELEASE would never leave.
        Thread {
            if (held != 0L) {
                sendState(PadState.NEUTRAL)
                send(Frame.Release(held))
            }
            socket.close()
        }.start()
    }

    /** HELLO every 250 ms until WELCOME or ERROR, 10 tries at most. */
    private suspend fun claim(): Boolean {
        // A reply left over from an earlier claim (a late duplicate) would be
        // read as the answer to this one.
        while (replies.tryReceive().isSuccess) Unit
        repeat(HELLO_TRIES) {
            send(Frame.Hello(slot, token, pin))
            val reply = withTimeoutOrNull(HELLO_RETRY_MS) { replies.receive() }
            when (reply) {
                is Frame.Welcome -> if (reply.slot == slot) {
                    token = reply.token
                    memory.save(token)
                    seq = 0
                    _link.value = Link.Connected(rtt.p50(), rtt.p95())
                    return true
                }
                is Frame.Error -> {
                    // Whatever token was sent no longer holds this slot.
                    token = 0
                    memory.save(0)
                    _link.value = Link.Refused(reply.code)
                    return false
                }
                else -> {}
            }
        }
        _link.value = Link.NoReply
        return false
    }

    private suspend fun sendLoop() {
        var sent: PadState? = null
        var lastSendAt = 0L
        var nextPing = now()
        var nextStats = now() + STATS_EVERY_MS
        var pingId = 0L
        while (scope.isActive) {
            val wait = (lastSendAt + KEEPALIVE_MS - now()).coerceAtLeast(1)
            val changed = withTimeoutOrNull(wait) { desired.first { it != sent } }
            // At most one STATE per display frame.
            val sinceLast = now() - lastSendAt
            if (changed != null && sinceLast < frameMs) delay(frameMs - sinceLast)
            val state = desired.value
            // The keepalive is the full state too, released or not: a lost
            // packet, a release included, is corrected by the next one, and a
            // pad the fail-safe neutralised during a Wi-Fi gap gets its input back.
            sendState(state)
            sent = state
            lastSendAt = now()
            if (now() >= nextPing) {
                send(Frame.Ping(token, pingId++, now() and 0xFFFFFFFFL))
                nextPing += PING_EVERY_MS
            }
            if (now() >= nextStats) {
                send(Frame.Stats(token, rtt.p50() ?: 0, rtt.p95() ?: 0))
                nextStats += STATS_EVERY_MS
            }
        }
    }

    private suspend fun receiveLoop() {
        val buf = ByteArray(2048)
        while (scope.isActive && !socket.isClosed) {
            val packet = DatagramPacket(buf, buf.size)
            try {
                socket.receive(packet)
            } catch (_: SocketTimeoutException) {
                continue
            } catch (_: java.io.IOException) {
                return
            }
            val frame = try {
                Frame.decode(packet.data, packet.length)
            } catch (_: ProtoException) {
                continue
            }
            onFrame(frame)
        }
    }

    private fun onFrame(frame: Frame) {
        when (frame) {
            is Frame.Pong -> {
                rtt.add(((now() and 0xFFFFFFFFL) - frame.phoneTimeMs).toInt())
                if (_link.value is Link.Connected) _link.value = Link.Connected(rtt.p50(), rtt.p95())
            }
            // The board restarted or another phone took the slot over: claim again.
            is Frame.Error -> if (frame.code == ErrorCode.UnknownToken && session?.isActive == true) {
                token = 0
                memory.save(0)
                session?.cancel()
                session = scope.launch(Dispatchers.IO) { if (claim()) sendLoop() }
            } else {
                replies.trySend(frame)
            }
            else -> replies.trySend(frame)
        }
    }

    private fun sendState(state: PadState) = send(Frame.State(token, nextSeq(), state))

    private fun nextSeq(): Int {
        seq = (seq + 1) and 0xFFFF
        return seq
    }

    private fun send(frame: Frame) {
        val bytes = frame.encode()
        try {
            socket.send(DatagramPacket(bytes, bytes.size, board))
        } catch (_: java.io.IOException) {
            // Wi-Fi gone for a moment: the next snapshot carries the full state anyway.
        }
    }

    private fun now() = SystemClock.elapsedRealtime()

    private companion object {
        const val RECEIVE_POLL_MS = 200
        const val HELLO_RETRY_MS = 250L
        const val HELLO_TRIES = 10
        const val KEEPALIVE_MS = 100L
        const val PING_EVERY_MS = 1000L
        const val STATS_EVERY_MS = 5000L
    }
}

/** Last 64 round trips; p50/p95 by nearest rank. */
class RttWindow(private val size: Int = 64) {
    private val samples = ArrayDeque<Int>()

    @Synchronized
    fun add(ms: Int) {
        if (ms < 0 || ms > 60_000) return
        if (samples.size == size) samples.removeFirst()
        samples.addLast(ms)
    }

    fun p50() = percentile(50)
    fun p95() = percentile(95)

    @Synchronized
    private fun percentile(pct: Int): Int? {
        if (samples.isEmpty()) return null
        val sorted = samples.sorted()
        val rank = maxOf(1, (pct * sorted.size + 99) / 100)
        return sorted[rank - 1]
    }
}
