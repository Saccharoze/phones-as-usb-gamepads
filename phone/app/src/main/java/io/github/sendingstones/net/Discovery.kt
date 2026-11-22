package io.github.sendingstones.net

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import io.github.sendingstones.proto.SERVICE_TYPE
import java.net.InetSocketAddress
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow

data class Board(val name: String, val address: InetSocketAddress) {
    val label: String get() = "${address.hostString}:${address.port}"
}

/**
 * Emits the boards currently advertising `_sendingstones._udp` on the Wi-Fi.
 * Resolution is serialised: NsdManager rejects a second resolve while one
 * is running on older Android versions.
 */
fun discoverBoards(context: Context): Flow<List<Board>> = callbackFlow {
    val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    val boards = LinkedHashMap<String, Board>()
    val pending = ArrayDeque<NsdServiceInfo>()
    var resolving = false

    fun publish() {
        trySend(boards.values.toList())
    }

    fun resolveNext() {
        if (resolving) return
        val next = pending.removeFirstOrNull() ?: return
        resolving = true
        @Suppress("DEPRECATION") // the replacement needs API 34; minSdk is 26
        nsd.resolveService(next, object : NsdManager.ResolveListener {
            override fun onServiceResolved(info: NsdServiceInfo) = synchronized(boards) {
                @Suppress("DEPRECATION")
                val host = info.host
                if (host != null) {
                    boards[info.serviceName] = Board(info.serviceName, InetSocketAddress(host, info.port))
                    publish()
                }
                resolving = false
                resolveNext()
            }

            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) = synchronized(boards) {
                resolving = false
                resolveNext()
            }
        })
    }

    val listener = object : NsdManager.DiscoveryListener {
        override fun onServiceFound(info: NsdServiceInfo) {
            synchronized(boards) {
                pending.addLast(info)
                resolveNext()
            }
        }

        override fun onServiceLost(info: NsdServiceInfo) {
            synchronized(boards) {
                boards.remove(info.serviceName)
                publish()
            }
        }

        override fun onDiscoveryStarted(serviceType: String) {}
        override fun onDiscoveryStopped(serviceType: String) {}
        override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
            close(IllegalStateException("mDNS discovery failed ($errorCode)"))
        }
        override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
    }

    publish()
    nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
    awaitClose { runCatching { nsd.stopServiceDiscovery(listener) } }
}
