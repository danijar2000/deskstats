package app.deskstats

import android.os.Build
import org.json.JSONException
import org.json.JSONObject
import java.io.IOException
import java.math.BigInteger
import java.net.HttpURLConnection
import java.net.Inet4Address
import java.net.NetworkInterface
import java.net.URL
import java.net.URLEncoder
import java.util.Locale
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** A Mac on the Wi‑Fi that announced itself, paired or not. [id] matches [Server.id] when the phone knows its token. */
class Nearby(val id: String, val name: String, val model: String, val host: String, val port: Int, val seen: Long) {
    companion object {
        const val ANNOUNCE = "DESKSTATS3 "

        /** What the Mac announces and what /info answers: {"app":"deskstats","id":…,"name":…,"model":…,"port":…}. */
        fun parse(json: String, host: String): Nearby? = try {
            val o = JSONObject(json)
            val id = o.optString("id")
            val port = o.optInt("port")
            if (o.optString("app") != "deskstats" || id.length != 16 || !id.all { it in '0'..'9' || it in 'a'..'f' } ||
                port !in 1..65535 || !Settings.isPrivateAddress(host)
            ) null else Nearby(id, o.optString("name").trim().take(64), o.optString("model").trim().take(64), host, port, System.currentTimeMillis())
        } catch (e: JSONException) {
            null
        }
    }
}

/** Plain requests to the open part of a Mac's server: /info and /pair. */
object Door {
    class Reply(val code: Int, val json: JSONObject?)

    fun get(host: String, port: Int, target: String, connectTimeout: Int, readTimeout: Int): Reply {
        val c = URL("http://$host:$port$target").openConnection() as HttpURLConnection
        try {
            c.connectTimeout = connectTimeout
            c.readTimeout = readTimeout
            c.useCaches = false
            c.setRequestProperty("X-Deskstats", "1")
            val code = c.responseCode
            val stream = if (code in 200..299) c.inputStream else c.errorStream
            val text = stream?.use { input ->
                val bytes = java.io.ByteArrayOutputStream()
                val buf = ByteArray(4096)
                while (true) {
                    val n = input.read(buf)
                    if (n < 0) break
                    bytes.write(buf, 0, n)
                    if (bytes.size() > 16 * 1024) throw IOException("answer too large")
                }
                bytes.toString("UTF-8")
            }
            val json = try {
                text?.let { JSONObject(it) }
            } catch (e: JSONException) {
                null
            }
            return Reply(code, json)
        } finally {
            c.disconnect()
        }
    }

    /**
     * Asks every address of the phone's own /24 network for /info, a few dozen at a time: Wi‑Fi networks that do not
     * pass broadcasts on still let the phone find the Macs this way, as LocalSend does.
     */
    fun scan(port: Int, found: (Nearby) -> Unit) {
        val own = ownAddress() ?: return
        val prefix = own.substring(0, own.lastIndexOf('.') + 1)
        val pool = Executors.newFixedThreadPool(32)
        for (last in 1..254) {
            val host = prefix + last
            if (host == own) continue
            pool.execute {
                try {
                    val reply = get(host, port, "/info", 600, 1500)
                    if (reply.code == 200) reply.json?.let { Nearby.parse(it.toString(), host) }?.let(found)
                } catch (e: Exception) {
                }
            }
        }
        pool.shutdown()
        try {
            pool.awaitTermination(30, TimeUnit.SECONDS)
        } catch (e: InterruptedException) {
            pool.shutdownNow()
        }
    }

    private fun ownAddress(): String? = try {
        NetworkInterface.getNetworkInterfaces()?.toList().orEmpty()
            .filter { it.isUp && !it.isLoopback }
            .sortedBy { if (it.name.startsWith("wlan")) 0 else 1 }
            .flatMap { it.inetAddresses.toList() }
            .filterIsInstance<Inet4Address>()
            .map { it.hostAddress ?: "" }
            .firstOrNull { Settings.isPrivateAddress(it) }
    } catch (e: Exception) {
        null
    }

    fun phoneName(): String {
        val maker = Build.MANUFACTURER.orEmpty().trim().replaceFirstChar { it.titlecase(Locale.US) }
        val model = Build.MODEL.orEmpty().trim()
        return if (model.startsWith(maker, ignoreCase = true) || maker.isEmpty()) model else "$maker $model"
    }

    fun encode(text: String): String = URLEncoder.encode(text, "UTF-8")
}

/**
 * P-256 (secp256r1) for the one key agreement in pairing, in plain BigInteger arithmetic: old Android versions lack ECDH.
 * Keys live for one pairing only.
 */
class EcKey private constructor(private val d: BigInteger) {
    private val q = mul(d, G)
    val public: ByteArray = encode(q!!)

    /** The X coordinate of d times their point, as the Mac computes it; null for a point that is not on the curve. */
    fun shared(theirs: ByteArray): ByteArray? {
        val point = decode(theirs) ?: return null
        val s = mul(d, point) ?: return null
        return fixed(s.x)
    }

    private class Point(val x: BigInteger, val y: BigInteger)

    companion object {
        private val P = BigInteger("ffffffff00000001000000000000000000000000ffffffffffffffffffffffff", 16)
        private val A = P.subtract(BigInteger.valueOf(3))
        private val B = BigInteger("5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b", 16)
        private val N = BigInteger("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551", 16)
        private val G = Point(
            BigInteger("6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296", 16),
            BigInteger("4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5", 16),
        )
        private val TWO = BigInteger.valueOf(2)
        private val THREE = BigInteger.valueOf(3)

        fun create(): EcKey {
            val bytes = ByteArray(40)
            Keys.random.nextBytes(bytes)
            return EcKey(BigInteger(1, bytes).mod(N.subtract(BigInteger.ONE)).add(BigInteger.ONE))
        }

        private fun add(a: Point?, b: Point?): Point? {
            if (a == null) return b
            if (b == null) return a
            val slope = if (a.x == b.x) {
                if (a.y != b.y || a.y.signum() == 0) return null
                a.x.multiply(a.x).multiply(THREE).add(A).multiply(a.y.multiply(TWO).modInverse(P)).mod(P)
            } else {
                b.y.subtract(a.y).multiply(b.x.subtract(a.x).modInverse(P)).mod(P)
            }
            val x = slope.multiply(slope).subtract(a.x).subtract(b.x).mod(P)
            return Point(x, slope.multiply(a.x.subtract(x)).subtract(a.y).mod(P))
        }

        private fun mul(k: BigInteger, point: Point): Point? {
            var result: Point? = null
            for (i in k.bitLength() - 1 downTo 0) {
                result = add(result, result)
                if (k.testBit(i)) result = add(result, point)
            }
            return result
        }

        private fun fixed(v: BigInteger): ByteArray {
            val raw = v.toByteArray()
            val out = ByteArray(32)
            val n = minOf(32, raw.size)
            System.arraycopy(raw, raw.size - n, out, 32 - n, n)
            return out
        }

        private fun encode(p: Point) = byteArrayOf(4) + fixed(p.x) + fixed(p.y)

        private fun decode(bytes: ByteArray): Point? {
            if (bytes.size != 65 || bytes[0].toInt() != 4) return null
            val x = BigInteger(1, bytes.copyOfRange(1, 33))
            val y = BigInteger(1, bytes.copyOfRange(33, 65))
            if (x >= P || y >= P) return null
            val left = y.multiply(y).mod(P)
            val right = x.multiply(x).multiply(x).add(A.multiply(x)).add(B).mod(P)
            return if (left == right) Point(x, y) else null
        }
    }
}

/** One go at letting the phone in over Wi‑Fi: start, show the code, wait for the click on the Mac. */
class PairClient(private val mac: Nearby) {
    enum class Result { PAIRED, DENIED, BUSY, FAILED, TIMEOUT }

    class Started(val code: String, val session: String, val secret: ByteArray)

    @Volatile var cancelled = false

    /** Sends the phone's key; null with a [Result] when the Mac would not start. */
    fun start(language: String): Pair<Started?, Result?> {
        val key = EcKey.create()
        val hexKey = hex(key.public)
        val reply = try {
            Door.get(mac.host, mac.port, "/pair?k=$hexKey&n=${Door.encode(Door.phoneName())}&l=$language", 3000, 8000)
        } catch (e: IOException) {
            return null to Result.FAILED
        }
        if (reply.code == 429) return null to Result.BUSY
        val json = reply.json
        if (reply.code != 200 || json == null) return null to Result.FAILED
        val macKey = Keys.unhex(json.optString("k")) ?: return null to Result.FAILED
        val session = json.optString("s").takeIf { it.isNotEmpty() && it.all(Char::isLetterOrDigit) } ?: return null to Result.FAILED
        val secret = key.shared(macKey) ?: return null to Result.FAILED
        return Started(code(secret, key.public, macKey), session, secret) to null
    }

    /** Waits for the user of the Mac; on PAIRED, the token is passed to [paired] first. */
    fun finish(started: Started, paired: (String) -> Unit): Result {
        val until = System.currentTimeMillis() + 100_000L
        while (!cancelled && System.currentTimeMillis() < until) {
            val reply = try {
                Door.get(mac.host, mac.port, "/pair/wait?s=${started.session}", 3000, 30_000)
            } catch (e: IOException) {
                return Result.FAILED
            }
            when (reply.code) {
                202 -> continue
                200 -> {
                    val sealed = reply.json?.optString("t")?.let(Keys::unhex) ?: return Result.FAILED
                    val token = Keys.pairing(started.secret).open(sealed)?.trim() ?: return Result.FAILED
                    if (!Servers.validToken(token) || Server.idOf(token) != mac.id) return Result.FAILED
                    paired(token)
                    return Result.PAIRED
                }
                403 -> return Result.DENIED
                else -> return Result.FAILED
            }
        }
        return Result.TIMEOUT
    }

    companion object {
        /** The six digits the Mac shows too: from the shared secret and both public keys. */
        fun code(secret: ByteArray, phone: ByteArray, mac: ByteArray): String {
            val tag = Keys.hmac(secret, "deskstats-pair-code".toByteArray(Charsets.UTF_8) + phone + mac)
            var n = 0L
            for (i in 0 until 4) n = (n shl 8) or (tag[i].toLong() and 0xFF)
            return String.format(Locale.US, "%06d", n % 1_000_000L)
        }

        private fun hex(bytes: ByteArray): String {
            val out = StringBuilder(bytes.size * 2)
            for (b in bytes) {
                val v = b.toInt() and 0xFF
                out.append("0123456789abcdef"[v shr 4]).append("0123456789abcdef"[v and 0xF])
            }
            return out.toString()
        }
    }
}
