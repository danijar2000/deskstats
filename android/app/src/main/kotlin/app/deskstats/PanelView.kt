package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
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

/**
 * Everything but the clock: connection state, date, weather, the computer's notifications, the
 * battery of its mouse and keyboard, and a row of CPU, GPU, RAM and network cards with charts.
 * It redraws when what it shows changes (and once a minute for the date), not every second;
 * the texts are made ready when data arrives, so a redraw only draws.
 *
 * The blocks can be tapped: a load card opens the list of applications sorted by that load, the
 * clock and the weather open the choice of the city, the notifications open their full list.
 * The gear in the corner opens the list of blocks. Which blocks are shown is in [setup].
 */
class PanelView(context: Context) : View(context) {
    private class Series(val color: Int) {
        val values = FloatArray(HISTORY)
        var count = 0

        /** NaN marks a gap (no value), which the chart leaves empty. */
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
    }

    /** Ready-made texts of one card. */
    private class CardText(var title: String, var value: String = "—", var unit: String, var note: String = "")

    /** One notification of the list, already cut to the width of its block. */
    private class Row(val app: String, val time: String, val first: String, val second: String, val secondIsTitle: Boolean)

    private class Slot(val label: String, val percent: String, val pct: Int)

    private val cpu = Series(TEAL)
    private val gpu = Series(GREEN)
    private val ram = Series(PURPLE)
    private val down = Series(ORANGE)
    private val up = Series(BLUE)
    // Indexed like METRICS: CPU, GPU, RAM, network.
    private val series = arrayOf(arrayOf(cpu), arrayOf(gpu), arrayOf(ram), arrayOf(down, up))
    private val cards = arrayOf(CardText("CPU", unit = "%"), CardText("GPU", unit = "%"), CardText("RAM", unit = "ГБ"), CardText("СЕТЬ", unit = "КБ/с"))
    private var networkMax = ""

    private var stats: Stats? = null
    /** The last data shown, for a page that opens between two polls. */
    val latest: Stats? get() = stats
    var address = ""

    var onBlock: ((Block) -> Unit)? = null
    var onGear: (() -> Unit)? = null
    /** Tells the clock, a view of its own, where its digits go. */
    var onLayout: ((Layout) -> Unit)? = null

    var setup = BlockSetup(Block.values().toList(), emptySet())
        set(value) {
            field = value
            rebuild()
        }

    /** The chosen city, whose time zone the date and the notifications follow and whose weather is shown. */
    var place: City? = null
        set(value) {
            val moved = value?.id != field?.id
            field = value
            val zone = value?.zone ?: TimeZone.getDefault()
            dateFormat.timeZone = zone
            stamps.zone = zone
            // The weather on the screen belongs to the previous place: gone until the Mac answers for this one.
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

    /** The HTTP status the Mac refused with, shown while [link] is REFUSED. */
    var refused = 0
        set(value) {
            if (field != value) {
                field = value
                refusedLabel = "Mac · сервер отказал (код $value)"
                describe(true)
                invalidate()
            }
        }
    private var refusedLabel = "Mac · сервер отказал"

    // The weather of the current place; null until the Mac answers for it.
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

    // A new notification fills the notifications card for a few seconds.
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
    /** What lies outside the rounded bottom corners of each card: painted over the chart instead of clipping it. */
    private var cardCorners: List<Path> = emptyList()
    private val gradients = HashMap<Int, LinearGradient>()

    private var downX = 0f
    private var downY = 0f
    private var tracking = false
    /** What the finger is on: a [Block], or [GEAR]. */
    private var pressed: Any? = null

    private val dateFormat = SimpleDateFormat("EEEE, d MMMM", Locale("ru"))
    private val stamps = Stamps()
    private val now = Date()
    private var dateText = ""
    private var dateMinute = -1L
    private var describedAt = 0L

    private val minute = object : Runnable {
        override fun run() {
            // The day may have changed: "14:05" becomes "вчера 14:05".
            if (stamps.day(System.currentTimeMillis()) != alertsDay) prepareAlerts()
            invalidate()
            postDelayed(this, 60_000L - System.currentTimeMillis() % 60_000L)
        }
    }

    // Posted for the moments the picture changes by itself: a highlight ends, the weather is given up on.
    private val redraw = Runnable { invalidate() }
    private var redrawPending = false
    private val redrawOnTick = Runnable {
        redrawPending = false
        invalidate()
    }

    init {
        contentDescription = "Панель компьютера: ожидание данных"
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
        val row = l.cardRects.firstOrNull()
        if (row != null) {
            for (c in intArrayOf(TEAL, GREEN, PURPLE, ORANGE, BLUE)) {
                gradients[c] = LinearGradient(
                    0f, row.top + cardUnit(row) * CHART_TOP, 0f, row.bottom,
                    (c and 0x00FFFFFF) or 0x40000000, c and 0x00FFFFFF, Shader.TileMode.CLAMP,
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

    /** The block (or the gear) under a touch; null where a tap does nothing. */
    private fun hitAt(l: Layout, x: Float, y: Float): Any? {
        // The gear is small; a finger gets a generous area around it.
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

    /** A tap on a block or on the gear; the clock view above lets touches through. */
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
                val slop = l.h * 0.05f
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
            // A second finger or a cancelled touch is not a tap.
            MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_CANCEL -> {
                tracking = false
                press(null)
            }
        }
        return true
    }

    /**
     * Takes the Mac's answer. [weatherIsCurrent] is false for an answer that was asked for another
     * place than the chosen one: its weather is not shown.
     */
    fun push(s: Stats, weatherIsCurrent: Boolean) {
        val old = stats
        stats = s
        // The same measurement again (the Mac measures less often than the panel asks) adds no point.
        val sameSample = old != null && s.ts != 0L && s.ts == old.ts
        var changed = old == null || !sameSample
        if (!sameSample) {
            // A break in the link (sleep, lost cable) is a gap in the charts, not a straight line.
            if (old != null && s.ts - old.ts > max(3 * s.intervalMs, 5000L)) for (series in arrayOf(cpu, gpu, ram, down, up)) series.add(Float.NaN)
            cpu.add(s.cpuPct)
            gpu.add(s.gpuPct ?: Float.NaN)
            ram.add(s.ramPct)
            down.add(s.down)
            up.add(s.up)

            cards[0].apply { value = Fmt.number("%.0f", s.cpuPct); note = s.cpuName }
            cards[1].apply {
                value = s.gpuPct?.let { Fmt.number("%.0f", it) } ?: "—"
                unit = if (s.gpuPct == null) "" else "%"
                note = if (s.gpuPct == null) "нет данных о видеоядре" else if (s.gpuMem > 0) "видеопамять ${Fmt.gb(s.gpuMem)} ГБ" else ""
            }
            cards[2].apply { title = Fmt.number("RAM (%.0f%%)", s.ramPct); value = Fmt.gb(s.ramUsed); note = "${Fmt.gb(s.ramUsed)} / ${Fmt.gb(s.ramTotal)} ГБ" }
            cards[3].apply {
                val d = Fmt.rate(s.down)
                value = d.substringBefore(' ')
                unit = d.substringAfter(' ')
                note = "↑ ${Fmt.rate(s.up)}"
            }
            networkMax = "макс ${Fmt.rate(networkScale() / 1.15f)}"
        }

        if (weatherIsCurrent && s.weather != weather) {
            weather = s.weather
            if (weather == null) dropWeather() else weatherMissingSince = 0L
            prepareWeather()
            changed = true
        } else if (old == null && weather == null) {
            // The first answer came without weather: it gets the same time to load as after a change of place.
            dropWeather()
        }
        if (old == null || s.alerts != old.alerts || s.alertsDenied != old.alertsDenied) {
            prepareAlerts()
            changed = true
        }
        if (old == null || s.batteries != old.batteries) {
            val sorted = s.batteries.sortedBy { it.pct }
            slots = sorted.take(BATTERY_SLOTS).map { b ->
                val label = when (b.kind) {
                    "mouse" -> "Мышь"
                    "keyboard" -> "Клавиатура"
                    "trackpad" -> "Трекпад"
                    else -> b.name.ifEmpty { "Устройство" }
                }
                Slot(label, "${b.pct}%", b.pct)
            }
            slotsHidden = if (sorted.size > slots.size) "+${sorted.size - slots.size}" else ""
            changed = true
        }
        if (noticeNewAlert(s)) changed = true
        describe(false)
        // New numbers are drawn on the next whole second, in the frame the clock redraws its seconds
        // in anyway: one frame a second instead of two.
        if (changed && !redrawPending) {
            redrawPending = true
            postDelayed(redrawOnTick, 1000L - System.currentTimeMillis() % 1000L)
        }
    }

    /** Forgets the weather shown and, after a while, says it is unavailable rather than loading. */
    private fun dropWeather() {
        weather = null
        weatherMissingSince = System.currentTimeMillis()
        postDelayed(redraw, WEATHER_GRACE_MS + 50)
    }

    /** The newest notification not seen before becomes the highlighted one (not on the first load). */
    private fun noticeNewAlert(s: Stats): Boolean {
        val alerts = s.alerts ?: return false
        var newest: Alert? = null
        for (alert in alerts) {
            // Only a notification that came in the last two minutes (server clock) is news.
            if (alert.key in seen || s.ts - alert.ts >= 120_000L) continue
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
        // The same notification often shows up twice: first read from the screen, a moment later
        // from the database with the app name, in place of the first or right next to it. That is
        // an update, not news; a real repeat comes later and stays in the list with the earlier one.
        // Once its big showing is over, the earlier one is history: the same text again is news.
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

    private fun statusLabel(): String = when (link) {
        Link.ONLINE -> if (route == Route.WIFI) "Mac · на связи по Wi-Fi" else "Mac · на связи по USB"
        Link.STARTING -> "Mac · запуск"
        Link.WAITING -> "Mac · ожидание данных"
        Link.NO_TOKEN -> "Mac · нужен токен: подключите телефон по USB"
        Link.REFUSED -> refusedLabel
        Link.OFFLINE -> "Mac · нет связи"
    }

    /** A spoken summary for TalkBack, refreshed every few seconds and when the link changes. */
    private fun describe(force: Boolean) {
        val t = System.currentTimeMillis()
        if (!force && t - describedAt < 5000L) return
        describedAt = t
        val s = stats
        contentDescription = statusLabel() + if (s == null) "" else
            ". Процессор ${Fmt.number("%.0f", s.cpuPct)} процентов, память ${Fmt.gb(s.ramUsed)} из ${Fmt.gb(s.ramTotal)} гигабайт, " +
                "загрузка ${Fmt.rate(s.down)}, отдача ${Fmt.rate(s.up)}" +
                (weather?.let { ", ${place?.name ?: Places.named(it.city)} ${it.temp.roundToInt()} градусов" } ?: "") +
                (s.alerts?.firstOrNull()?.let { ". Последнее уведомление: ${it.app}, ${it.title}" } ?: "")
    }

    private fun networkScale() = max(max(down.max(), up.max()), 10_000f) * 1.15f

    /** The lines of the weather block, sized so that they fit its width as well as its height. */
    private fun prepareWeather() {
        weatherTemp = ""
        val l = layout ?: return
        val block = l.weather ?: return
        val w = weather ?: return
        val (icon, label) = describe(w.code, w.isDay)
        val name = place?.name ?: Places.named(w.city)
        val first = if (name.isEmpty()) label else "$label · $name"
        val range = listOfNotNull(
            w.max?.let { "↑${it.roundToInt()}°" },
            w.min?.let { "↓${it.roundToInt()}°" },
        ).joinToString(" ")
        // The felt temperature goes first and short, so a narrow line cuts the range, not it.
        val feels = w.feels?.let { "ощущ. ${it.roundToInt()}°" }
        val second = listOfNotNull(feels, range.ifEmpty { null }).joinToString(" · ")
        val temp = "${w.temp.roundToInt()}°"

        // Everything scales with one size; measured at 100 px, the widths are a matter of proportion.
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
        // A larger block must not cut the lines short: the weather shrinks to its width instead,
        // though never below what is readable from a desk; only then are the lines cut.
        val size = max(min(l.weatherSize, block.width() / need), min(l.weatherSize, l.h * 0.085f))
        val room = block.width() - (iconWidth + tempWidth + 0.28f + chevron) * size
        // Only the two lines shrink further, with a margin for rounding; then the description is
        // cut before the city's name.
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

    /** The rows of the notifications block: how many fit readably, and their texts cut to its width. */
    private fun prepareAlerts() {
        rows.clear()
        val t = System.currentTimeMillis()
        alertsDay = stamps.day(t)
        val block = layout?.alerts ?: return
        val alerts = stats?.alerts ?: return
        val height = block.height()
        val inset = height * 0.07f
        val listHeight = block.bottom - inset * 0.4f - (block.top + inset + height * 0.068f * 1.7f)
        // Three rows; two when their text would be too small to read from across a desk, more on a tall block.
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
                // Without a body the title has two lines, so its end (a sum, a name) is not lost.
                val lines = text.wrap(alert.title, width, 2)
                rows.add(Row(app, time, lines.getOrElse(0) { "" }, lines.getOrElse(1) { "" }, true))
            } else {
                val title = text.fit(alert.title, width)
                text.typeface = regular
                text.textSize = rowHeight * 0.22f
                rows.add(Row(app, time, title, text.fit(alert.body, width), false))
            }
        }
    }

    private fun prepareHighlight() {
        highlightTitle = emptyList()
        highlightBody = emptyList()
        val block = layout?.alerts ?: return
        val alert = highlight ?: return
        val height = block.height()
        val inset = height * 0.07f
        val width = block.width() - inset * 2
        highlightTime = stamps.text(alert.ts, System.currentTimeMillis())
        text.typeface = bold
        text.textSize = height * 0.1f
        highlightApp = text.fit(alert.app, width)
        text.textSize = height * 0.095f
        highlightTitle = text.wrap(alert.title, width, 2)
        val y = block.top + inset + height * 0.068f * 1.9f + height * 0.1f + highlightTitle.size * height * 0.095f * 1.25f
        text.typeface = regular
        text.textSize = height * 0.08f
        highlightBody = text.wrap(alert.body, width, ((block.bottom - inset - y) / (text.textSize * 1.3f)).toInt())
    }

    override fun onDraw(canvas: Canvas) {
        val l = layout ?: return
        val t = System.currentTimeMillis()
        val s = stats
        val stale = s != null && link != Link.ONLINE && link != Link.STARTING

        drawStatus(canvas, l)
        drawGear(canvas, l)
        if (l.empty) {
            text.typeface = regular
            text.textAlign = Paint.Align.CENTER
            text.textSize = l.h * 0.05f
            text.color = MUTED
            canvas.drawText("Все блоки скрыты", l.w / 2, l.h * 0.5f, text)
            text.textSize = l.h * 0.036f
            text.color = DIM_WHITE
            canvas.drawText("Нажмите на шестерёнку в углу, чтобы вернуть их", l.w / 2, l.h * 0.5f + l.h * 0.07f, text)
            return
        }
        l.clock?.let { block ->
            if (pressed === Block.CLOCK) drawPressed(canvas, l, block)
            text.typeface = regular
            text.textSize = l.clockSize * 0.2f
            text.textAlign = Paint.Align.LEFT
            text.color = if (stale) MUTED else SOFT_WHITE
            if (t / 60_000L != dateMinute) {
                dateMinute = t / 60_000L
                now.time = t
                dateText = dateFormat.format(now)
                // With the weather hidden, the date line is where the chosen city is named, if it has the room.
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
            } else if (s != null) {
                // Why there is no weather: still being fetched, not available, or no city to fetch it for.
                val name = place?.name
                val waiting = !stale && t - weatherMissingSince < WEATHER_GRACE_MS
                val first = name ?: if (waiting) "Погода загружается…" else "Город не задан"
                val second = if (name == null) (if (waiting) "" else "Нажмите, чтобы выбрать город") else if (waiting) "Погода загружается…" else "Погода недоступна"
                text.typeface = regular
                text.textAlign = if (l.weatherCentred) Paint.Align.CENTER else Paint.Align.LEFT
                val x = if (l.weatherCentred) block.centerX() else block.left
                text.textSize = l.weatherSize * 0.41f
                text.color = SOFT_WHITE
                canvas.drawText(text.fit(first, block.width()), x, block.centerY() - (if (second.isEmpty()) -text.textSize * 0.35f else text.textSize * 0.2f), text)
                text.color = MUTED
                if (second.isNotEmpty()) canvas.drawText(text.fit(second, block.width()), x, block.centerY() + text.textSize * 1.1f, text)
            }
        }

        l.alerts?.let { block ->
            if (highlight != null && t < highlightUntil) drawHighlight(canvas, block) else drawAlerts(canvas, block, s)
        }
        l.batteries?.let { if (l.batteriesAlone) drawBatteryList(canvas, l, it, s, stale) else drawBatteries(canvas, it, s, stale) }

        for (i in l.cards.indices) {
            val block = l.cards[i]
            card.set(l.cardRects[i])
            val m = METRICS.indexOf(block)
            val c = cards[m]
            val corner = if (block == Block.NET && s != null) networkMax else ""
            drawCard(canvas, i, c, corner, COLORS[m], if (block == Block.NET) networkScale() else 100f, series[m], stale, pressed === block)
        }
    }

    private fun drawStatus(canvas: Canvas, l: Layout) {
        val size = l.barHeight * 0.62f
        val baseline = l.pad + l.barHeight * 0.72f
        val bad = link == Link.NO_TOKEN || link == Link.OFFLINE || link == Link.REFUSED
        fill.shader = null
        fill.color = if (link == Link.ONLINE) GOOD else if (bad) BAD else MUTED
        canvas.drawCircle(l.pad + size * 0.4f, baseline - size * 0.33f, size * 0.32f, fill)
        text.typeface = regular
        text.textAlign = Paint.Align.LEFT
        text.textSize = size
        text.color = if (bad) BAD else MUTED
        canvas.drawText(text.fit(statusLabel(), l.gear.left - l.gap - l.pad - size * 1.15f), l.pad + size * 1.15f, baseline, text)
    }

    /** The settings gear: a ring with eight teeth. */
    private fun drawGear(canvas: Canvas, l: Layout) {
        val r = l.gear
        val cx = r.centerX()
        val cy = r.centerY()
        val size = r.height()
        fill.shader = null
        if (pressed === GEAR) {
            fill.color = PRESSED_FILL
            canvas.drawCircle(cx, cy, size * 0.95f, fill)
        }
        stroke.shader = null
        stroke.color = MUTED
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

    /** The answer to a finger on a block without a frame of its own. */
    private fun drawPressed(canvas: Canvas, l: Layout, block: RectF) {
        card.set(block.left - l.pad * 0.4f, block.top, block.right, block.bottom)
        fill.shader = null
        fill.color = PRESSED_FILL
        canvas.drawRoundRect(card, l.h * 0.03f, l.h * 0.03f, fill)
    }

    private fun drawChevron(canvas: Canvas, x: Float, y: Float, arm: Float) {
        stroke.shader = null
        stroke.color = CHEVRON
        stroke.strokeWidth = arm * 0.45f
        stroke.strokeCap = Paint.Cap.ROUND
        stroke.strokeJoin = Paint.Join.ROUND
        canvas.chevron(mark, stroke, x, y, arm)
    }

    private fun drawWeather(canvas: Canvas, block: RectF, stale: Boolean) {
        val w = weather ?: return
        val size = weatherSize
        val baseline = block.centerY() + size * 0.423f
        text.textAlign = Paint.Align.LEFT
        val iconSize = size * 0.81f
        if (Build.VERSION.SDK_INT >= 23) {
            text.typeface = regular
            text.textSize = iconSize
            text.color = faded(Color.WHITE, stale)
            canvas.drawText(weatherIcon, weatherLeft, baseline - size * 0.06f, text)
        } else {
            // Colour emoji fonts are missing or incomplete on old Android versions.
            drawWeatherIcon(canvas, w.code, w.isDay, weatherLeft + iconSize * 0.6f, baseline - size * 0.375f, iconSize * 0.55f, stale)
        }
        text.typeface = Layout.LIGHT
        text.textSize = size
        text.color = if (stale) MUTED else Color.WHITE
        canvas.drawText(weatherTemp, weatherTempX, baseline, text)

        text.typeface = regular
        text.textSize = weatherLinesSize
        canvas.drawText(weatherFirst, weatherTextX, baseline - size * 0.625f, text)
        text.color = MUTED
        canvas.drawText(weatherSecond, weatherTextX, baseline - size * 0.09f, text)
        drawChevron(canvas, weatherChevronX + size * 0.08f, baseline - size * 0.625f - size * 0.14f, size * 0.08f)
    }

    /** Simple drawn icons for phones without colour emoji: a sun, a moon or a cloud. */
    private fun drawWeatherIcon(canvas: Canvas, code: Int, day: Boolean, cx: Float, cy: Float, r: Float, stale: Boolean) {
        fill.shader = null
        stroke.shader = null
        if (code <= 1) {
            fill.color = faded(if (day) 0xFFFACC15.toInt() else 0xFFE5E7EB.toInt(), stale)
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
        // Opaque even when stale: the three circles of the cloud overlap.
        fill.color = if (stale) 0xFF5B6472.toInt() else if (code in 51..99) 0xFF94A3B8.toInt() else 0xFFCBD5E1.toInt()
        canvas.drawCircle(cx - r * 0.35f, cy + r * 0.15f, r * 0.45f, fill)
        canvas.drawCircle(cx + r * 0.2f, cy - r * 0.1f, r * 0.6f, fill)
        canvas.drawCircle(cx + r * 0.7f, cy + r * 0.2f, r * 0.4f, fill)
        if (code in 51..99) {
            stroke.color = faded(BLUE, stale)
            stroke.strokeWidth = r * 0.1f
            for (k in -1..1) canvas.drawLine(cx + k * r * 0.4f, cy + r * 0.75f, cx + k * r * 0.4f - r * 0.12f, cy + r * 1.05f, stroke)
        }
    }

    private fun drawBox(canvas: Canvas, block: RectF, radius: Float, border: Int, width: Float) {
        fill.shader = null
        fill.color = CARD
        canvas.drawRoundRect(block, radius, radius, fill)
        stroke.shader = null
        stroke.strokeWidth = width
        stroke.color = border
        canvas.drawRoundRect(block, radius, radius, stroke)
    }

    private fun drawHeading(canvas: Canvas, label: String, x: Float, baseline: Float, size: Float, color: Int) {
        text.typeface = bold
        text.textSize = size
        text.spacing(0.14f)
        text.color = color
        text.textAlign = Paint.Align.LEFT
        canvas.drawText(label, x, baseline, text)
        text.spacing(0f)
    }

    private fun drawAlerts(canvas: Canvas, block: RectF, s: Stats?) {
        val left = block.left
        val top = block.top
        val right = block.right
        val height = block.height()
        val inset = height * 0.07f
        val down = pressed === Block.ALERTS
        drawBox(canvas, block, height * 0.085f, if (down) PRESSED else BORDER, max(1f, height * 0.003f) * (if (down) 2f else 1f))

        val header = height * 0.068f
        drawHeading(canvas, "УВЕДОМЛЕНИЯ", left + inset, top + inset + header * 0.8f, header, MUTED)
        drawChevron(canvas, right - inset, top + inset + header * 0.45f, header * 0.3f)

        val alerts = s?.alerts
        val listTop = top + inset + header * 1.7f
        val contentWidth = right - left - inset * 2
        if (alerts.isNullOrEmpty()) {
            text.typeface = regular
            text.textSize = height * 0.075f
            text.color = MUTED
            for (i in 0 until 3) {
                val t = when {
                    s == null -> if (i == 0) "Запустите deskstats на Mac" else if (i == 1) "и подключите телефон по USB." else address
                    s.alertsDenied -> if (i == 0) "macOS не даёт серверу читать" else if (i == 1) "уведомления: выдайте ему" else "«Полный доступ к диску»."
                    alerts == null -> if (i == 0) "Уведомления на сервере выключены." else ""
                    else -> if (i == 0) "Новых уведомлений нет." else ""
                }
                canvas.drawText(text.fit(t, contentWidth), left + inset, listTop + height * (0.12f + 0.11f * i), text)
            }
            return
        }

        stroke.strokeWidth = max(1f, height * 0.003f)
        for (i in rows.indices) {
            val row = rows[i]
            val rowTop = listTop + rowHeight * i
            if (i > 0) {
                stroke.color = GRID
                canvas.drawLine(left + inset, rowTop, right - inset, rowTop, stroke)
            }
            text.typeface = regular
            text.textSize = rowHeight * 0.21f
            text.color = MUTED
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(row.time, right - inset, rowTop + rowHeight * 0.3f, text)
            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.color = TEAL
            canvas.drawText(row.app, left + inset, rowTop + rowHeight * 0.3f, text)

            text.textSize = rowHeight * 0.235f
            text.color = Color.WHITE
            canvas.drawText(row.first, left + inset, rowTop + rowHeight * 0.6f, text)
            if (!row.secondIsTitle) {
                text.typeface = regular
                text.textSize = rowHeight * 0.22f
                text.color = MUTED
            }
            canvas.drawText(row.second, left + inset, rowTop + rowHeight * 0.88f, text)
        }
    }

    /** A just-arrived notification, large, over the whole notifications card. */
    private fun drawHighlight(canvas: Canvas, block: RectF) {
        val left = block.left
        val top = block.top
        val right = block.right
        val height = block.height()
        val inset = height * 0.07f
        drawBox(canvas, block, height * 0.085f, TEAL, max(2f, height * 0.008f))

        val header = height * 0.068f
        drawHeading(canvas, "НОВОЕ УВЕДОМЛЕНИЕ", left + inset, top + inset + header * 0.8f, header, TEAL)
        text.typeface = regular
        text.color = MUTED
        text.textAlign = Paint.Align.RIGHT
        canvas.drawText(highlightTime, right - inset, top + inset + header * 0.8f, text)
        text.textAlign = Paint.Align.LEFT

        var y = top + inset + header * 1.9f
        text.typeface = bold
        text.textSize = height * 0.1f
        text.color = TEAL
        y += text.textSize
        canvas.drawText(highlightApp, left + inset, y, text)

        text.textSize = height * 0.095f
        text.color = Color.WHITE
        for (i in highlightTitle.indices) {
            y += text.textSize * 1.25f
            canvas.drawText(highlightTitle[i], left + inset, y, text)
        }
        text.typeface = regular
        text.textSize = height * 0.08f
        text.color = SOFT_WHITE
        for (i in highlightBody.indices) {
            y += text.textSize * 1.3f
            canvas.drawText(highlightBody[i], left + inset, y, text)
        }
    }

    private fun batteryColor(pct: Int, stale: Boolean) = faded(if (pct < 15) BAD else if (pct <= 30) ORANGE else GOOD, stale)

    /** What the batteries block says when it has no device to show. */
    private fun batteryNote(s: Stats?) = if (s == null) "Заряд мыши и клавиатуры появится, когда ответит Mac" else "Нет беспроводных устройств с зарядом"

    /** One slot per wireless device, emptiest first: its kind, the charge in percent and a bar. */
    private fun drawBatteries(canvas: Canvas, block: RectF, s: Stats?, stale: Boolean) {
        val left = block.left
        val top = block.top
        val right = block.right
        val height = block.height()
        val inset = height * 0.24f
        drawBox(canvas, block, height * 0.28f, BORDER, max(1f, height * 0.012f))

        text.textAlign = Paint.Align.LEFT
        if (slots.isEmpty()) {
            text.typeface = regular
            text.textSize = height * 0.2f
            text.color = MUTED
            canvas.drawText(text.fit(batteryNote(s), right - left - inset * 2), left + inset, top + height * 0.58f, text)
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
            text.color = if (stale) MUTED else Color.WHITE
            text.textAlign = Paint.Align.RIGHT
            val percentWidth = text.measureText(slot.percent)
            canvas.drawText(slot.percent, x + slotWidth, baseline, text)

            text.typeface = regular
            text.textSize = height * 0.21f
            text.color = MUTED
            text.textAlign = Paint.Align.LEFT
            canvas.drawText(text.fit(slot.label, slotWidth - percentWidth - height * 0.1f), x, baseline, text)
            drawBar(canvas, x, top + height * 0.64f, slotWidth, height * 0.1f, slot.pct, stale)
        }
        if (slotsHidden.isNotEmpty()) {
            text.typeface = bold
            text.textSize = height * 0.22f
            text.color = MUTED
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(slotsHidden, right - inset, top + height * 0.58f, text)
        }
    }

    private fun drawBar(canvas: Canvas, x: Float, top: Float, width: Float, height: Float, pct: Int, stale: Boolean) {
        fill.shader = null
        fill.color = GRID
        card.set(x, top, x + width, top + height)
        canvas.drawRoundRect(card, height / 2, height / 2, fill)
        fill.color = batteryColor(pct, stale)
        card.set(x, top, x + max(height, width * pct / 100f), top + height)
        canvas.drawRoundRect(card, height / 2, height / 2, fill)
    }

    /** The batteries with their zone to themselves: a card like the notifications, a device per row. */
    private fun drawBatteryList(canvas: Canvas, l: Layout, block: RectF, s: Stats?, stale: Boolean) {
        val inset = l.h * 0.03f
        val header = l.h * 0.027f
        drawBox(canvas, block, l.h * 0.034f, BORDER, max(1f, l.h * 0.0015f))
        drawHeading(canvas, "ЗАРЯД УСТРОЙСТВ", block.left + inset, block.top + inset + header * 0.8f, header, MUTED)
        if (slotsHidden.isNotEmpty()) {
            text.typeface = regular
            text.color = MUTED
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(slotsHidden, block.right - inset, block.top + inset + header * 0.8f, text)
            text.textAlign = Paint.Align.LEFT
        }
        val listTop = block.top + inset + header * 1.7f
        val width = block.width() - inset * 2
        if (slots.isEmpty()) {
            text.typeface = regular
            text.textSize = l.h * 0.034f
            text.color = MUTED
            canvas.drawText(text.fit(batteryNote(s), width), block.left + inset, listTop + l.h * 0.06f, text)
            return
        }
        val rowHeight = min((block.bottom - inset * 0.6f - listTop) / slots.size, block.height() * 0.38f)
        for (i in slots.indices) {
            val slot = slots[i]
            val top = listTop + rowHeight * i
            val baseline = top + rowHeight * 0.5f
            text.typeface = bold
            text.textSize = rowHeight * 0.34f
            text.color = if (stale) MUTED else Color.WHITE
            text.textAlign = Paint.Align.RIGHT
            val percentWidth = text.measureText(slot.percent)
            canvas.drawText(slot.percent, block.right - inset, baseline, text)
            text.typeface = regular
            text.textSize = rowHeight * 0.26f
            text.color = if (stale) MUTED else SOFT_WHITE
            text.textAlign = Paint.Align.LEFT
            canvas.drawText(text.fit(slot.label, width - percentWidth - inset), block.left + inset, baseline, text)
            drawBar(canvas, block.left + inset, top + rowHeight * 0.68f, width, rowHeight * 0.085f, slot.pct, stale)
        }
    }

    /**
     * What a card's texts and strokes are sized by. A card is normally wider than tall and its
     * height sets the sizes; a tall narrow one (cards alone on the screen) goes by its width, so
     * the texts stay inside it and the chart takes the rest.
     */
    private fun cardUnit(r: RectF) = min(r.height(), r.width() * 0.72f)

    /**
     * Draws card [index] at [card]: background, chart of the given series scaled to [top], the
     * texts and the small [corner] label (the top of the network scale).
     */
    private fun drawCard(canvas: Canvas, index: Int, c: CardText, corner: String, color: Int, top: Float, series: Array<Series>, stale: Boolean, down: Boolean) {
        val ch = cardUnit(card)
        val radius = ch * CARD_RADIUS

        fill.shader = null
        fill.color = CARD
        canvas.drawRoundRect(card, radius, radius, fill)

        // A rectangular clip is cheap; what it leaves outside the round corners is painted over below.
        canvas.save()
        canvas.clipRect(card)
        val chartTop = card.top + ch * CHART_TOP
        val chartBottom = card.bottom - ch * 0.03f
        stroke.shader = null
        stroke.strokeWidth = ch * 0.016f
        stroke.strokeJoin = Paint.Join.ROUND
        stroke.strokeCap = Paint.Cap.ROUND
        val step = card.width() / (HISTORY - 1)
        for (s in series) {
            val startX = card.right - step * (s.count - 1)
            var i = 0
            // Each unbroken run of values is one line with its fill; NaN gaps stay empty.
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
                // Charts from before the link was lost are faded, so the panel does not look alive.
                fill.color = Color.WHITE
                fill.alpha = if (stale) STALE_ALPHA else 255
                fill.shader = gradients[s.color]
                canvas.drawPath(area, fill)
                fill.shader = null
                stroke.color = faded(s.color, stale)
                canvas.drawPath(line, stroke)
            }
        }
        canvas.restore()
        fill.color = BACKGROUND
        canvas.drawPath(cardCorners[index], fill)

        stroke.strokeWidth = max(1f, ch * 0.008f) * (if (down) 2f else 1f)
        stroke.color = if (down) PRESSED else BORDER
        canvas.drawRoundRect(card, radius, radius, stroke)

        val x = card.left + ch * 0.12f
        val right = card.right - ch * 0.1f
        drawChevron(canvas, right, card.top + ch * 0.165f, ch * 0.03f)
        text.typeface = bold
        text.textSize = ch * 0.1f
        text.spacing(0.14f)
        val titleWidth = text.measureText(c.title)
        text.spacing(0f)
        // The corner label shares the title's line when both fit, else it goes down to the note's line.
        var noteRoom = card.right - ch * 0.08f - x
        if (corner.isNotEmpty()) {
            text.typeface = regular
            text.textSize = ch * 0.085f
            text.color = MUTED
            text.textAlign = Paint.Align.RIGHT
            val cornerWidth = text.measureText(corner)
            if (x + titleWidth + ch * 0.06f + cornerWidth <= right - ch * 0.07f) {
                canvas.drawText(corner, right - ch * 0.07f, card.top + ch * 0.2f, text)
            } else {
                canvas.drawText(corner, card.right - ch * 0.08f, card.top + ch * 0.62f, text)
                noteRoom -= cornerWidth + ch * 0.05f
            }
        }
        text.typeface = bold
        text.textSize = ch * 0.1f
        text.spacing(0.14f)
        text.color = faded(color, stale)
        text.textAlign = Paint.Align.LEFT
        canvas.drawText(text.fit(c.title, right - ch * 0.07f - x), x, card.top + ch * 0.2f, text)
        text.spacing(0f)

        val baseline = card.top + ch * 0.46f
        text.textSize = ch * 0.25f
        // Values from before the link was lost are greyed out, so they are not taken as current.
        text.color = if (stale) DIM_WHITE else Color.WHITE
        canvas.drawText(c.value, x, baseline, text)
        val width = text.measureText(c.value)
        text.typeface = regular
        text.textSize = ch * 0.115f
        text.color = MUTED
        canvas.drawText(text.fit(c.unit, card.right - ch * 0.08f - (x + width + ch * 0.03f)), x + width + ch * 0.03f, baseline, text)
        text.textSize = ch * 0.12f
        canvas.drawText(text.fit(c.note, noteRoom), x, card.top + ch * 0.62f, text)
    }

    /** Icon and Russian label for a WMO weather code. */
    private fun describe(code: Int, day: Boolean): Pair<String, String> = when (code) {
        0 -> (if (day) "☀️" else "🌙") to "Ясно"
        1 -> (if (day) "🌤️" else "🌙") to "Малооблачно"
        2 -> "⛅" to "Переменная облачность"
        3 -> "☁️" to "Пасмурно"
        45, 48 -> "🌫️" to "Туман"
        in 51..57 -> "🌦️" to "Морось"
        in 61..67 -> "🌧️" to "Дождь"
        in 71..77 -> "❄️" to "Снег"
        in 80..82 -> "🌧️" to "Ливень"
        85, 86 -> "🌨️" to "Снегопад"
        in 95..99 -> "⛈️" to "Гроза"
        else -> "☁️" to "Облачно"
    }

    companion object {
        private const val HISTORY = 120
        private const val ALERT_ROWS = 3
        private const val BATTERY_SLOTS = 3
        private const val HIGHLIGHT_MS = 7000L
        private const val WEATHER_GRACE_MS = 20_000L
        private const val CHART_TOP = 0.68f
        private const val CARD_RADIUS = 0.14f
        private const val STALE_ALPHA = 0x73
        private val GEAR = Any()
        private val METRICS = arrayOf(Block.CPU, Block.GPU, Block.RAM, Block.NET)
        val COLORS = intArrayOf(TEAL, GREEN, PURPLE, ORANGE)
        const val BACKGROUND = 0xFF06070A.toInt()
        const val CARD = 0xFF111318.toInt()
        const val BORDER = 0xFF262A32.toInt()
        const val GRID = 0xFF1C2027.toInt()
        const val MUTED = 0xFFA0A8B5.toInt()
        const val DIM_WHITE = 0xFF7C8494.toInt()
        const val SOFT_WHITE = 0xFFD5D9E0.toInt()
        const val CHEVRON = 0xFF596170.toInt()
        const val PRESSED = 0xFF6B7585.toInt()
        const val PRESSED_FILL = 0x16FFFFFF
        const val TEAL = 0xFF2DD4BF.toInt()
        const val GREEN = 0xFF84CC16.toInt()
        const val PURPLE = 0xFFA78BFA.toInt()
        const val ORANGE = 0xFFF59E0B.toInt()
        const val BLUE = 0xFF60A5FA.toInt()
        const val GOOD = 0xFF22C55E.toInt()
        const val BAD = 0xFFF87171.toInt()
    }
}
