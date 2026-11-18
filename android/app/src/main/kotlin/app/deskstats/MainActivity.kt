package app.deskstats

import android.app.Activity
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import org.json.JSONObject
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.util.Locale
import java.util.TimeZone

data class Weather(
    val city: String,
    val temp: Float,
    val feels: Float?,
    val code: Int,
    val isDay: Boolean,
    val max: Float?,
    val min: Float?,
)

data class Battery(val kind: String, val name: String, val pct: Int)

data class Alert(val id: String, val ts: Long, val app: String, val title: String, val body: String) {
    /** The server's id with the time: two notifications with the same text are two notifications. */
    val key: String = if (id.isEmpty()) "$app|$title|$body|$ts" else "$id|$ts"
}

class Stats(
    val ts: Long,
    val intervalMs: Long,
    val cpuName: String,
    val cpuPct: Float,
    val gpuPct: Float?,
    val gpuMem: Long,
    val ramUsed: Long,
    val ramTotal: Long,
    val ramPct: Float,
    val down: Float,
    val up: Float,
    val weather: Weather?,
    val batteries: List<Battery>,
    /** Null when the server does not serve notifications. */
    val alerts: List<Alert>?,
    /** The server serves notifications but macOS does not let it read them. */
    val alertsDenied: Boolean,
) {
    companion object {
        private fun JSONObject.optFloat(name: String): Float? =
            if (isNull(name)) null else optDouble(name).takeIf { !it.isNaN() }?.toFloat()

        private fun parseWeather(o: JSONObject?): Weather? {
            if (o == null) return null
            // A broken weather block must not take the rest of the panel down with it.
            return try {
                Weather(
                    city = o.optString("city"),
                    temp = o.optFloat("temp") ?: return null,
                    feels = o.optFloat("feels"),
                    code = o.optInt("code", -1),
                    isDay = o.optInt("is_day", 1) != 0,
                    max = o.optFloat("max"),
                    min = o.optFloat("min"),
                )
            } catch (e: Exception) {
                null
            }
        }

        fun parse(o: JSONObject): Stats {
            val cpu = o.getJSONObject("cpu")
            val gpu = o.optJSONObject("gpu")
            val ram = o.getJSONObject("ram")
            val net = o.getJSONObject("net")
            val batteries = o.optJSONArray("batteries")?.let { list ->
                (0 until list.length()).mapNotNull { i ->
                    list.optJSONObject(i)?.let { Battery(it.optString("kind"), it.optString("name"), it.optInt("pct")) }
                }
            } ?: emptyList()
            val alerts = o.optJSONArray("alerts")?.let { list ->
                (0 until list.length()).mapNotNull { i ->
                    val a = list.optJSONObject(i) ?: return@mapNotNull null
                    val subtitle = a.optString("subtitle")
                    val title = a.optString("title")
                    val heading = if (subtitle.isEmpty()) title else if (title.isEmpty()) subtitle else "$title · $subtitle"
                    val body = a.optString("body")
                    // A notification without a title shows its text on the title line instead of a gap.
                    Alert(
                        id = a.optString("id"),
                        ts = a.optLong("ts"),
                        app = a.optString("app"),
                        title = if (heading.isEmpty()) body else heading,
                        body = if (heading.isEmpty()) "" else body,
                    )
                }
            }
            return Stats(
                ts = o.optLong("ts"),
                intervalMs = o.optLong("interval_ms", 1000L).coerceIn(200L, 60_000L),
                cpuName = cpu.optString("name"),
                cpuPct = cpu.getDouble("pct").toFloat(),
                gpuPct = gpu?.optFloat("pct"),
                gpuMem = gpu?.optLong("mem_used") ?: 0L,
                ramUsed = ram.getLong("used"),
                ramTotal = ram.getLong("total"),
                ramPct = ram.getDouble("pct").toFloat(),
                down = net.getDouble("down_bps").toFloat(),
                up = net.getDouble("up_bps").toFloat(),
                weather = parseWeather(o.optJSONObject("weather")),
                batteries = batteries,
                alerts = alerts,
                alertsDenied = o.has("alerts") && o.isNull("alerts"),
            )
        }
    }
}

/** What the last poll found, besides data. */
enum class Link { WAITING, ONLINE, STARTING, NO_TOKEN, REFUSED, OFFLINE }

/** How the panel reached the Mac. */
enum class Route { USB, WIFI }

class MainActivity : Activity(), Page.Host {
    private lateinit var panel: PanelView
    private lateinit var clock: ClockView
    private lateinit var root: FrameLayout

    /** The pages opened over the main screen, the top one last. */
    private val pages = ArrayList<Page>()
    /** What the poller adds to its requests for the page on top; read on its thread. */
    @Volatile private var procsSort: String? = null
    @Volatile private var fullAlerts = false
    @Volatile private var placeQuery: String? = null

    override var places = Places(emptyList(), null)
        set(value) {
            field = value
            value.save(getSharedPreferences(Settings.PREFS, Context.MODE_PRIVATE))
            applyPlace()
        }

    override var setup = BlockSetup(Block.values().toList(), emptySet())
        set(value) {
            field = value
            value.save(getSharedPreferences(Settings.PREFS, Context.MODE_PRIVATE))
            panel.setup = value
        }
    @Volatile private var usbPort = Settings.DEFAULT_PORT
    @Volatile private var token = ""
    @Volatile private var keys: Keys? = null
    @Volatile private var wifiHost: String? = null
    @Volatile private var wifiPort = 0
    private var beacon: Beacon? = null
    private var multicastLock: android.net.wifi.WifiManager.MulticastLock? = null

    @Volatile private var poller: Thread? = null
    @Volatile private var connection: HttpURLConnection? = null

    private var shiftStep = 0
    private var offlineSince = 0L
    private var dimmed = false
    /** The Mac said it sleeps, shuts down or turned its display off: the panel is dark until it wakes. */
    @Volatile private var macAsleep = false

    /**
     * Moves the whole picture every two minutes so the AMOLED does not burn in: 5 px at a time,
     * back and forth over a square of ±10 px (the margins of the screen are almost 30 px).
     */
    private val shift = object : Runnable {
        override fun run() {
            shiftStep = (shiftStep + 1) % (SHIFT_SIDE * SHIFT_SIDE)
            val row = shiftStep / SHIFT_SIDE
            val column = shiftStep % SHIFT_SIDE
            val dx = ((if (row % 2 == 0) column else SHIFT_SIDE - 1 - column) - SHIFT_SIDE / 2) * SHIFT_STEP
            val dy = (row - SHIFT_SIDE / 2) * SHIFT_STEP
            root.scrollTo(-dx, -dy)
            root.postDelayed(this, 120_000L)
        }
    }

    /** A page nobody touches closes, so the phone goes back to being a panel. */
    private val idle = Runnable { home() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        showOverLockScreen()
        panel = PanelView(this)
        clock = ClockView(this)
        root = FrameLayout(this)
        // The background belongs to the root and stays put, so the shifted picture leaves no black edge.
        root.setBackgroundColor(PanelView.BACKGROUND)
        root.addView(panel)
        root.addView(clock)
        panel.onLayout = clock::place
        setContentView(root)
        hideSystemBars()

        val prefs = getSharedPreferences(Settings.PREFS, Context.MODE_PRIVATE)
        setup = BlockSetup.load(prefs)
        places = Places.load(prefs)
        panel.onGear = { open(BlocksPage(this, this)) }
        panel.onBlock = { block ->
            when (block) {
                Block.CLOCK, Block.WEATHER -> open(PlacesPage(this, this))
                Block.ALERTS -> open(AlertsPage(this, this).also { page -> panel.latest?.let(page::push) })
                Block.CPU, Block.GPU, Block.RAM, Block.NET -> open(ProcsPage(this, this, block) { watchTopPage() })
                Block.BATTERIES -> {}
            }
        }
    }

    /** The clock, the date and the weather follow the chosen city. */
    private fun applyPlace() {
        val city = places.active
        placeQuery = city?.let { String.format(Locale.US, "place=%.2f,%.2f", it.lat, it.lon) }
        clock.zone = city?.zone ?: TimeZone.getDefault()
        panel.place = city
    }

    override fun open(page: Page) {
        // Only the page on top is drawn and touched.
        pages.lastOrNull()?.visibility = View.GONE
        pages.add(page)
        root.addView(page)
        panel.visibility = View.GONE
        clock.visibility = View.GONE
        watchTopPage()
    }

    override fun back() {
        if (pages.isEmpty()) return
        root.removeView(pages.removeAt(pages.size - 1))
        val top = pages.lastOrNull()
        if (top == null) {
            panel.visibility = View.VISIBLE
            clock.visibility = View.VISIBLE
        } else {
            top.visibility = View.VISIBLE
            top.resumed()
        }
        watchTopPage()
    }

    override fun home() {
        while (pages.isNotEmpty()) back()
    }

    /** Tells the poller what the page on top needs from the Mac. */
    private fun watchTopPage() {
        val top = pages.lastOrNull()
        procsSort = (top as? ProcsPage)?.sort?.id
        fullAlerts = top is AlertsPage
        restartIdle()
    }

    /**
     * One timer for all pages, started again by any touch: three minutes, and ten for the
     * applications, which are watched without touching the screen.
     */
    private fun restartIdle() {
        root.removeCallbacks(idle)
        val top = pages.lastOrNull() ?: return
        root.postDelayed(idle, if (top is ProcsPage) 600_000L else 180_000L)
    }

    override fun dispatchTouchEvent(event: MotionEvent?): Boolean {
        restartIdle()
        return super.dispatchTouchEvent(event)
    }

    /** After a restart of the phone the panel is shown without unlocking it first. */
    private fun showOverLockScreen() {
        if (Build.VERSION.SDK_INT >= 27) {
            setShowWhenLocked(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED)
        }
    }

    /** The back key closes a page. (onBackPressed appeared only in Android 2.0.) */
    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        // On the main screen the back key does nothing: the panel is not meant to be left by accident.
        if (keyCode == KeyEvent.KEYCODE_BACK) {
            back()
            return true
        }
        return super.onKeyDown(keyCode, event)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) hideSystemBars()
    }

    @Suppress("DEPRECATION")
    private fun hideSystemBars() {
        // Before Android 4.4 there is no immersive mode; the fullscreen theme hides the status bar there.
        if (Build.VERSION.SDK_INT < 19) return
        window.decorView.systemUiVisibility = View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY or
            View.SYSTEM_UI_FLAG_FULLSCREEN or
            View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or
            View.SYSTEM_UI_FLAG_LAYOUT_STABLE or
            View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or
            View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
    }

    override fun onStart() {
        super.onStart()
        loadSettings()
        // The phone's time zone may have changed while the panel was away.
        applyPlace()
        // A notice of the Mac's sleep or wake-up that onStop cut short must not leave the panel black.
        macAsleep = false
        root.visibility = View.VISIBLE
        setBrightness(if (dimmed) DIM else WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE)
        root.postDelayed(shift, 120_000L)
        restartIdle()
        poller = Thread(::poll, "poller").also { it.start() }
        listenForBeacon()
    }

    /** Learns the Mac's Wi-Fi address from its signed beacon, also when it changes. */
    private fun listenForBeacon() {
        try {
            // Some Wi-Fi drivers drop broadcast packets unless an app holds this lock.
            val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as android.net.wifi.WifiManager
            multicastLock = wifi.createMulticastLock("deskstats").apply {
                setReferenceCounted(false)
                acquire()
            }
        } catch (e: Exception) {
        }
        beacon = Beacon({ keys }) { host, port ->
            if (host == wifiHost && port == wifiPort) return@Beacon
            wifiHost = host
            wifiPort = port
            getSharedPreferences(Settings.PREFS, Context.MODE_PRIVATE).edit()
                .putString(Settings.WIFI_HOST, host).putInt(Settings.WIFI_PORT, port).commit()
        }.also { it.start() }
    }

    /** ConfigActivity brings an already open panel back with a new address or token. */
    override fun onNewIntent(intent: android.content.Intent?) {
        super.onNewIntent(intent)
        loadSettings()
    }

    /** The poller reads these fields before every request, so a change applies at once. */
    private fun loadSettings() {
        val prefs = getSharedPreferences(Settings.PREFS, Context.MODE_PRIVATE)
        usbPort = prefs.getInt(Settings.USB_PORT, Settings.DEFAULT_PORT)
        token = prefs.getString(Settings.TOKEN, null) ?: ""
        keys = if (token.isEmpty()) null else Keys(token)
        wifiHost = prefs.getString(Settings.WIFI_HOST, null)
        wifiPort = prefs.getInt(Settings.WIFI_PORT, 0)
        panel.address = usbBase()
    }

    override fun onStop() {
        super.onStop()
        root.removeCallbacks(shift)
        root.removeCallbacks(idle)
        val old = poller
        poller = null
        old?.interrupt()
        // interrupt() does not stop a blocking socket read; closing the connection does.
        val open = connection
        if (open != null) Thread { open.disconnect() }.start()
        beacon?.stopListening()
        beacon = null
        try {
            multicastLock?.release()
        } catch (e: Exception) {
        }
        multicastLock = null
    }

    /**
     * Puts the result of a poll on the screen. [code] is the HTTP status of a refusal; [asked] is
     * the place the request was made for, which decides whether its weather may be shown.
     */
    private fun show(link: Link, stats: Stats?, route: Route? = null, code: Int = 0, asked: String? = null) {
        val me = Thread.currentThread()
        panel.post {
            // A poller replaced by onStop/onStart must not touch the panel any more.
            if (poller !== me) return@post
            if (stats != null) {
                panel.push(stats, asked == placeQuery)
                (pages.lastOrNull() as? AlertsPage)?.push(stats)
            }
            if (route != null) panel.route = route
            if (link == Link.REFUSED) panel.refused = code
            panel.link = link
            clock.link = link
            (pages.lastOrNull() as? ProcsPage)?.link = link
            dimWhenAway(link)
        }
    }

    /**
     * Sleeps and wakes with the Mac. Asleep, nothing is drawn: on an AMOLED screen black pixels
     * give no light, so the screen is as good as off, while the app keeps asking the Mac quietly
     * and lights up the moment it is back.
     */
    private fun showAsleep(asleep: Boolean) {
        val me = Thread.currentThread()
        // A poller that onStop/onStart replaced must not change the state of the new one.
        if (poller !== me) return
        macAsleep = asleep
        panel.post {
            if (poller !== me) return@post
            val visibility = if (asleep) View.INVISIBLE else View.VISIBLE
            if (root.visibility == visibility) return@post
            root.visibility = visibility
            // Not zero: some old phones take it as "screen off", and then the polling stops too.
            setBrightness(if (asleep) 0.01f else if (dimmed) DIM else WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE)
        }
    }

    private fun setBrightness(value: Float) {
        val attrs = window.attributes
        if (attrs.screenBrightness == value) return
        attrs.screenBrightness = value
        window.attributes = attrs
    }

    /** After five minutes without the computer the screen is dimmed, saving the AMOLED and the battery. */
    private fun dimWhenAway(link: Link) {
        if (macAsleep) return
        val now = System.currentTimeMillis()
        val away = link == Link.OFFLINE || link == Link.NO_TOKEN || link == Link.REFUSED
        if (!away) offlineSince = 0L else if (offlineSince == 0L) offlineSince = now
        val dim = away && now - offlineSince > 5 * 60_000L
        if (dim == dimmed) return
        dimmed = dim
        setBrightness(if (dim) DIM else WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE)
    }

    /** The phone's own end of the USB tunnel (adb reverse). */
    private fun usbBase() = "http://127.0.0.1:$usbPort"

    /** The applications' load for the open page, asked right after the regular poll. */
    private fun askProcs(route: Route, sort: String) {
        val base = if (route == Route.USB) usbBase() else "http://$wifiHost:$wifiPort"
        val answer = try {
            // The first answer takes the Mac about half a second: it measures over a moment.
            ask(base, "/procs?sort=$sort", 1500)
        } catch (e: IOException) {
            return procsFailed(0)
        }
        // A refusal (an older server, a wrong address) is said on the page instead of waiting for ever.
        if (answer.code != 200) return procsFailed(answer.code)
        val json = answer.json ?: return procsFailed(0)
        // A broken list must not look like a lost connection to the regular poll.
        val o = try {
            JSONObject(json)
        } catch (e: org.json.JSONException) {
            return procsFailed(0)
        }
        val list = o.optJSONArray("procs") ?: return procsFailed(0)
        val procs = (0 until list.length()).mapNotNull { i ->
            val p = list.optJSONObject(i) ?: return@mapNotNull null
            Proc(
                name = p.optString("name"),
                cpu = p.optDouble("cpu", 0.0).toFloat(),
                ram = p.optLong("ram"),
                down = p.optDouble("down", 0.0).toFloat(),
                up = p.optDouble("up", 0.0).toFloat(),
                gpu = p.optDouble("gpu", 0.0).toFloat(),
                count = p.optInt("count", 1),
            )
        }
        val total = o.optInt("total", procs.size)
        val me = Thread.currentThread()
        panel.post {
            if (poller === me) (pages.lastOrNull() as? ProcsPage)?.push(o.optString("sort"), procs, total)
        }
    }

    private fun procsFailed(code: Int) {
        val me = Thread.currentThread()
        panel.post {
            if (poller === me) (pages.lastOrNull() as? ProcsPage)?.fail(code)
        }
    }

    /** One answer from the Mac: its status and, for 200, the JSON text. */
    private class Answer(val code: Int, val json: String?)

    /**
     * Asks the Mac once for [target] ("/stats?…", "/procs?…"). The token is never sent: the
     * request is sealed with keys derived from it, and so is the answer, which must carry the
     * nonce of this very request — an answer recorded earlier is not taken.
     */
    private fun ask(base: String, target: String, connectTimeout: Int): Answer {
        // Without a token there is nothing to ask with; the Mac hands one over USB.
        val k = keys ?: return Answer(401, null)
        val nonce = k.nonce()
        val c = URL("$base/q?" + k.seal("Q\n$nonce\n$target")).openConnection() as HttpURLConnection
        connection = c
        try {
            c.connectTimeout = connectTimeout
            c.readTimeout = 3000
            val code = c.responseCode
            if (code != 200) {
                drain(c)
                return Answer(code, null)
            }
            // Sealed with another token: the same as having none.
            val text = k.open(read(c)) ?: return Answer(401, null)
            val parts = text.split('\n', limit = 4)
            if (parts.size != 4 || parts[0] != "A" || parts[1] != nonce) throw IOException("not the answer to this request")
            val status = parts[2].toIntOrNull() ?: throw IOException("no status")
            return Answer(status, if (status == 200) parts[3] else null)
        } finally {
            if (connection === c) connection = null
        }
    }

    /**
     * USB first (adb reverse on localhost), Wi-Fi when the cable is out. While on Wi-Fi, USB is
     * tried again every 10 seconds, so plugging the cable in switches back to it.
     */
    private fun poll() {
        val me = Thread.currentThread()
        var route = Route.USB
        var usbRetryAt = 0L
        var procsAsked: String? = null
        var procsAt = 0L
        while (poller === me) {
            var wait = 2000L
            try {
                val host = wifiHost
                val asked = placeQuery
                val target = "/stats" + listOfNotNull(asked, if (fullAlerts) "alerts=full" else null).joinToString("&").let { if (it.isEmpty()) "" else "?$it" }
                val routes = ArrayList<Route>(2)
                if (route == Route.USB || System.currentTimeMillis() >= usbRetryAt) routes.add(Route.USB)
                if (host != null && wifiPort > 0) routes.add(Route.WIFI)
                var answer: Answer? = null
                var used = route
                for (r in routes) {
                    try {
                        answer = if (r == Route.USB) ask(usbBase(), target, 700) else ask("http://$host:$wifiPort", target, 1500)
                        used = r
                        break
                    } catch (e: IOException) {
                        // Not reachable this way; try the next one.
                    }
                }
                if (used == Route.WIFI && Route.USB in routes) {
                    usbRetryAt = System.currentTimeMillis() + 10_000L
                }
                route = used
                val mac = if (answer?.code == 200 && answer.json != null) JSONObject(answer.json).optString("mac", "awake") else null
                if (mac != null && mac != "awake") {
                    // Sleeping, shutting down or display off: go dark and check back every 2 seconds.
                    showAsleep(true)
                    Thread.sleep(2000L)
                    continue
                }
                // While the Mac sleeps it does not answer at all; stay dark until it does.
                if (answer == null && macAsleep) {
                    Thread.sleep(2000L)
                    continue
                }
                if (macAsleep) showAsleep(false)
                when {
                    answer == null -> show(Link.OFFLINE, null)
                    answer.code == 200 -> {
                        val stats = Stats.parse(JSONObject(answer.json!!))
                        wait = stats.intervalMs
                        show(Link.ONLINE, stats, used, asked = asked)
                        // The list costs the Mac two processes a time: asked at once for a new
                        // measure, then every few seconds rather than with every poll.
                        val sort = procsSort
                        val now = System.currentTimeMillis()
                        if (sort != null && (sort != procsAsked || now - procsAt >= PROCS_MS)) {
                            procsAt = now
                            askProcs(used, sort)
                        }
                        procsAsked = sort
                    }
                    answer.code == 503 -> {
                        wait = 500L
                        show(Link.STARTING, null, used)
                    }
                    answer.code == 401 -> show(Link.NO_TOKEN, null, used)
                    // The Mac is there but refuses (403, 404, 421…): not a matter of the cable.
                    else -> show(Link.REFUSED, null, used, code = answer.code)
                }
            } catch (e: InterruptedException) {
                return
            } catch (e: Throwable) {
                if (poller !== me) return
                show(Link.OFFLINE, null)
            }
            try {
                Thread.sleep(wait)
            } catch (e: InterruptedException) {
                return
            }
        }
    }

    /** Reads the body, refusing anything larger than a sane response. */
    private fun read(c: HttpURLConnection): ByteArray {
        c.inputStream.use { input ->
            val out = java.io.ByteArrayOutputStream()
            val buf = ByteArray(8192)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                out.write(buf, 0, n)
                if (out.size() > MAX_BODY) throw IOException("response too large")
            }
            return out.toByteArray()
        }
    }

    private fun drain(c: HttpURLConnection) {
        try {
            (c.errorStream ?: c.inputStream)?.use { it.skip(MAX_BODY.toLong()) }
        } catch (e: IOException) {
        }
    }

    private companion object {
        const val MAX_BODY = 256 * 1024
        const val SHIFT_SIDE = 5
        const val SHIFT_STEP = 5
        const val PROCS_MS = 2500L
        const val DIM = 0.02f
    }
}
