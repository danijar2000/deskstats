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

/**
 * Keys shared with the server, derived from the token the phone got over USB. The token itself
 * is never sent: both sides seal what they say with these keys, as
 * IV (16) | AES-256-CBC ciphertext | HMAC-SHA256 of IV and ciphertext.
 */
class Keys(token: String) {
    private val enc = hmac(token.toByteArray(Charsets.UTF_8), "deskstats-enc".toByteArray(Charsets.UTF_8))
    private val mac = hmac(token.toByteArray(Charsets.UTF_8), "deskstats-mac".toByteArray(Charsets.UTF_8))

    /** The plain text of a sealed answer, or null when it was not sealed with these keys. */
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

    /** A text sealed for the server, as hex for the request line. */
    fun seal(plain: String): String {
        val iv = ByteArray(IV)
        random.nextBytes(iv)
        val cipher = Cipher.getInstance("AES/CBC/PKCS5Padding")
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(enc, "AES"), IvParameterSpec(iv))
        val body = iv + cipher.doFinal(plain.toByteArray(Charsets.UTF_8))
        return hex(body + hmac(mac, body))
    }

    /** A fresh random value that ties an answer to the request it was sent for. */
    fun nonce(): String {
        val bytes = ByteArray(16)
        random.nextBytes(bytes)
        return hex(bytes)
    }

    /** Whether a beacon text was signed with these keys. */
    fun verify(text: String, signature: String): Boolean =
        MessageDigest.isEqual(hex(hmac(mac, text.toByteArray(Charsets.UTF_8))).toByteArray(), signature.toByteArray())

    private companion object {
        const val IV = 16
        const val TAG = 32
        // Only IVs and nonces come from it, which have to be unique, not secret; the weak
        // seeding of Android 4.1-4.3 that lint warns about does not matter for them.
        @android.annotation.SuppressLint("TrulyRandom")
        val random = java.security.SecureRandom()

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
 * Listens for the server's beacon `DESKSTATS2 <ip> <port> <time> <hmac>` on UDP port 8788 and
 * reports the Mac's Wi-Fi address. Only a beacon signed with our token, sent from the address
 * it names, naming a private network address and newer than the last one is taken, so another
 * device can neither point the panel at itself nor replay an old beacon.
 */
class Beacon(private val keys: () -> Keys?, private val found: (host: String, port: Int) -> Unit) : Thread("beacon") {
    @Volatile private var socket: DatagramSocket? = null

    override fun run() {
        val s = try {
            DatagramSocket(null).apply {
                reuseAddress = true
                broadcast = true
                soTimeout = 2000
                bind(InetSocketAddress(PORT))
            }
        } catch (e: Exception) {
            return
        }
        socket = s
        val buf = ByteArray(512)
        var signer: Keys? = null
        var newest = 0L
        try {
            while (!isInterrupted) {
                val packet = DatagramPacket(buf, buf.size)
                try {
                    s.receive(packet)
                } catch (e: SocketTimeoutException) {
                    continue
                }
                val parts = String(packet.data, 0, packet.length, Charsets.UTF_8).trim().split(' ')
                if (parts.size != 5 || parts[0] != "DESKSTATS2") continue
                val host = parts[1]
                val port = parts[2].toIntOrNull() ?: continue
                val time = parts[3].toLongOrNull() ?: continue
                if (port !in 1..65535) continue
                if (packet.address?.hostAddress != host || !Settings.isPrivateAddress(host)) continue
                val k = keys() ?: continue
                // A new token starts the count of beacon times anew.
                if (k !== signer) {
                    signer = k
                    newest = 0L
                }
                if (time <= newest || !k.verify("DESKSTATS2 $host $port $time", parts[4])) continue
                newest = time
                found(host, port)
            }
        } catch (e: Exception) {
            // Closed by stopListening(), or the network went away; the next onStart starts again.
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
    }
}
