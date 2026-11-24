package app.deskstats

import android.app.Activity
import android.content.Context
import android.content.res.Configuration
import android.graphics.drawable.ColorDrawable
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
    val temp: Float,
    val feels: Float?,
    val code: Int,
    val isDay: Boolean,
    val max: Float?,
    val min: Float?,
)

data class Battery(val kind: String, val name: String, val pct: Int)

data class Alert(val id: String, val ts: Long, val app: String, val title: String, val body: String) {
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
    val alerts: List<Alert>?,
    val alertsDenied: Boolean,
) {
    companion object {
        private fun JSONObject.optFloat(name: String): Float? =
            if (isNull(name)) null else optDouble(name).takeIf { !it.isNaN() }?.toFloat()

        private fun parseWeather(o: JSONObject?): Weather? {
            if (o == null) return null
            return try {
                Weather(
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

enum class Link { WAITING, ONLINE, STARTING, NO_TOKEN, REFUSED, OFFLINE }

enum class Route { USB, WIFI }

class MainActivity : Activity(), Page.Host {
    private lateinit var panel: PanelView
    private lateinit var clock: ClockView
    private lateinit var root: FrameLayout
    private lateinit var ui: Context
    private val history = History()

    private val pages = ArrayList<Page>()
    @Volatile private var procsSort: String? = null
    @Volatile private var fullAlerts = false
    @Volatile private var placeQuery: String? = null
    private var lastAsked: String? = null

    private var link = Link.WAITING
    private var route = Route.USB
    private var refused = 0

    private val prefs get() = getSharedPreferences(Settings.PREFS, Context.MODE_PRIVATE)

    override var places = Places(emptyList(), null)
        set(value) {
            field = value
            value.save(prefs)
            applyPlace()
        }

    override var setup = BlockSetup(Block.values().toList(), emptySet())
        set(value) {
            field = value
            value.save(prefs)
            panel.setup = value
        }

    override var options = Options()
        set(value) {
            val old = field
            field = value
            intervalMs = value.intervalMs.toLong()
            configQuery = value.query
            requestedOrientation = value.requestedOrientation
            if (!built) return
            value.save(prefs)
            if (value.language != old.language || value.theme != old.theme) build(reopenSettings = true) else panel.alertsEnabled = value.alerts
        }

    @Volatile private var intervalMs = 1000L
    @Volatile private var configQuery = Options().query
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
    @Volatile private var macAsleep = false
    private var builtLanguage = ""

    // Moves the picture by a few pixels every two minutes so the AMOLED does not burn in.
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

    private val idle = Runnable { home() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        showOverLockScreen()
        root = FrameLayout(this)
        setContentView(root)
        hideSystemBars()

        val prefs = prefs
        options = Options.load(prefs)
        build(reopenSettings = false)
        setup = BlockSetup.load(prefs)
        places = Places.load(prefs)
    }

    private fun setupLocale(o: Options) {
        ui = Ui.localize(this, o.language)
        builtLanguage = Ui.language
        Ui.p = Palette.of(o.theme)
        Fmt.init(ui)
        Places.localize(ui)
    }

    private var built = false

    private fun build(reopenSettings: Boolean) {
        setupLocale(options)
        pages.clear()
        root.removeAllViews()
        root.setBackgroundColor(Ui.p.background)
        if (root.visibility == View.VISIBLE) window.setBackgroundDrawable(ColorDrawable(Ui.p.background))
        panel = PanelView(ui, history)
        clock = ClockView(ui)
        root.addView(panel)
        root.addView(clock)
        panel.onLayout = clock::place
        panel.onGear = { open(SettingsPage(ui, this)) }
        panel.onBlock = { block ->
            when (block) {
                Block.CLOCK, Block.WEATHER -> open(PlacesPage(ui, this))
                Block.ALERTS -> open(AlertsPage(ui, this).also { page -> panel.latest?.let(page::push) })
                Block.CPU, Block.GPU, Block.RAM, Block.NET -> open(ProcsPage(ui, this, block) { watchTopPage() })
                Block.BATTERIES -> {}
            }
        }
        panel.alertsEnabled = options.alerts
        if (built) {
            panel.setup = setup
            applyPlace()
            panel.address = usbBase()
            panel.route = route
            if (refused != 0) panel.refused = refused
            panel.link = link
            clock.link = link
            history.last?.let { panel.push(it, lastAsked != null && lastAsked == placeQuery, sample = false) }
        }
        built = true
        if (reopenSettings) open(SettingsPage(ui, this))
        watchTopPage()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (options.language.isEmpty() && Ui.systemLanguage() != builtLanguage) {
            val top = pages.lastOrNull()
            build(reopenSettings = top is SettingsPage || top is LanguagePage)
        } else if (Build.VERSION.SDK_INT < 17) {
            Ui.localize(this, options.language)
        }
    }

    private fun applyPlace() {
        val city = places.active
        placeQuery = city?.let { String.format(Locale.US, "place=%.2f,%.2f", it.lat, it.lon) }
        clock.zone = city?.zone ?: TimeZone.getDefault()
        panel.place = city
    }

    override fun open(page: Page) {
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

    private fun watchTopPage() {
        val top = pages.lastOrNull()
        procsSort = (top as? ProcsPage)?.sort?.id
        fullAlerts = top is AlertsPage
        restartIdle()
    }

    private fun restartIdle() {
        root.removeCallbacks(idle)
        val top = pages.lastOrNull() ?: return
        root.postDelayed(idle, if (top is ProcsPage) 600_000L else 180_000L)
    }

    override fun dispatchTouchEvent(event: MotionEvent?): Boolean {
        restartIdle()
        return super.dispatchTouchEvent(event)
    }

    private fun showOverLockScreen() {
        if (Build.VERSION.SDK_INT >= 27) {
            setShowWhenLocked(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED)
        }
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
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
        applyPlace()
        macAsleep = false
        root.visibility = View.VISIBLE
        window.setBackgroundDrawable(ColorDrawable(Ui.p.background))
        setBrightness(if (dimmed) DIM else WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE)
        root.postDelayed(shift, 120_000L)
        restartIdle()
        poller = Thread(::poll, "poller").also { it.start() }
        listenForBeacon()
    }

    private fun listenForBeacon() {
        try {
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
            prefs.edit().putString(Settings.WIFI_HOST, host).putInt(Settings.WIFI_PORT, port).commit()
        }.also { it.start() }
    }

    override fun onNewIntent(intent: android.content.Intent?) {
        super.onNewIntent(intent)
        loadSettings()
    }

    private fun loadSettings() {
        val prefs = prefs
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

    private fun show(link: Link, stats: Stats?, route: Route? = null, code: Int = 0, asked: String? = null) {
        val me = Thread.currentThread()
        panel.post {
            if (poller !== me) return@post
            if (stats != null) {
                lastAsked = asked
                panel.push(stats, asked != null && asked == placeQuery)
                (pages.lastOrNull() as? AlertsPage)?.push(stats)
            }
            if (route != null) {
                this.route = route
                panel.route = route
            }
            if (link == Link.REFUSED) {
                refused = code
                panel.refused = code
            }
            this.link = link
            panel.link = link
            clock.link = link
            (pages.lastOrNull() as? ProcsPage)?.link = link
            dimWhenAway(link)
        }
    }

    private fun showAsleep(asleep: Boolean) {
        val me = Thread.currentThread()
        if (poller !== me) return
        macAsleep = asleep
        panel.post {
            if (poller !== me) return@post
            val visibility = if (asleep) View.INVISIBLE else View.VISIBLE
            if (root.visibility == visibility) return@post
            root.visibility = visibility
            window.setBackgroundDrawable(ColorDrawable(if (asleep) 0xFF000000.toInt() else Ui.p.background))
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

    private fun usbBase() = "http://127.0.0.1:$usbPort"

    private fun base(route: Route) = if (route == Route.USB) usbBase() else "http://$wifiHost:$wifiPort"

    private fun askProcs(route: Route, sort: String) {
        val answer = try {
            ask(base(route), "/procs?sort=$sort", 1500)
        } catch (e: IOException) {
            return procsFailed(0)
        }
        if (answer.code != 200) return procsFailed(answer.code)
        val json = answer.json ?: return procsFailed(0)
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

    private fun sendConfig(route: Route, query: String): Boolean = try {
        ask(base(route), query, 1500).code == 200
    } catch (e: IOException) {
        false
    }

    private class Answer(val code: Int, val json: String?)

    private fun ask(base: String, target: String, connectTimeout: Int): Answer {
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
            val text = k.open(read(c)) ?: return Answer(401, null)
            val parts = text.split('\n', limit = 4)
            if (parts.size != 4 || parts[0] != "A" || parts[1] != nonce) throw IOException("not the answer to this request")
            val status = parts[2].toIntOrNull() ?: throw IOException("no status")
            return Answer(status, if (status == 200) parts[3] else null)
        } finally {
            if (connection === c) connection = null
        }
    }

    private fun poll() {
        val me = Thread.currentThread()
        var route = Route.USB
        var usbRetryAt = 0L
        var procsAsked: String? = null
        var procsAt = 0L
        var configSent: String? = null
        var configAt = 0L
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
                    }
                }
                if (used == Route.WIFI && Route.USB in routes) {
                    usbRetryAt = System.currentTimeMillis() + 10_000L
                }
                route = used
                val mac = if (answer?.code == 200 && answer.json != null) JSONObject(answer.json).optString("mac", "awake") else null
                if (mac != null && mac != "awake") {
                    showAsleep(true)
                    Thread.sleep(2000L)
                    continue
                }
                if (answer == null && macAsleep) {
                    Thread.sleep(2000L)
                    continue
                }
                if (macAsleep) showAsleep(false)
                when {
                    answer == null -> show(Link.OFFLINE, null)
                    answer.code == 200 -> {
                        val stats = Stats.parse(JSONObject(answer.json!!))
                        wait = intervalMs
                        show(Link.ONLINE, stats, used, asked = asked)
                        val query = configQuery
                        val now = System.currentTimeMillis()
                        // A restarted or reinstalled Mac server may have lost the settings.
                        val forgotten = stats.intervalMs != intervalMs
                        if ((query != configSent || forgotten) && now - configAt >= 10_000L) {
                            configAt = now
                            configSent = if (sendConfig(used, query)) query else null
                        }
                        val sort = procsSort
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
