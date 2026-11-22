package io.github.sendingstones.proto

/**
 * Just enough JSON to read vectors.json in JVM tests: android.jar's org.json
 * is a stub there, and the project allows no third-party libraries.
 * Objects -> Map, arrays -> List, numbers -> Long, plus String/Boolean/null.
 */
class MiniJson private constructor(private val s: String) {
    private var i = 0

    companion object {
        fun parse(text: String): Any? = MiniJson(text).run { value().also { skipWs(); check(i == s.length) { "trailing data at $i" } } }
    }

    private fun value(): Any? {
        skipWs()
        return when (val c = s[i]) {
            '{' -> obj()
            '[' -> arr()
            '"' -> str()
            't' -> literal("true", true)
            'f' -> literal("false", false)
            'n' -> literal("null", null)
            else -> if (c == '-' || c.isDigit()) num() else error("unexpected '$c' at $i")
        }
    }

    private fun obj(): Map<String, Any?> {
        val out = LinkedHashMap<String, Any?>()
        i++
        skipWs()
        if (s[i] == '}') return out.also { i++ }
        while (true) {
            skipWs()
            val key = str()
            skipWs(); expect(':')
            out[key] = value()
            skipWs()
            if (s[i] == ',') { i++; continue }
            expect('}')
            return out
        }
    }

    private fun arr(): List<Any?> {
        val out = ArrayList<Any?>()
        i++
        skipWs()
        if (s[i] == ']') return out.also { i++ }
        while (true) {
            out.add(value())
            skipWs()
            if (s[i] == ',') { i++; continue }
            expect(']')
            return out
        }
    }

    private fun str(): String {
        expect('"')
        val sb = StringBuilder()
        while (s[i] != '"') {
            val c = s[i++]
            if (c != '\\') { sb.append(c); continue }
            when (val e = s[i++]) {
                '"', '\\', '/' -> sb.append(e)
                'n' -> sb.append('\n')
                't' -> sb.append('\t')
                'r' -> sb.append('\r')
                'b' -> sb.append('\b')
                'f' -> sb.append('\u000c')
                'u' -> { sb.append(s.substring(i, i + 4).toInt(16).toChar()); i += 4 }
                else -> error("bad escape \\$e")
            }
        }
        i++
        return sb.toString()
    }

    private fun num(): Long {
        val start = i
        if (s[i] == '-') i++
        while (i < s.length && s[i].isDigit()) i++
        check(i >= s.length || s[i] !in ".eE") { "only integers are expected in vectors.json" }
        return s.substring(start, i).toLong()
    }

    private fun literal(word: String, v: Any?): Any? {
        check(s.startsWith(word, i)) { "expected $word at $i" }
        i += word.length
        return v
    }

    private fun expect(c: Char) {
        check(s[i] == c) { "expected '$c' at $i, got '${s[i]}'" }
        i++
    }

    private fun skipWs() {
        while (i < s.length && s[i].isWhitespace()) i++
    }
}
