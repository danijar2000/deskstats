package app.deskstats

import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.security.MessageDigest

/** One Mac the phone is paired with: its token, the name it reports and where it was last heard on Wi‑Fi. */
class Server(val token: String, val name: String = "", val host: String? = null, val port: Int = 0) {
    val id: String = idOf(token)
    val keys: Keys by lazy { Keys(token) }
    val hasWifi get() = host != null && port in 1..65535

    fun named(name: String) = Server(token, name, host, port)

    fun at(host: String, port: Int) = Server(token, name, host, port)

    companion object {
        fun idOf(token: String): String {
            val digest = MessageDigest.getInstance("SHA-256").digest(token.lowercase(java.util.Locale.US).toByteArray(Charsets.UTF_8))
            val out = StringBuilder(16)
            for (i in 0 until 8) {
                val v = digest[i].toInt() and 0xFF
                out.append("0123456789abcdef"[v shr 4]).append("0123456789abcdef"[v and 0xF])
            }
            return out.toString()
        }
    }
}

/** The paired Macs in the order they were added, and the one the panel shows. Immutable: every change makes a new list. */
class Servers(val list: List<Server>, activeId: String?) {
    val active: Server? = list.firstOrNull { it.id == activeId } ?: list.firstOrNull()
    val index get() = active?.let { a -> list.indexOfFirst { it.id == a.id } } ?: -1

    fun byId(id: String) = list.firstOrNull { it.id == id }

    /** Adds a Mac by its token, or keeps the one already there. A new Mac becomes the active one. */
    fun withToken(token: String): Servers {
        val id = Server.idOf(token)
        if (list.any { it.id == id }) return this
        return Servers(list + Server(token), id)
    }

    fun select(id: String) = if (byId(id) == null) this else Servers(list, id)

    /** The neighbour [step] places away from the active Mac, going round at the ends. */
    fun next(step: Int): Servers {
        val i = index
        if (i < 0 || list.size < 2) return this
        return Servers(list, list[((i + step) % list.size + list.size) % list.size].id)
    }

    fun replace(server: Server) = Servers(list.map { if (it.id == server.id) server else it }, active?.id)

    fun without(id: String): Servers {
        val i = list.indexOfFirst { it.id == id }
        if (i < 0) return this
        val rest = list.filter { it.id != id }
        val keep = active?.id?.takeIf { it != id } ?: rest.getOrNull(i.coerceAtMost(rest.size - 1))?.id
        return Servers(rest, keep)
    }

    fun restored(server: Server, index: Int, chosen: Boolean): Servers {
        if (list.any { it.id == server.id }) return this
        val copy = ArrayList(list)
        copy.add(index.coerceIn(0, copy.size), server)
        return Servers(copy, if (chosen || active == null) server.id else active.id)
    }

    fun displayName(server: Server, unnamed: (Int) -> String): String =
        server.name.ifEmpty { unnamed(list.indexOfFirst { it.id == server.id } + 1) }

    private fun save(prefs: SharedPreferences) {
        val array = JSONArray()
        for (s in list) {
            val o = JSONObject().put("token", s.token).put("name", s.name)
            if (s.hasWifi) o.put("host", s.host).put("port", s.port)
            array.put(o)
        }
        prefs.edit().putString(LIST, array.toString()).putString(ACTIVE, active?.id ?: "").commit()
    }

    companion object {
        private const val LIST = "servers"
        private const val ACTIVE = "server"

        /** Every change goes through here, so the beacon thread and the pairing activity do not overwrite each other. */
        private val lock = Any()

        fun load(prefs: SharedPreferences): Servers = synchronized(lock) {
            val text = prefs.getString(LIST, null) ?: return@synchronized migrate(prefs)
            val list = ArrayList<Server>()
            try {
                val array = JSONArray(text)
                for (i in 0 until array.length()) {
                    val o = array.optJSONObject(i) ?: continue
                    val token = o.optString("token")
                    if (!validToken(token) || list.any { it.id == Server.idOf(token) }) continue
                    val host = o.optString("host").takeIf { it.isNotEmpty() && Settings.isPrivateAddress(it) }
                    list.add(Server(token, o.optString("name"), host, if (host == null) 0 else o.optInt("port")))
                }
            } catch (e: JSONException) {
            }
            Servers(list, prefs.getString(ACTIVE, null))
        }

        fun update(prefs: SharedPreferences, change: (Servers) -> Servers): Servers = synchronized(lock) {
            val old = load(prefs)
            val new = change(old)
            if (new !== old) new.save(prefs)
            new
        }

        /** Before several Macs: one token and one Wi‑Fi address kept under their own keys. */
        private fun migrate(prefs: SharedPreferences): Servers {
            val token = prefs.getString(Settings.TOKEN, null)?.trim() ?: ""
            if (!validToken(token)) return Servers(emptyList(), null)
            val host = prefs.getString(Settings.WIFI_HOST, null)?.takeIf { Settings.isPrivateAddress(it) }
            val port = prefs.getInt(Settings.WIFI_PORT, 0)
            val server = if (host != null && port in 1..65535) Server(token, "", host, port) else Server(token)
            val servers = Servers(listOf(server), server.id)
            servers.save(prefs)
            prefs.edit().remove(Settings.TOKEN).remove(Settings.WIFI_HOST).remove(Settings.WIFI_PORT).commit()
            return servers
        }

        fun validToken(token: String) =
            token.length in 32..128 && token.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }
    }
}
