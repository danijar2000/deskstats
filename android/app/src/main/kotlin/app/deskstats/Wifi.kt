package app.deskstats

import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetSocketAddress
import java.net.SocketTimeoutException
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.Mac
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

class Keys private constructor(private val enc: ByteArray, private val mac: ByteArray) {
    constructor(token: String) : this(derive(token.toByteArray(Charsets.UTF_8), "deskstats-enc"), derive(token.toByteArray(Charsets.UTF_8), "deskstats-mac"))

    fun open(sealed: ByteArray): String? {
        if (sealed.size < IV + 16 + TAG) return null
        val body = sealed.copyOfRange(0, sealed.size - TAG)
        val tag = sealed.copyOfRange(sealed.size - TAG, sealed.size)
        if (!MessageDigest.isEqual(hmac(mac, body), tag)) return null
        return try {
            val cipher = Cipher.getInstance("AES/CBC/PKCS5Padding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(enc, "AES"), IvParameterSpec(body, 0, IV))
            String(cipher.doFinal(body, IV, body.size - IV), Charsets.UTF_8)
        } catch (e: Exception) {
            null
        }
    }

    fun seal(plain: String): String {
        val iv = ByteArray(IV)
        random.nextBytes(iv)
        val cipher = Cipher.getInstance("AES/CBC/PKCS5Padding")
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(enc, "AES"), IvParameterSpec(iv))
        val body = iv + cipher.doFinal(plain.toByteArray(Charsets.UTF_8))
        return hex(body + hmac(mac, body))
    }

    fun nonce(): String {
        val bytes = ByteArray(16)
        random.nextBytes(bytes)
        return hex(bytes)
    }

    fun verify(text: String, signature: String): Boolean =
        MessageDigest.isEqual(hex(hmac(mac, text.toByteArray(Charsets.UTF_8))).toByteArray(), signature.toByteArray())

    companion object {
        private const val IV = 16
        private const val TAG = 32
        @android.annotation.SuppressLint("TrulyRandom")
        val random = java.security.SecureRandom()

        /** Keys for the token the Mac hands over when it lets the phone in over Wi‑Fi, from the ECDH secret. */
        fun pairing(secret: ByteArray) = Keys(derive(secret, "deskstats-pair-enc"), derive(secret, "deskstats-pair-mac"))

        private fun derive(key: ByteArray, label: String) = hmac(key, label.toByteArray(Charsets.UTF_8))

        fun unhex(text: String): ByteArray? {
            if (text.length % 2 != 0) return null
            val out = ByteArray(text.length / 2)
            for (i in out.indices) {
                val hi = Character.digit(text[i * 2], 16)
                val lo = Character.digit(text[i * 2 + 1], 16)
                if (hi < 0 || lo < 0) return null
                out[i] = (hi * 16 + lo).toByte()
            }
            return out
        }

        fun hmac(key: ByteArray, data: ByteArray): ByteArray {
            val m = Mac.getInstance("HmacSHA256")
            m.init(SecretKeySpec(key, "HmacSHA256"))
            return m.doFinal(data)
        }

        fun hex(bytes: ByteArray): String {
            val out = StringBuilder(bytes.size * 2)
            for (b in bytes) {
                val v = b.toInt() and 0xFF
                out.append("0123456789abcdef"[v shr 4]).append("0123456789abcdef"[v and 0xF])
            }
            return out.toString()
        }
    }
}

/**
 * Listens on the Wi‑Fi for two kinds of broadcasts: the open announcements every deskstats Mac sends (who it is, for the
 * list of Macs nearby) and the beacons the paired Macs sign with their token (where each of them is now).
 */
class Beacon(
    private val servers: () -> List<Server>,
    private val announced: (Nearby) -> Unit,
    private val found: (server: Server, host: String, port: Int) -> Unit,
) : Thread("beacon") {
    @Volatile private var socket: DatagramSocket? = null
    @Volatile private var wanted = true

    /** Asks every Mac on the Wi‑Fi to say who and where it is: Macs say nothing unless asked, so that they can sleep. */
    fun ask() {
        wanted = true
    }

    override fun run() {
        val s = try {
            DatagramSocket(null).apply {
                reuseAddress = true
                broadcast = true
                soTimeout = 500
                bind(InetSocketAddress(PORT))
            }
        } catch (e: Exception) {
            return
        }
        socket = s
        val buf = ByteArray(1024)
        val newest = HashMap<String, Long>()
        try {
            while (!isInterrupted) {
                if (wanted) {
                    wanted = false
                    try {
                        val query = QUERY.toByteArray(Charsets.UTF_8)
                        s.send(DatagramPacket(query, query.size, java.net.InetAddress.getByName("255.255.255.255"), PORT))
                    } catch (e: Exception) {
                    }
                }
                val packet = DatagramPacket(buf, buf.size)
                try {
                    s.receive(packet)
                } catch (e: SocketTimeoutException) {
                    continue
                }
                val message = String(packet.data, 0, packet.length, Charsets.UTF_8).trim()
                if (message.startsWith(Nearby.ANNOUNCE)) {
                    val from = packet.address?.hostAddress ?: continue
                    Nearby.parse(message.substring(Nearby.ANNOUNCE.length), from)?.let(announced)
                    continue
                }
                val parts = message.split(' ')
                if (parts.size != 5 || parts[0] != "DESKSTATS2") continue
                val host = parts[1]
                val port = parts[2].toIntOrNull() ?: continue
                val time = parts[3].toLongOrNull() ?: continue
                if (port !in 1..65535) continue
                if (packet.address?.hostAddress != host || !Settings.isPrivateAddress(host)) continue
                val text = "DESKSTATS2 $host $port $time"
                val server = servers().firstOrNull { it.keys.verify(text, parts[4]) } ?: continue
                if (time <= (newest[server.id] ?: 0L)) continue
                newest[server.id] = time
                found(server, host, port)
            }
        } catch (e: Exception) {
        } finally {
            s.close()
        }
    }

    fun stopListening() {
        interrupt()
        socket?.close()
    }

    companion object {
        const val PORT = 8788
        const val QUERY = "DESKSTATS?"
    }
}
