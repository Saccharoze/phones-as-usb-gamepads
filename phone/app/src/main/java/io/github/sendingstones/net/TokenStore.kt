package io.github.sendingstones.net

import android.content.Context
import androidx.core.content.edit

/** Where one client keeps its session token between app runs. */
interface TokenMemory {
    fun load(): Long
    fun save(token: Long)
}

/**
 * The token of each board slot this phone holds, kept across app restarts:
 * a restarted app resumes its slot instead of waiting for the board's 120 s
 * reservation to run out.
 */
class TokenStore(context: Context) {
    private val prefs = context.getSharedPreferences("tokens", Context.MODE_PRIVATE)

    fun holds(board: Board, slot: Int): Boolean = prefs.getLong(key(board, slot), 0L) != 0L

    fun forSlot(board: Board, slot: Int): TokenMemory = object : TokenMemory {
        override fun load() = prefs.getLong(key(board, slot), 0L)
        override fun save(token: Long) = prefs.edit {
            if (token == 0L) remove(key(board, slot)) else putLong(key(board, slot), token)
        }
    }

    // The advertised name survives a new DHCP address; a typed address is its own name.
    private fun key(board: Board, slot: Int) = "${board.name}#$slot"
}
