package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.Typeface
import android.os.Build
import android.view.MotionEvent
import android.view.View
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sin

class Series {
    val values = FloatArray(HISTORY)
    var count = 0

    fun add(v: Float) {
        if (count < HISTORY) {
            values[count++] = v
        } else {
            System.arraycopy(values, 1, values, 0, HISTORY - 1)
            values[HISTORY - 1] = v
        }
    }

    fun max(): Float {
        var m = 0f
        for (i in 0 until count) if (!values[i].isNaN()) m = max(m, values[i])
        return m
    }

    companion object {
        const val HISTORY = 120
    }
}

class History {
    val cpu = Series()
    val gpu = Series()
    val ram = Series()
    val down = Series()
    val up = Series()
    var last: Stats? = null
    val all get() = arrayOf(cpu, gpu, ram, down, up)
}

class PanelView(context: Context, private val history: History) : View(context) {
    private class CardText(var title: String, var value: String = "—", var unit: String, var note: String = "")

    private class Row(val app: String, val time: String, val first: String, val second: String, val secondIsTitle: Boolean)

    private class Slot(val label: String, val percent: String, val pct: Int)

    private val series = arrayOf(arrayOf(history.cpu), arrayOf(history.gpu), arrayOf(history.ram), arrayOf(history.down, history.up))
    private val cards = arrayOf(
        CardText(context.getString(R.string.cpu).heading(), unit = "%"),
        CardText(context.getString(R.string.gpu).heading(), unit = "%"),
        CardText(context.getString(R.string.ram).heading(), unit = Fmt.gbUnit),
        CardText(context.getString(R.string.network).heading(), unit = ""),
    )
    private var networkMax = ""

    private var stats: Stats? = null
    val latest: Stats? get() = stats
    var address = ""

    var onBlock: ((Block) -> Unit)? = null
    var onGear: (() -> Unit)? = null
    var onLayout: ((Layout) -> Unit)? = null

    var alertsEnabled = true
        set(value) {
            if (field != value) {
                field = value
                invalidate()
            }
        }

    var setup = BlockSetup(Block.values().toList(), emptySet())
        set(value) {
            field = value
            rebuild()
        }

    var place: City? = null
        set(value) {
            val moved = value?.id != field?.id
            field = value
            val zone = value?.zone ?: TimeZone.getDefault()
            dateFormat.timeZone = zone
            stamps.zone = zone
            if (moved) dropWeather()
            dateMinute = -1L
            prepareWeather()
            prepareAlerts()
            prepareHighlight()
            invalidate()
        }

    var route = Route.USB
        set(value) {
            if (field != value) {
                field = value
                describe(true)
                invalidate()
            }
        }

    var link = Link.WAITING
        set(value) {
            if (field != value) {
                field = value
                describe(true)
                invalidate()
            }
        }

    var refused = 0
        set(value) {
            if (field != value) {
                field = value
                refusedLabel = context.getString(R.string.status_refused, value)
                describe(true)
                invalidate()
            }
        }
    private var refusedLabel = ""

    /** How many Macs are paired, which of them is shown and its name: with more than one, the status names it and dots show where it is. */
    var servers = 1
        set(value) {
            if (field != value) {
                field = value
                describe(true)
                invalidate()
            }
        }
    var serverIndex = 0
        set(value) {
            if (field != value) {
                field = value
                invalidate()
            }
        }
    var serverName = ""
        set(value) {
            if (field != value) {
                field = value
                describe(true)
                invalidate()
            }
        }

    private var weather: Weather? = null
    private var weatherMissingSince = 0L
    private var weatherIcon = ""
    private var weatherTemp = ""
    private var weatherFirst = ""
    private var weatherSecond = ""
    private var weatherSize = 0f
    private var weatherLinesSize = 0f
    private var weatherLeft = 0f
    private var weatherTempX = 0f
    private var weatherTextX = 0f
    private var weatherChevronX = 0f

    private val seen = HashSet<String>()
    private var firstData = true
    private var highlight: Alert? = null
    private var highlightUntil = 0L
    private var highlightApp = ""
    private var highlightTime = ""
    private var highlightTitle: List<String> = emptyList()
    private var highlightBody: List<String> = emptyList()

    private val rows = ArrayList<Row>()
    private var rowHeight = 0f
    private var alertsDay = 0L
    private var slots: List<Slot> = emptyList()
    private var slotsHidden = ""

    private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private val text = Paint(Paint.ANTI_ALIAS_FLAG)
    private val regular = Typeface.DEFAULT
    private val bold = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
    private val card = RectF()
    private val line = Path()
    private val area = Path()
    private val mark = Path()

    private var layout: Layout? = null
    private var cardCorners: List<Path> = emptyList()
    private val gradients = HashMap<Long, LinearGradient>()

    private fun gradientKey(r: RectF, color: Int) = (r.top.toLong() shl 32) or (r.bottom.toLong() shl 24) or (color and 0xFFFFFF).toLong()

    private var downX = 0f
    private var downY = 0f
    private var tracking = false
    private var pressed: Any? = null

    private val dateFormat = SimpleDateFormat(context.getString(R.string.fmt_date), Ui.locale)
    private val stamps = Stamps(context)
    private val now = Date()
    private var dateText = ""
    private var dateMinute = -1L
    private var describedAt = 0L

    private val words = Strings(context)

    private class Strings(c: Context) {
        val usb = c.getString(R.string.status_usb)
        val wifi = c.getString(R.string.status_wifi)
        val starting = c.getString(R.string.status_starting)
        val waiting = c.getString(R.string.status_waiting)
        val noToken = c.getString(R.string.status_no_token)
        val offline = c.getString(R.string.status_offline)
        val allHidden = c.getString(R.string.all_hidden)
        val allHiddenHint = c.getString(R.string.all_hidden_hint)
        val weatherLoading = c.getString(R.string.weather_loading)
        val weatherUnavailable = c.getString(R.string.weather_unavailable)
        val noCity = c.getString(R.string.no_city)
        val chooseCity = c.getString(R.string.choose_city_hint)
        val feels = c.getString(R.string.feels)
        val alerts = c.getString(R.string.block_alerts).heading()
        val newAlert = c.getString(R.string.new_alert).heading()
        val alertsStart = c.getString(R.string.alerts_start)
        val alertsDenied = c.getString(R.string.alerts_denied)
        val alertsServerOff = c.getString(R.string.alerts_server_off)
        val alertsOff = c.getString(R.string.alerts_off)
        val alertsNone = c.getString(R.string.alerts_none)
        val mouse = c.getString(R.string.battery_mouse)
        val keyboard = c.getString(R.string.battery_keyboard)
        val trackpad = c.getString(R.string.battery_trackpad)
        val device = c.getString(R.string.battery_device)
        val batteryWaiting = c.getString(R.string.battery_waiting)
        val batteryNone = c.getString(R.string.battery_none)
        val batteries = c.getString(R.string.block_batteries).heading()
        val gpuNone = c.getString(R.string.gpu_none)
        val gpuMem = c.getString(R.string.gpu_mem)
        val netMax = c.getString(R.string.net_max)
        val ram = c.getString(R.string.ram)
        val a11yWaiting = c.getString(R.string.a11y_waiting)
        val a11yStats = c.getString(R.string.a11y_stats)
        val a11yWeather = c.getString(R.string.a11y_weather)
        val a11yAlert = c.getString(R.string.a11y_alert)
        val weatherNames = c.resources.getStringArray(R.array.weather)
    }

    private val minute = object : Runnable {
        override fun run() {
            if (stamps.day(System.currentTimeMillis()) != alertsDay) prepareAlerts()
            invalidate()
            postDelayed(this, 60_000L - System.currentTimeMillis() % 60_000L)
        }
    }

    private val redraw = Runnable { invalidate() }
    private var redrawPending = false
    private val redrawOnTick = Runnable {
        redrawPending = false
        invalidate()
    }

    init {
        contentDescription = words.a11yWaiting
        refusedLabel = context.getString(R.string.status_refused, 0)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        post(minute)
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(minute)
        removeCallbacks(redraw)
        removeCallbacks(redrawOnTick)
        redrawPending = false
        super.onDetachedFromWindow()
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) = rebuild()

    private fun seriesColor(s: Series): Int = when (s) {
        history.cpu -> Ui.p.metrics[0]
        history.gpu -> Ui.p.metrics[1]
        history.ram -> Ui.p.metrics[2]
        history.down -> Ui.p.metrics[3]
        else -> Ui.p.up
    }

    private fun rebuild() {
        if (width == 0 || height == 0) return
        val l = Layout(width.toFloat(), height.toFloat(), setup)
        layout = l
        cardCorners = l.cardRects.map { r ->
            val d = cardUnit(r) * CARD_RADIUS * 2
            Path().apply {
                moveTo(r.left, r.bottom - d / 2)
                lineTo(r.left, r.bottom)
                lineTo(r.left + d / 2, r.bottom)
                arcTo(RectF(r.left, r.bottom - d, r.left + d, r.bottom), 90f, 90f)
                close()
                moveTo(r.right - d / 2, r.bottom)
                lineTo(r.right, r.bottom)
                lineTo(r.right, r.bottom - d / 2)
                arcTo(RectF(r.right - d, r.bottom - d, r.right, r.bottom), 0f, 90f)
                close()
            }
        }
        gradients.clear()
        val alpha = Ui.p.chartAlpha shl 24
        for (r in l.cardRects) {
            for (s in history.all) {
                val c = seriesColor(s)
                val k = gradientKey(r, c)
                if (k in gradients) continue
                gradients[k] = LinearGradient(
                    0f, r.top + cardUnit(r) * CHART_TOP, 0f, r.bottom,
                    (c and 0x00FFFFFF) or alpha, c and 0x00FFFFFF, Shader.TileMode.CLAMP,
                )
            }
        }
        dateMinute = -1L
        prepareWeather()
        prepareAlerts()
        prepareHighlight()
        onLayout?.invoke(l)
        invalidate()
    }

    private fun hitAt(l: Layout, x: Float, y: Float): Any? {
        if (x > l.gear.left - l.barHeight && y < l.gear.bottom + l.gap) return GEAR
        if (l.clock?.contains(x, y) == true) return Block.CLOCK
        if (l.weather?.contains(x, y) == true) return Block.WEATHER
        if (l.alerts?.contains(x, y) == true) return Block.ALERTS
        for (i in l.cardRects.indices) if (l.cardRects[i].contains(x, y)) return l.cards[i]
        return null
    }

    private fun press(target: Any?) {
        if (pressed !== target) {
            pressed = target
            invalidate()
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val l = layout ?: return true
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.x
                downY = event.y
                tracking = true
                press(hitAt(l, event.x, event.y))
            }
            MotionEvent.ACTION_MOVE -> {
                val slop = l.u * 0.05f
                if (tracking && (abs(event.x - downX) > slop || abs(event.y - downY) > slop)) {
                    tracking = false
                    press(null)
                }
            }
            MotionEvent.ACTION_UP -> {
                val target = pressed
                press(null)
                if (!tracking) return true
                tracking = false
                if (target === GEAR) onGear?.invoke() else if (target is Block) onBlock?.invoke(target)
            }
            MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_CANCEL -> {
                tracking = false
                press(null)
            }
        }
        return true
    }

    fun push(st: Stats, weatherIsCurrent: Boolean, sample: Boolean = true) {
        val old = stats
        stats = st
        val sameSample = !sample || (old != null && st.ts != 0L && st.ts == old.ts)
        var changed = old == null || !sameSample
        if (sample && !sameSample) {
            val last = history.last
            if (last != null && st.ts - last.ts > max(3 * st.intervalMs, 5000L)) for (series in history.all) series.add(Float.NaN)
            history.cpu.add(st.cpuPct)
            history.gpu.add(st.gpuPct ?: Float.NaN)
            history.ram.add(st.ramPct)
            history.down.add(st.down)
            history.up.add(st.up)
            history.last = st
        }
        if (old == null || !sameSample) {
            cards[0].apply { value = Fmt.number("%.0f", st.cpuPct); note = st.cpuName }
            cards[1].apply {
                value = st.gpuPct?.let { Fmt.number("%.0f", it) } ?: "—"
                unit = if (st.gpuPct == null) "" else "%"
                note = if (st.gpuPct == null) words.gpuNone else if (st.gpuMem > 0) String.format(words.gpuMem, "${Fmt.gb(st.gpuMem)} ${Fmt.gbUnit}") else ""
            }
            cards[2].apply {
                title = "${words.ram} (${Fmt.number("%.0f", st.ramPct)}%)".heading()
                value = Fmt.gb(st.ramUsed)
                note = "${Fmt.gb(st.ramUsed)} / ${Fmt.gb(st.ramTotal)} ${Fmt.gbUnit}"
            }
            cards[3].apply {
                val d = Fmt.rate(st.down)
                value = d.substringBefore(' ')
                unit = d.substringAfter(' ')
                note = "↑ ${Fmt.rate(st.up)}"
            }
            networkMax = String.format(words.netMax, Fmt.rate(networkScale() / 1.15f))
        }

        if (weatherIsCurrent && st.weather != weather) {
            weather = st.weather
            if (weather == null) dropWeather() else weatherMissingSince = 0L
            prepareWeather()
            changed = true
        } else if (old == null && weather == null) {
            dropWeather()
        }
        if (old == null || st.alerts != old.alerts || st.alertsDenied != old.alertsDenied) {
            prepareAlerts()
            changed = true
        }
        if (old == null || st.batteries != old.batteries) {
            val sorted = st.batteries.sortedBy { it.pct }
            slots = sorted.take(BATTERY_SLOTS).map { b ->
                val label = when (b.kind) {
                    "mouse" -> words.mouse
                    "keyboard" -> words.keyboard
                    "trackpad" -> words.trackpad
                    else -> b.name.ifEmpty { words.device }
                }
                Slot(label, "${b.pct}%", b.pct)
            }
            slotsHidden = if (sorted.size > slots.size) "+${sorted.size - slots.size}" else ""
            changed = true
        }
        if (sample && noticeNewAlert(st)) changed = true
        if (!sample) {
            firstData = false
            for (alert in st.alerts.orEmpty()) seen.add(alert.key)
        }
        describe(false)
        if (changed && !redrawPending) {
            redrawPending = true
            postDelayed(redrawOnTick, 1000L - System.currentTimeMillis() % 1000L)
        }
    }

    private fun dropWeather() {
        weather = null
        weatherMissingSince = System.currentTimeMillis()
        postDelayed(redraw, WEATHER_GRACE_MS + 50)
    }

    private fun noticeNewAlert(st: Stats): Boolean {
        val alerts = st.alerts ?: return false
        var newest: Alert? = null
        for (alert in alerts) {
            if (alert.key in seen || st.ts - alert.ts >= 120_000L) continue
            if (newest == null || alert.ts > newest.ts) newest = alert
        }
        val current = highlight
        val replaced = current != null && alerts.none { it.key == current.key }
        seen.clear()
        for (alert in alerts) seen.add(alert.key)
        if (firstData) {
            firstData = false
            return false
        }
        if (newest == null) return false
        val showing = System.currentTimeMillis() < highlightUntil
        if (current != null && ((replaced && showing) || abs(newest.ts - current.ts) < 5000L) && sameText(current, newest)) {
            highlight = newest
            if (System.currentTimeMillis() >= highlightUntil) return false
            prepareHighlight()
            return true
        }
        highlight = newest
        highlightUntil = System.currentTimeMillis() + HIGHLIGHT_MS
        prepareHighlight()
        postDelayed(redraw, HIGHLIGHT_MS + 50)
        return true
    }

    private fun sameText(a: Alert, b: Alert): Boolean {
        fun norm(x: String) = x.lowercase(Locale.US).split(' ', '·').filter { it.isNotBlank() }.joinToString(" ")
        val na = norm("${a.app} ${a.title} ${a.body}")
        val nb = norm("${b.app} ${b.title} ${b.body}")
        val body = norm(b.body).ifEmpty { norm(b.title) }
        return body.isNotEmpty() && na.contains(body) && nb.contains(norm(a.body).ifEmpty { norm(a.title) })
    }

    private fun statusLabel(): String {
        val label = linkLabel()
        return if (servers > 1 && serverName.isNotEmpty() && label.startsWith(MAC)) serverName + label.substring(MAC.length) else label
    }

    private fun linkLabel(): String = when (link) {
        Link.ONLINE -> if (route == Route.WIFI) words.wifi else words.usb
        Link.STARTING -> words.starting
        Link.WAITING -> words.waiting
        Link.NO_TOKEN -> words.noToken
        Link.REFUSED -> refusedLabel
        Link.OFFLINE -> words.offline
    }

    private fun describe(force: Boolean) {
        val t = System.currentTimeMillis()
        if (!force && t - describedAt < 5000L) return
        describedAt = t
        val st = stats
        val parts = arrayListOf(statusLabel())
        if (st != null) {
            parts.add(String.format(words.a11yStats, Fmt.number("%.0f", st.cpuPct), Fmt.gb(st.ramUsed), Fmt.size(st.ramTotal), Fmt.rate(st.down), Fmt.rate(st.up)))
            val w = weather
            val name = place?.name
            if (w != null && name != null) parts.add(String.format(words.a11yWeather, name, w.temp.roundToInt()))
            st.alerts?.firstOrNull()?.let { parts.add(String.format(words.a11yAlert, it.app, it.title)) }
        }
        contentDescription = parts.joinToString(". ")
    }

    private fun networkScale() = max(max(history.down.max(), history.up.max()), 10_000f) * 1.15f

    private fun prepareWeather() {
        weatherTemp = ""
        val l = layout ?: return
        val block = l.weather ?: return
        val w = weather ?: return
        val (icon, label) = describe(w.code, w.isDay)
        val name = place?.name ?: ""
        val first = if (name.isEmpty()) label else "$label · $name"
        val range = listOfNotNull(
            w.max?.let { "↑${it.roundToInt()}°" },
            w.min?.let { "↓${it.roundToInt()}°" },
        ).joinToString(" ")
        val feels = w.feels?.let { String.format(words.feels, "${it.roundToInt()}°") }
        val second = listOfNotNull(feels, range.ifEmpty { null }).joinToString(" · ")
        val temp = "${w.temp.roundToInt()}°"

        text.typeface = Layout.LIGHT
        text.textSize = 100f
        val tempWidth = text.measureText(temp) / 100f
        text.typeface = regular
        text.textSize = 41f
        val linesWidth = max(text.measureText(first), text.measureText(second)) / 100f
        val iconWidth = if (Build.VERSION.SDK_INT >= 23) {
            text.textSize = 81f
            text.measureText(icon) / 100f + 0.25f
        } else {
            0.81f * 1.3f
        }
        val chevron = 0.3f
        val need = iconWidth + tempWidth + 0.28f + linesWidth + chevron
        val size = max(min(l.weatherSize, block.width() / need), min(l.weatherSize, l.u * 0.085f))
        val room = block.width() - (iconWidth + tempWidth + 0.28f + chevron) * size
        weatherLinesSize = min(size * 0.41f, room * 0.97f * 0.41f / linesWidth)
        text.textSize = weatherLinesSize
        weatherFirst = if (name.isEmpty() || text.measureText(first) <= room) text.fit(first, room) else text.fit(label, room - text.measureText(" · $name")) + " · $name"
        weatherSecond = text.fit(second, room)
        val lines = max(text.measureText(weatherFirst), text.measureText(weatherSecond))
        val total = (iconWidth + tempWidth + 0.28f) * size + lines
        weatherIcon = icon
        weatherTemp = temp
        weatherSize = size
        weatherLeft = if (l.weatherCentred) block.centerX() - (total + chevron * size) / 2 else block.left
        weatherTempX = weatherLeft + iconWidth * size
        weatherTextX = weatherTempX + (tempWidth + 0.28f) * size
        weatherChevronX = weatherTextX + text.measureText(weatherFirst) + size * 0.2f
    }

    private fun prepareAlerts() {
        rows.clear()
        val t = System.currentTimeMillis()
        alertsDay = stamps.day(t)
        val block = layout?.alerts ?: return
        val alerts = stats?.alerts ?: return
        val height = block.height()
        val inset = alertInset(block)
        val header = alertHeader(block)
        val listHeight = block.bottom - inset * 0.4f - (block.top + inset + header * 1.7f)
        val density = resources.displayMetrics.density
        var count = ALERT_ROWS
        if (listHeight / count * 0.22f < 7.5f * density) count = 2
        while (count < 6 && listHeight / (count + 1) * 0.22f >= 12f * density) count++
        rowHeight = listHeight / count
        val width = block.width() - inset * 2
        for (alert in alerts.take(count)) {
            text.typeface = regular
            text.textSize = rowHeight * 0.21f
            val time = stamps.text(alert.ts, t)
            val timeWidth = text.measureText(time)
            text.typeface = bold
            val app = text.fit(alert.app, width - timeWidth - inset)
            text.textSize = rowHeight * 0.235f
            if (alert.body.isEmpty()) {
                val lines = text.wrap(alert.title, width, 2)
                rows.add(Row(app, time, lines.getOrElse(0) { "" }, lines.getOrElse(1) { "" }, true))
            } else {
                val title = text.fit(alert.title, width)
                text.typeface = regular
                text.textSize = rowHeight * 0.22f
                rows.add(Row(app, time, title, text.fit(alert.body, width), false))
            }
        }
        if (height <= 0f) rows.clear()
    }

    private fun alertInset(block: RectF) = min(block.height() * 0.07f, (layout?.u ?: block.height()) * 0.045f)

    private fun alertHeader(block: RectF) = min(block.height() * 0.068f, (layout?.u ?: block.height()) * 0.034f)

    private fun prepareHighlight() {
        highlightTitle = emptyList()
        highlightBody = emptyList()
        val block = layout?.alerts ?: return
        val alert = highlight ?: return
        val unit = highlightUnit(block)
        val inset = alertInset(block)
        val width = block.width() - inset * 2
        highlightTime = stamps.text(alert.ts, System.currentTimeMillis())
        text.typeface = bold
        text.textSize = unit * 0.1f
        highlightApp = text.fit(alert.app, width)
        text.textSize = unit * 0.095f
        highlightTitle = text.wrap(alert.title, width, 2)
        val y = block.top + inset + alertHeader(block) * 1.9f + unit * 0.1f + highlightTitle.size * unit * 0.095f * 1.25f
        text.typeface = regular
        text.textSize = unit * 0.08f
        highlightBody = text.wrap(alert.body, width, ((block.bottom - inset - y) / (text.textSize * 1.3f)).toInt())
    }

    private fun highlightUnit(block: RectF) = min(block.height(), (layout?.u ?: block.height()) * 0.5f)

    override fun onDraw(canvas: Canvas) {
        val l = layout ?: return
        val p = Ui.p
        val t = System.currentTimeMillis()
        val st = stats
        val stale = st != null && link != Link.ONLINE && link != Link.STARTING

        drawStatus(canvas, l)
        drawGear(canvas, l)
        if (l.empty) {
            text.typeface = regular
            text.textAlign = Paint.Align.CENTER
            text.textSize = l.u * 0.05f
            text.color = p.muted
            canvas.drawText(text.fit(words.allHidden, l.w - l.pad * 2), l.w / 2, l.h * 0.5f, text)
            text.textSize = l.u * 0.036f
            text.color = p.dim
            canvas.drawText(text.fit(words.allHiddenHint, l.w - l.pad * 2), l.w / 2, l.h * 0.5f + l.u * 0.07f, text)
            return
        }
        l.clock?.let { block ->
            if (pressed === Block.CLOCK) drawPressed(canvas, l, block)
            text.typeface = regular
            text.textSize = l.clockSize * 0.2f
            text.textAlign = Paint.Align.LEFT
            text.color = if (stale) p.muted else p.soft
            if (t / 60_000L != dateMinute) {
                dateMinute = t / 60_000L
                now.time = t
                dateText = dateFormat.format(now).capitalized()
                val city = if (l.weather == null) place?.name else null
                if (city != null && text.measureText("$dateText · $city") <= block.width() - text.textSize * 1.2f) dateText = "$dateText · $city"
            }
            val width = text.measureText(dateText)
            val x = if (l.clockCentred) block.centerX() - width / 2 - text.textSize * 0.35f else block.left
            canvas.drawText(dateText, x, l.dateBaseline, text)
            drawChevron(canvas, x + width + text.textSize * 0.7f, l.dateBaseline - text.textSize * 0.33f, text.textSize * 0.2f)
        }
        l.weather?.let { block ->
            if (pressed === Block.WEATHER) drawPressed(canvas, l, block)
            if (weather != null && weatherTemp.isNotEmpty()) {
                drawWeather(canvas, block, stale)
            } else if (st != null || place == null) {
                val name = place?.name
                val waiting = st != null && !stale && t - weatherMissingSince < WEATHER_GRACE_MS
                val first = name ?: words.noCity
                val second = if (name == null) words.chooseCity else if (waiting) words.weatherLoading else words.weatherUnavailable
                text.typeface = regular
                text.textAlign = if (l.weatherCentred) Paint.Align.CENTER else Paint.Align.LEFT
                val x = if (l.weatherCentred) block.centerX() else block.left
                text.textSize = min(l.weatherSize * 0.41f, l.u * 0.05f)
                text.color = p.soft
                canvas.drawText(text.fit(first, block.width()), x, block.centerY() - text.textSize * 0.2f, text)
                text.color = if (name == null) p.accent else p.muted
                canvas.drawText(text.fit(second, block.width()), x, block.centerY() + text.textSize * 1.1f, text)
                text.textAlign = Paint.Align.LEFT
            }
        }

        l.alerts?.let { block ->
            if (alertsEnabled && highlight != null && t < highlightUntil) drawHighlight(canvas, block) else drawAlerts(canvas, block, st)
        }
        l.batteries?.let { if (l.batteriesAlone) drawBatteryList(canvas, l, it, st, stale) else drawBatteries(canvas, it, st, stale) }

        for (i in l.cards.indices) {
            val block = l.cards[i]
            card.set(l.cardRects[i])
            val m = METRICS.indexOf(block)
            val c = cards[m]
            val corner = if (block == Block.NET && st != null) networkMax else ""
            drawCard(canvas, i, c, corner, p.metrics[m], if (block == Block.NET) networkScale() else 100f, series[m], stale, pressed === block)
        }
    }

    private fun drawStatus(canvas: Canvas, l: Layout) {
        val p = Ui.p
        val size = l.barHeight * 0.62f
        val baseline = l.pad + l.barHeight * 0.72f
        val bad = link == Link.NO_TOKEN || link == Link.OFFLINE || link == Link.REFUSED
        fill.shader = null
        fill.color = if (link == Link.ONLINE) p.good else if (bad) p.bad else p.muted
        canvas.drawCircle(l.pad + size * 0.4f, baseline - size * 0.33f, size * 0.32f, fill)
        text.typeface = regular
        text.textAlign = Paint.Align.LEFT
        text.textSize = size
        text.color = if (bad) p.bad else p.muted
        var right = l.gear.left - l.gap
        if (servers > 1) {
            val r = size * 0.15f
            val step = r * 3.4f
            val cy = baseline - size * 0.33f
            val first = right - r - step * (servers - 1)
            for (i in 0 until servers) {
                fill.color = if (i == serverIndex) p.text else p.dim
                canvas.drawCircle(first + step * i, cy, if (i == serverIndex) r * 1.2f else r, fill)
            }
            right = first - r - size
        }
        canvas.drawText(text.fit(statusLabel(), right - l.pad - size * 1.15f), l.pad + size * 1.15f, baseline, text)
    }

    private fun drawGear(canvas: Canvas, l: Layout) {
        val p = Ui.p
        val r = l.gear
        val cx = r.centerX()
        val cy = r.centerY()
        val size = r.height()
        fill.shader = null
        if (pressed === GEAR) {
            fill.color = p.pressedFill
            canvas.drawCircle(cx, cy, size * 0.95f, fill)
        }
        stroke.shader = null
        stroke.color = p.muted
        stroke.strokeCap = Paint.Cap.BUTT
        stroke.strokeWidth = size * 0.17f
        for (k in 0 until 8) {
            val a = Math.PI * k / 4
            val c = cos(a).toFloat()
            val sn = sin(a).toFloat()
            canvas.drawLine(cx + c * size * 0.3f, cy + sn * size * 0.3f, cx + c * size * 0.47f, cy + sn * size * 0.47f, stroke)
        }
        stroke.strokeWidth = size * 0.15f
        canvas.drawCircle(cx, cy, size * 0.27f, stroke)
    }

    private fun drawPressed(canvas: Canvas, l: Layout, block: RectF) {
        card.set(block.left - l.pad * 0.4f, block.top, block.right, block.bottom)
        fill.shader = null
        fill.color = Ui.p.pressedFill
        canvas.drawRoundRect(card, l.u * 0.03f, l.u * 0.03f, fill)
    }

    private fun drawChevron(canvas: Canvas, x: Float, y: Float, arm: Float) {
        stroke.shader = null
        stroke.color = Ui.p.chevron
        stroke.strokeWidth = arm * 0.45f
        stroke.strokeCap = Paint.Cap.ROUND
        stroke.strokeJoin = Paint.Join.ROUND
        canvas.chevron(mark, stroke, x, y, arm)
    }

    private fun drawWeather(canvas: Canvas, block: RectF, stale: Boolean) {
        val p = Ui.p
        val w = weather ?: return
        val size = weatherSize
        val baseline = block.centerY() + size * 0.423f
        text.textAlign = Paint.Align.LEFT
        val iconSize = size * 0.81f
        if (Build.VERSION.SDK_INT >= 23) {
            text.typeface = regular
            text.textSize = iconSize
            text.color = faded(0xFF000000.toInt(), stale)
            canvas.drawText(weatherIcon, weatherLeft, baseline - size * 0.06f, text)
        } else {
            drawWeatherIcon(canvas, w.code, w.isDay, weatherLeft + iconSize * 0.6f, baseline - size * 0.375f, iconSize * 0.55f, stale)
        }
        text.typeface = Layout.LIGHT
        text.textSize = size
        text.color = if (stale) p.muted else p.text
        Ui.glow(text, text.color, if (stale) 0f else size * 0.06f)
        canvas.drawText(weatherTemp, weatherTempX, baseline, text)
        Ui.noGlow(text)

        text.typeface = regular
        text.textSize = weatherLinesSize
        text.color = if (stale) p.muted else p.soft
        canvas.drawText(weatherFirst, weatherTextX, baseline - size * 0.625f, text)
        text.color = p.muted
        canvas.drawText(weatherSecond, weatherTextX, baseline - size * 0.09f, text)
        drawChevron(canvas, weatherChevronX + size * 0.08f, baseline - size * 0.625f - size * 0.14f, size * 0.08f)
    }

    private fun drawWeatherIcon(canvas: Canvas, code: Int, day: Boolean, cx: Float, cy: Float, r: Float, stale: Boolean) {
        val p = Ui.p
        fill.shader = null
        stroke.shader = null
        if (code <= 1) {
            fill.color = faded(if (day) p.sun else p.moon, stale)
            canvas.drawCircle(cx, cy, r * 0.55f, fill)
            if (day) {
                stroke.color = fill.color
                stroke.strokeWidth = r * 0.12f
                for (k in 0 until 8) {
                    val a = Math.PI * k / 4
                    val c = cos(a).toFloat()
                    val sn = sin(a).toFloat()
                    canvas.drawLine(cx + c * r * 0.75f, cy + sn * r * 0.75f, cx + c * r, cy + sn * r, stroke)
                }
            }
            return
        }
        fill.color = if (stale) p.staleCloud else if (code in 51..99) p.rainCloud else p.cloud
        canvas.drawCircle(cx - r * 0.35f, cy + r * 0.15f, r * 0.45f, fill)
        canvas.drawCircle(cx + r * 0.2f, cy - r * 0.1f, r * 0.6f, fill)
        canvas.drawCircle(cx + r * 0.7f, cy + r * 0.2f, r * 0.4f, fill)
        if (code in 51..99) {
            stroke.color = faded(p.up, stale)
            stroke.strokeWidth = r * 0.1f
            for (k in -1..1) canvas.drawLine(cx + k * r * 0.4f, cy + r * 0.75f, cx + k * r * 0.4f - r * 0.12f, cy + r * 1.05f, stroke)
        }
    }

    private fun drawBox(canvas: Canvas, block: RectF, radius: Float, border: Int, width: Float) {
        fill.shader = null
        fill.color = Ui.p.card
        canvas.drawRoundRect(block, radius, radius, fill)
        stroke.shader = null
        stroke.strokeWidth = width
        stroke.color = border
        canvas.drawRoundRect(block, radius, radius, stroke)
    }

    private fun drawHeading(canvas: Canvas, label: String, x: Float, baseline: Float, size: Float, color: Int, room: Float) {
        text.typeface = bold
        text.textSize = size
        text.spacing(0.14f)
        text.color = color
        text.textAlign = Paint.Align.LEFT
        canvas.drawText(text.fit(label, room), x, baseline, text)
        text.spacing(0f)
    }

    private fun drawAlerts(canvas: Canvas, block: RectF, st: Stats?) {
        val p = Ui.p
        val left = block.left
        val top = block.top
        val right = block.right
        val height = block.height()
        val inset = alertInset(block)
        val down = pressed === Block.ALERTS
        val radius = min(height * 0.085f, (layout?.u ?: height) * 0.045f)
        drawBox(canvas, block, radius, if (down) p.pressed else p.border, max(1f, height * 0.003f) * (if (down) 2f else 1f))

        val header = alertHeader(block)
        drawHeading(canvas, words.alerts, left + inset, top + inset + header * 0.8f, header, p.muted, right - left - inset * 3)
        drawChevron(canvas, right - inset, top + inset + header * 0.45f, header * 0.3f)

        val alerts = if (alertsEnabled) st?.alerts else null
        val listTop = top + inset + header * 1.7f
        val contentWidth = right - left - inset * 2
        if (alerts.isNullOrEmpty()) {
            text.typeface = regular
            text.textSize = header * 1.1f
            text.color = p.muted
            val note = when {
                st == null -> words.alertsStart
                !alertsEnabled -> words.alertsOff
                st.alertsDenied -> words.alertsDenied
                alerts == null -> words.alertsServerOff
                else -> words.alertsNone
            }
            val lines = text.wrap(note, contentWidth, 3).toMutableList()
            if (st == null && address.isNotEmpty()) lines.add(address)
            for (i in lines.indices) canvas.drawText(lines[i], left + inset, listTop + header * (1.76f + 1.62f * i), text)
            return
        }

        stroke.strokeWidth = max(1f, height * 0.003f)
        for (i in rows.indices) {
            val row = rows[i]
            val rowTop = listTop + rowHeight * i
            if (i > 0) {
                stroke.color = p.grid
                canvas.drawLine(left + inset, rowTop, right - inset, rowTop, stroke)
            }
            text.typeface = regular
            text.textSize = rowHeight * 0.21f
            text.color = p.muted
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(row.time, right - inset, rowTop + rowHeight * 0.3f, text)
            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.color = p.accent
            canvas.drawText(row.app, left + inset, rowTop + rowHeight * 0.3f, text)

            text.textSize = rowHeight * 0.235f
            text.color = p.text
            canvas.drawText(row.first, left + inset, rowTop + rowHeight * 0.6f, text)
            if (!row.secondIsTitle) {
                text.typeface = regular
                text.textSize = rowHeight * 0.22f
                text.color = p.muted
            }
            canvas.drawText(row.second, left + inset, rowTop + rowHeight * 0.88f, text)
        }
    }

    private fun drawHighlight(canvas: Canvas, block: RectF) {
        val p = Ui.p
        val left = block.left
        val top = block.top
        val right = block.right
        val height = block.height()
        val unit = highlightUnit(block)
        val inset = alertInset(block)
        drawBox(canvas, block, min(height * 0.085f, (layout?.u ?: height) * 0.045f), p.accent, max(2f, unit * 0.008f))

        val header = alertHeader(block)
        text.typeface = regular
        text.textSize = header
        val timeWidth = text.measureText(highlightTime)
        drawHeading(canvas, words.newAlert, left + inset, top + inset + header * 0.8f, header, p.accent, right - left - inset * 3 - timeWidth)
        text.typeface = regular
        text.color = p.muted
        text.textAlign = Paint.Align.RIGHT
        canvas.drawText(highlightTime, right - inset, top + inset + header * 0.8f, text)
        text.textAlign = Paint.Align.LEFT

        var y = top + inset + header * 1.9f
        text.typeface = bold
        text.textSize = unit * 0.1f
        text.color = p.accent
        y += text.textSize
        canvas.drawText(highlightApp, left + inset, y, text)

        text.textSize = unit * 0.095f
        text.color = p.text
        for (i in highlightTitle.indices) {
            y += text.textSize * 1.25f
            canvas.drawText(highlightTitle[i], left + inset, y, text)
        }
        text.typeface = regular
        text.textSize = unit * 0.08f
        text.color = p.soft
        for (i in highlightBody.indices) {
            y += text.textSize * 1.3f
            canvas.drawText(highlightBody[i], left + inset, y, text)
        }
    }

    private fun batteryColor(pct: Int, stale: Boolean) = faded(if (pct < 15) Ui.p.bad else if (pct <= 30) Ui.p.warn else Ui.p.good, stale)

    private fun batteryNote(st: Stats?) = if (st == null) words.batteryWaiting else words.batteryNone

    private fun drawBatteries(canvas: Canvas, block: RectF, st: Stats?, stale: Boolean) {
        val p = Ui.p
        val left = block.left
        val top = block.top
        val right = block.right
        val height = block.height()
        val inset = height * 0.24f
        drawBox(canvas, block, height * 0.28f, p.border, max(1f, height * 0.012f))

        text.textAlign = Paint.Align.LEFT
        if (slots.isEmpty()) {
            text.typeface = regular
            text.textSize = height * 0.2f
            text.color = p.muted
            canvas.drawText(text.fit(batteryNote(st), right - left - inset * 2), left + inset, top + height * 0.58f, text)
            return
        }

        val slotGap = height * 0.34f
        val extra = if (slotsHidden.isNotEmpty()) height * 0.6f else 0f
        val slotWidth = (right - left - inset * 2 - extra - slotGap * (slots.size - 1)) / slots.size
        for (i in slots.indices) {
            val slot = slots[i]
            val x = left + inset + (slotWidth + slotGap) * i
            val baseline = top + height * 0.47f
            text.typeface = bold
            text.textSize = height * 0.26f
            text.color = if (stale) p.muted else p.text
            text.textAlign = Paint.Align.RIGHT
            val percentWidth = text.measureText(slot.percent)
            canvas.drawText(slot.percent, x + slotWidth, baseline, text)

            text.typeface = regular
            text.textSize = height * 0.21f
            text.color = p.muted
            text.textAlign = Paint.Align.LEFT
            canvas.drawText(text.fit(slot.label, slotWidth - percentWidth - height * 0.1f), x, baseline, text)
            drawBar(canvas, x, top + height * 0.64f, slotWidth, height * 0.1f, slot.pct, stale)
        }
        if (slotsHidden.isNotEmpty()) {
            text.typeface = bold
            text.textSize = height * 0.22f
            text.color = p.muted
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(slotsHidden, right - inset, top + height * 0.58f, text)
        }
    }

    private fun drawBar(canvas: Canvas, x: Float, top: Float, width: Float, height: Float, pct: Int, stale: Boolean) {
        fill.shader = null
        fill.color = Ui.p.grid
        card.set(x, top, x + width, top + height)
        canvas.drawRoundRect(card, height / 2, height / 2, fill)
        fill.color = batteryColor(pct, stale)
        card.set(x, top, x + max(height, width * pct / 100f), top + height)
        canvas.drawRoundRect(card, height / 2, height / 2, fill)
    }

    private fun drawBatteryList(canvas: Canvas, l: Layout, block: RectF, st: Stats?, stale: Boolean) {
        val p = Ui.p
        val inset = l.u * 0.03f
        val header = l.u * 0.027f
        drawBox(canvas, block, l.u * 0.034f, p.border, max(1f, l.u * 0.0015f))
        drawHeading(canvas, words.batteries, block.left + inset, block.top + inset + header * 0.8f, header, p.muted, block.width() - inset * 3)
        if (slotsHidden.isNotEmpty()) {
            text.typeface = regular
            text.color = p.muted
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(slotsHidden, block.right - inset, block.top + inset + header * 0.8f, text)
            text.textAlign = Paint.Align.LEFT
        }
        val listTop = block.top + inset + header * 1.7f
        val width = block.width() - inset * 2
        if (slots.isEmpty()) {
            text.typeface = regular
            text.textSize = l.u * 0.034f
            text.color = p.muted
            canvas.drawText(text.fit(batteryNote(st), width), block.left + inset, listTop + l.u * 0.06f, text)
            return
        }
        val rowHeight = min((block.bottom - inset * 0.6f - listTop) / slots.size, min(block.height() * 0.38f, l.u * 0.2f))
        for (i in slots.indices) {
            val slot = slots[i]
            val top = listTop + rowHeight * i
            val baseline = top + rowHeight * 0.5f
            text.typeface = bold
            text.textSize = rowHeight * 0.34f
            text.color = if (stale) p.muted else p.text
            text.textAlign = Paint.Align.RIGHT
            val percentWidth = text.measureText(slot.percent)
            canvas.drawText(slot.percent, block.right - inset, baseline, text)
            text.typeface = regular
            text.textSize = rowHeight * 0.26f
            text.color = if (stale) p.muted else p.soft
            text.textAlign = Paint.Align.LEFT
            canvas.drawText(text.fit(slot.label, width - percentWidth - inset), block.left + inset, baseline, text)
            drawBar(canvas, block.left + inset, top + rowHeight * 0.68f, width, rowHeight * 0.085f, slot.pct, stale)
        }
    }

    private fun cardUnit(r: RectF) = min(r.height(), r.width() * 0.72f)

    private fun drawCard(canvas: Canvas, index: Int, c: CardText, corner: String, color: Int, top: Float, series: Array<Series>, stale: Boolean, down: Boolean) {
        val p = Ui.p
        val ch = cardUnit(card)
        val radius = ch * CARD_RADIUS

        fill.shader = null
        fill.color = p.card
        canvas.drawRoundRect(card, radius, radius, fill)

        canvas.save()
        canvas.clipRect(card)
        val chartTop = card.top + ch * CHART_TOP
        val chartBottom = card.bottom - ch * 0.03f
        stroke.shader = null
        stroke.strokeJoin = Paint.Join.ROUND
        stroke.strokeCap = Paint.Cap.ROUND
        val step = card.width() / (Series.HISTORY - 1)
        for (s in series) {
            val sc = seriesColor(s)
            val startX = card.right - step * (s.count - 1)
            var i = 0
            while (i < s.count) {
                while (i < s.count && s.values[i].isNaN()) i++
                if (i >= s.count) break
                val runStart = i
                line.rewind()
                while (i < s.count && !s.values[i].isNaN()) {
                    val x = startX + step * i
                    val y = chartBottom - (chartBottom - chartTop) * (s.values[i] / top).coerceIn(0f, 1f)
                    if (i == runStart) line.moveTo(x, y) else line.lineTo(x, y)
                    i++
                }
                if (i - runStart < 2) continue
                area.set(line)
                area.lineTo(startX + step * (i - 1), card.bottom)
                area.lineTo(startX + step * runStart, card.bottom)
                area.close()
                fill.color = 0xFF000000.toInt()
                fill.alpha = if (stale) STALE_ALPHA else 255
                fill.shader = gradients[gradientKey(card, sc)]
                canvas.drawPath(area, fill)
                fill.shader = null
                if (p.glow && !stale) {
                    stroke.color = (sc and 0x00FFFFFF) or 0x38000000
                    stroke.strokeWidth = ch * 0.05f
                    canvas.drawPath(line, stroke)
                }
                stroke.color = faded(sc, stale)
                stroke.strokeWidth = ch * 0.016f
                canvas.drawPath(line, stroke)
            }
        }
        canvas.restore()
        fill.color = p.background
        cardCorners.getOrNull(index)?.let { canvas.drawPath(it, fill) }

        stroke.strokeWidth = max(1f, ch * 0.008f) * (if (down) 2f else 1f)
        stroke.color = if (down) p.pressed else if (p.glow) (color and 0x00FFFFFF) or 0x66000000 else p.border
        canvas.drawRoundRect(card, radius, radius, stroke)

        val x = card.left + ch * 0.12f
        val right = card.right - ch * 0.1f
        drawChevron(canvas, right, card.top + ch * 0.165f, ch * 0.03f)
        text.typeface = bold
        text.textSize = ch * 0.1f
        text.spacing(0.14f)
        val titleWidth = text.measureText(c.title)
        text.spacing(0f)
        var noteRoom = card.right - ch * 0.08f - x
        if (corner.isNotEmpty()) {
            text.typeface = regular
            text.textSize = ch * 0.085f
            text.color = p.muted
            text.textAlign = Paint.Align.RIGHT
            val cornerWidth = text.measureText(corner)
            text.textSize = ch * 0.12f
            val noteWidth = text.measureText(c.note)
            text.textSize = ch * 0.085f
            if (x + titleWidth + ch * 0.06f + cornerWidth <= right - ch * 0.07f) {
                canvas.drawText(corner, right - ch * 0.07f, card.top + ch * 0.2f, text)
            } else if (noteWidth + cornerWidth + ch * 0.08f <= noteRoom) {
                canvas.drawText(corner, card.right - ch * 0.08f, card.top + ch * 0.62f, text)
                noteRoom -= cornerWidth + ch * 0.05f
            }
        }
        text.typeface = bold
        text.textSize = ch * 0.1f
        text.spacing(0.14f)
        text.color = faded(color, stale)
        text.textAlign = Paint.Align.LEFT
        Ui.glow(text, color, if (stale) 0f else ch * 0.035f)
        canvas.drawText(text.fit(c.title, right - ch * 0.07f - x), x, card.top + ch * 0.2f, text)
        Ui.noGlow(text)
        text.spacing(0f)

        val baseline = card.top + ch * 0.46f
        text.textSize = ch * 0.25f
        text.color = if (stale) p.dim else p.text
        Ui.glow(text, text.color, if (stale) 0f else ch * 0.04f)
        canvas.drawText(c.value, x, baseline, text)
        Ui.noGlow(text)
        val width = text.measureText(c.value)
        text.typeface = regular
        text.textSize = ch * 0.115f
        text.color = p.muted
        canvas.drawText(text.fit(c.unit, card.right - ch * 0.08f - (x + width + ch * 0.03f)), x + width + ch * 0.03f, baseline, text)
        text.textSize = ch * 0.12f
        canvas.drawText(text.fit(c.note, noteRoom), x, card.top + ch * 0.62f, text)
    }

    private fun describe(code: Int, day: Boolean): Pair<String, String> {
        val n = words.weatherNames
        return when (code) {
            0 -> (if (day) "☀️" else "🌙") to n[0]
            1 -> (if (day) "🌤️" else "🌙") to n[1]
            2 -> "⛅" to n[2]
            3 -> "☁️" to n[3]
            45, 48 -> "🌫️" to n[4]
            in 51..57 -> "🌦️" to n[5]
            in 61..67 -> "🌧️" to n[6]
            in 71..77 -> "❄️" to n[7]
            in 80..82 -> "🌧️" to n[8]
            85, 86 -> "🌨️" to n[9]
            in 95..99 -> "⛈️" to n[10]
            else -> "☁️" to n[11]
        }
    }

    companion object {
        private const val MAC = "Mac"
        private const val ALERT_ROWS = 3
        private const val BATTERY_SLOTS = 3
        private const val HIGHLIGHT_MS = 7000L
        private const val WEATHER_GRACE_MS = 20_000L
        private const val CHART_TOP = 0.68f
        private const val CARD_RADIUS = 0.14f
        private const val STALE_ALPHA = 0x73
        private val GEAR = Any()
        private val METRICS = arrayOf(Block.CPU, Block.GPU, Block.RAM, Block.NET)
    }
}
