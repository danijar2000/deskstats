package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Typeface
import android.view.MotionEvent
import android.view.VelocityTracker
import android.view.View
import android.widget.Scroller
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

/**
 * A full-screen page opened from the main screen: a header with a way back and a body, which
 * scrolls when it is taller than the screen. A page left alone is closed by the activity, so the
 * phone goes back to being a panel (and the Mac stops gathering what only the page needed).
 */
abstract class Page(context: Context, protected val host: Host) : View(context) {
    interface Host {
        fun back()
        fun home()
        fun open(page: Page)
        var places: Places
        var setup: BlockSetup
    }

    protected abstract val title: String
    protected val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    protected val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    protected val text = Paint(Paint.ANTI_ALIAS_FLAG)
    protected val regular: Typeface = Typeface.DEFAULT
    protected val bold: Typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
    protected val box = RectF()
    /** Where the message at the bottom was drawn; empty while there is none. */
    protected val toastBox = RectF()
    private val chevron = Path()

    /** Screen height, the unit every size is derived from. */
    protected var h = 0f
    protected var w = 0f
    protected var pad = 0f
    protected var gap = 0f
    protected var headerBottom = 0f
    protected val bodyTop get() = headerBottom + gap
    protected val bodyHeight get() = h - pad - bodyTop

    // Scroll offsets of the panes: the body and, on a page with two, the one beside it.
    private val scrolls = FloatArray(PANES)
    protected val scroll get() = scrolls[0]

    private val scroller = Scroller(context)
    private var velocity: VelocityTracker? = null
    private var downX = 0f
    private var downY = 0f
    private var lastY = 0f
    private var moved = false
    private var grabbed = false
    private var tracking = false
    private var pane = -1
    private var flingPane = 0

    /** Height of what pane [pane] scrolls; zero when it does not scroll. */
    protected open fun contentHeight(pane: Int) = 0f

    /** The pane a drag that started at [x] scrolls, -1 for none. */
    protected open fun paneAt(x: Float) = 0

    protected open fun viewTop(pane: Int) = bodyTop

    protected open fun viewHeight(pane: Int) = bodyHeight

    /** Where the scroll mark of [pane] is drawn. */
    protected open fun barX(pane: Int) = w - pad * 0.5f

    protected open fun drawHeader(canvas: Canvas) {}

    /** Drawn clipped to the body and moved by the scroll offset. */
    protected abstract fun drawBody(canvas: Canvas)

    /** Drawn over the body without scrolling. */
    protected open fun drawFixed(canvas: Canvas) {}

    protected open fun onTap(x: Float, y: Float) {}

    /** A touch that starts a drag of an item rather than a scroll or a tap. */
    protected open fun onGrab(x: Float, y: Float) = false

    protected open fun onDrag(x: Float, y: Float) {}

    protected open fun onDrop() {}

    protected open fun arrange() {}

    /** What the page shows, in words, for TalkBack. */
    protected open fun summary() = ""

    /** Tells TalkBack what the page shows now; called whenever its content changes. */
    protected fun announce() {
        val body = summary()
        contentDescription = if (body.isEmpty()) title else "$title. $body"
    }

    /** The page is on top again after the one opened from it was closed. */
    open fun resumed() {
        announce()
        invalidate()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        announce()
    }

    override fun onDetachedFromWindow() {
        velocity?.recycle()
        velocity = null
        super.onDetachedFromWindow()
    }

    override fun onSizeChanged(width: Int, height: Int, oldw: Int, oldh: Int) {
        w = width.toFloat()
        h = height.toFloat()
        pad = h * 0.04f
        gap = h * 0.028f
        headerBottom = pad + h * 0.1f
        arrange()
    }

    private fun maxScroll(pane: Int) = max(0f, contentHeight(pane) - viewHeight(pane))

    protected fun scrollOf(pane: Int) = scrolls[pane]

    protected fun scrollTo(pane: Int, value: Float) {
        scrolls[pane] = value.coerceIn(0f, maxScroll(pane))
    }

    override fun onDraw(canvas: Canvas) {
        // The content may have become shorter since the last touch.
        for (p in 0 until PANES) scrolls[p] = scrolls[p].coerceIn(0f, maxScroll(p))
        // The way back: a chevron and the title.
        val mid = pad + h * 0.05f
        val arm = h * 0.022f
        chevron.rewind()
        chevron.moveTo(pad + arm * 1.4f, mid - arm)
        chevron.lineTo(pad + arm * 0.4f, mid)
        chevron.lineTo(pad + arm * 1.4f, mid + arm)
        stroke.color = PanelView.MUTED
        stroke.strokeWidth = h * 0.007f
        stroke.strokeCap = Paint.Cap.ROUND
        stroke.strokeJoin = Paint.Join.ROUND
        canvas.drawPath(chevron, stroke)
        text.typeface = bold
        text.textAlign = Paint.Align.LEFT
        text.textSize = h * 0.058f
        text.color = Color.WHITE
        canvas.drawText(title, pad + h * 0.065f, mid + h * 0.021f, text)
        drawHeader(canvas)

        canvas.save()
        canvas.clipRect(0f, bodyTop, w, h)
        canvas.translate(0f, -scroll)
        drawBody(canvas)
        canvas.restore()
        toastBox.setEmpty()
        drawFixed(canvas)

        // A mark at the side of whatever scrolls: where in the list the screen is.
        fill.shader = null
        fill.color = SCROLL_MARK
        for (p in 0 until PANES) {
            val range = maxScroll(p)
            if (range <= 0f) continue
            val view = viewHeight(p)
            val length = max(h * 0.06f, view * view / contentHeight(p))
            val top = viewTop(p) + (view - length) * scrolls[p] / range
            val half = h * 0.003f
            box.set(barX(p) - half, top, barX(p) + half, top + length)
            canvas.drawRoundRect(box, half, half, fill)
        }
    }

    private fun endGesture() {
        if (grabbed) onDrop()
        grabbed = false
        tracking = false
        velocity?.recycle()
        velocity = null
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                scroller.forceFinished(true)
                downX = event.x
                downY = event.y
                lastY = event.y
                moved = false
                tracking = true
                pane = if (event.y > bodyTop) paneAt(event.x) else -1
                grabbed = event.y > bodyTop && onGrab(event.x, event.y)
                velocity?.recycle()
                velocity = VelocityTracker.obtain().also { it.addMovement(event) }
            }
            MotionEvent.ACTION_MOVE -> {
                if (!tracking) return true
                velocity?.addMovement(event)
                if (grabbed) {
                    onDrag(event.x, event.y)
                    return true
                }
                val slop = h * 0.025f
                if (!moved && (abs(event.x - downX) > slop || abs(event.y - downY) > slop)) moved = true
                if (moved && pane >= 0) {
                    scrollTo(pane, scrolls[pane] - (event.y - lastY))
                    invalidate()
                }
                lastY = event.y
            }
            MotionEvent.ACTION_UP -> {
                if (!tracking) return true
                when {
                    grabbed -> {}
                    !moved && event.y < headerBottom && event.x < w * 0.35f -> host.back()
                    !moved -> onTap(event.x, event.y)
                    pane >= 0 -> velocity?.let {
                        it.computeCurrentVelocity(1000)
                        flingPane = pane
                        scroller.fling(0, scrolls[pane].toInt(), 0, -it.yVelocity.toInt(), 0, 0, 0, maxScroll(pane).toInt())
                        invalidate()
                    }
                }
                endGesture()
            }
            // A second finger ends the gesture: its coordinates must not move the list or the dragged row.
            MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_CANCEL -> endGesture()
        }
        return true
    }

    override fun computeScroll() {
        if (scroller.computeScrollOffset()) {
            scrollTo(flingPane, scroller.currY.toFloat())
            invalidate()
        }
    }

    /** A rounded card at [box]. */
    protected fun card(canvas: Canvas, radius: Float, border: Int = PanelView.BORDER, background: Int = PanelView.CARD) {
        fill.shader = null
        fill.color = background
        canvas.drawRoundRect(box, radius, radius, fill)
        stroke.color = border
        stroke.strokeWidth = max(1f, h * 0.002f)
        canvas.drawRoundRect(box, radius, radius, stroke)
    }

    /** A line in the middle of the body, for when there is nothing to list. */
    protected fun message(canvas: Canvas, line: String) {
        text.typeface = regular
        text.textAlign = Paint.Align.CENTER
        text.textSize = h * 0.045f
        text.color = PanelView.MUTED
        canvas.drawText(text.fit(line, w - pad * 2), w / 2, bodyTop + bodyHeight * 0.45f, text)
        text.textAlign = Paint.Align.LEFT
    }

    /** A small heading over a group of rows. */
    protected fun heading(canvas: Canvas, label: String, baseline: Float) {
        text.typeface = bold
        text.textSize = h * 0.03f
        text.spacing(0.14f)
        text.color = PanelView.MUTED
        text.textAlign = Paint.Align.LEFT
        canvas.drawText(label, pad + h * 0.008f, baseline, text)
        text.spacing(0f)
    }

    /** A message over the bottom of the page, with an [action] to tap when there is one; sets [toastBox]. */
    protected fun toast(canvas: Canvas, line: String, action: String, color: Int) {
        val height = h * 0.1f
        val inset = height * 0.4f
        text.typeface = bold
        text.textSize = height * 0.36f
        val actionWidth = if (action.isEmpty()) 0f else text.measureText(action) + inset
        text.typeface = regular
        val shown = text.fit(line, w - pad * 2 - inset * 2 - actionWidth)
        val width = text.measureText(shown) + inset * 2 + actionWidth
        toastBox.set((w - width) / 2, h - pad - height, (w + width) / 2, h - pad)
        box.set(toastBox)
        card(canvas, height / 2, color, 0xFF1A1D24.toInt())
        text.textAlign = Paint.Align.LEFT
        text.color = PanelView.SOFT_WHITE
        canvas.drawText(shown, box.left + inset, box.centerY() + text.textSize * 0.36f, text)
        if (action.isNotEmpty()) {
            text.typeface = bold
            text.color = color
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(action, box.right - inset, box.centerY() + text.textSize * 0.36f, text)
            text.textAlign = Paint.Align.LEFT
        }
    }

    private companion object {
        const val PANES = 2
        const val SCROLL_MARK = 0xFF3B4250.toInt()
    }
}

class Proc(val name: String, val cpu: Float, val ram: Long, val down: Float, val up: Float, val gpu: Float, val count: Int)

/**
 * The applications of the Mac in two columns, heaviest first by the load whose card was tapped.
 * The tabs in the header switch the measure. The Mac gathers this list only while it is asked.
 */
class ProcsPage(context: Context, host: Host, sort: Block, private val onSort: (Block) -> Unit) : Page(context, host) {
    /** The texts of one application, made when the list arrives rather than on every frame. */
    private class Line(val name: String, val value: String, val parts: String, val share: Float)

    override val title = "Приложения"
    var sort = sort
        private set
    private var procs: List<Proc>? = null
    private var lines: List<Line> = emptyList()
    private var footer = ""
    private var total = 0
    private var problem: String? = null
    private val tabs = Array(TABS.size) { RectF() }
    private var rowHeight = 0f

    var link = Link.ONLINE
        set(value) {
            if (field != value) {
                field = value
                invalidate()
            }
        }

    fun push(sortId: String, list: List<Proc>, total: Int) {
        // An answer to the measure that was shown a moment ago is dropped.
        if (sortId != sort.id) return
        procs = list
        this.total = total
        problem = null
        prepare()
        invalidate()
    }

    /** The Mac did not give the list: [code] is its HTTP status, 0 when there was no usable answer. */
    fun fail(code: Int) {
        problem = if (code == 0) "Mac не ответил на запрос списка приложений" else "Mac не отдал список приложений (код $code)"
        announce()
        invalidate()
    }

    private fun key(p: Proc): Float = when (sort) {
        Block.GPU -> p.gpu
        Block.RAM -> p.ram.toFloat()
        Block.NET -> p.down + p.up
        else -> p.cpu
    }

    private fun prepare() {
        val all = procs ?: return
        // Network and GPU are used by a few applications; the rest would be a page of zeroes.
        val list = if (sort == Block.NET || sort == Block.GPU) all.filter { key(it) > 0f } else all
        val top = list.maxOfOrNull { key(it) } ?: 0f
        lines = list.map { p ->
            val value = when (sort) {
                Block.RAM -> Fmt.size(p.ram)
                // The list is ordered by the sum of both directions, so the sum is what is shown.
                Block.NET -> Fmt.rate(p.down + p.up)
                Block.GPU -> Fmt.percent(p.gpu)
                else -> Fmt.percent(p.cpu)
            }
            val parts = ArrayList<String>(5)
            if (sort == Block.NET) parts.add("↓ ${Fmt.rate(p.down)}  ↑ ${Fmt.rate(p.up)}")
            if (sort != Block.CPU) parts.add("CPU " + Fmt.percent(p.cpu))
            if (sort != Block.RAM) parts.add("RAM " + Fmt.size(p.ram))
            if (sort != Block.GPU && p.gpu >= 0.1f) parts.add("GPU " + Fmt.percent(p.gpu))
            if (sort != Block.NET && p.down + p.up >= 1024f) parts.add("сеть " + Fmt.rate(p.down + p.up))
            if (p.count > 1) parts.add("процессов: ${p.count}")
            // The Mac counts CPU per core, as Activity Monitor does: over 100 % means several cores.
            if (sort == Block.CPU && p.cpu >= 100f) parts.add(0, Fmt.number("%.1f", p.cpu / 100f) + " ядра")
            Line(p.name, value, parts.joinToString(" · "), if (top > 0f) (key(p) / top).coerceIn(0f, 1f) else 0f)
        }
        footer = if (list === all && total > all.size) "Показаны первые ${all.size} из $total" else ""
        announce()
    }

    override fun summary(): String {
        val what = LABELS[TABS.indexOf(sort)]
        problem?.let { return it }
        if (procs == null) return "Список по нагрузке $what загружается"
        return "По нагрузке $what: " + lines.take(5).joinToString(", ") { "${it.name} ${it.value}" }
    }

    override fun arrange() {
        val rows = max(1, (bodyHeight / (h * 0.118f)).toInt())
        rowHeight = bodyHeight / rows
        // The tabs take what the title leaves, so on a narrow screen they shrink instead of covering it.
        text.typeface = bold
        text.textSize = h * 0.058f
        val titleEnd = pad + h * 0.065f + text.measureText(title) + gap
        val between = gap * 0.5f
        val tabWidth = min(h * 0.19f, (w - pad - titleEnd - between * (TABS.size - 1)) / TABS.size)
        val tabHeight = h * 0.075f
        val top = pad + h * 0.05f - tabHeight / 2
        for (i in TABS.indices) {
            val right = w - pad - (TABS.size - 1 - i) * (tabWidth + between)
            tabs[i].set(right - tabWidth, top, right, top + tabHeight)
        }
    }

    override fun contentHeight(pane: Int) = if (pane != 0) 0f else (lines.size + 1) / 2 * rowHeight + if (footer.isEmpty()) 0f else rowHeight * 0.6f

    override fun drawHeader(canvas: Canvas) {
        for (i in TABS.indices) {
            val block = TABS[i]
            val active = block == sort
            box.set(tabs[i])
            card(canvas, box.height() / 2, if (active) color(block) else PanelView.BORDER, if (active) PanelView.CARD else PanelView.BACKGROUND)
            text.typeface = bold
            text.textAlign = Paint.Align.CENTER
            text.textSize = min(box.height() * 0.42f, box.width() * 0.23f)
            text.spacing(0.1f)
            text.color = if (active) color(block) else PanelView.MUTED
            canvas.drawText(LABELS[i], box.centerX(), box.centerY() + text.textSize * 0.36f, text)
            text.spacing(0f)
        }
        text.textAlign = Paint.Align.LEFT
    }

    override fun drawBody(canvas: Canvas) {
        if (procs == null || lines.isEmpty()) return
        val stale = link != Link.ONLINE || problem != null
        val columnWidth = (w - pad * 2 - gap) / 2
        val accent = color(sort)
        for (i in lines.indices) {
            val line = lines[i]
            val left = pad + (columnWidth + gap) * (i % 2)
            val y = bodyTop + rowHeight * (i / 2)
            if (y + rowHeight < bodyTop + scroll || y > scroll + h) continue
            box.set(left, y + rowHeight * 0.06f, left + columnWidth, y + rowHeight * 0.94f)
            val rh = box.height()
            card(canvas, rh * 0.22f)
            val inset = rh * 0.26f
            val idleRow = line.share <= 0f

            text.typeface = bold
            text.textSize = rh * 0.3f
            text.textAlign = Paint.Align.RIGHT
            text.color = if (stale || idleRow) PanelView.DIM_WHITE else accent
            canvas.drawText(line.value, box.right - inset, box.top + rh * 0.42f, text)
            val valueWidth = text.measureText(line.value)

            text.textAlign = Paint.Align.LEFT
            text.color = if (stale) PanelView.DIM_WHITE else Color.WHITE
            canvas.drawText(text.fit(line.name, box.width() - inset * 2 - valueWidth - rh * 0.2f), box.left + inset, box.top + rh * 0.42f, text)

            text.typeface = regular
            text.textSize = rh * 0.21f
            text.color = PanelView.MUTED
            canvas.drawText(text.fit(line.parts, box.width() - inset * 2), box.left + inset, box.top + rh * 0.72f, text)

            // How this application compares with the heaviest one on the page.
            val barTop = box.top + rh * 0.84f
            val barHeight = rh * 0.05f
            val barWidth = box.width() - inset * 2
            fill.color = PanelView.GRID
            box.set(left + inset, barTop, left + inset + barWidth, barTop + barHeight)
            canvas.drawRoundRect(box, barHeight / 2, barHeight / 2, fill)
            if (!idleRow) {
                fill.color = if (stale) PanelView.DIM_WHITE else accent
                box.right = box.left + max(barHeight, barWidth * line.share)
                canvas.drawRoundRect(box, barHeight / 2, barHeight / 2, fill)
            }
        }
        if (footer.isNotEmpty()) {
            text.typeface = regular
            text.textSize = h * 0.032f
            text.color = PanelView.DIM_WHITE
            text.textAlign = Paint.Align.CENTER
            canvas.drawText(footer, w / 2, bodyTop + (lines.size + 1) / 2 * rowHeight + rowHeight * 0.4f, text)
            text.textAlign = Paint.Align.LEFT
        }
    }

    override fun drawFixed(canvas: Canvas) {
        val trouble = problem
        val away = link == Link.OFFLINE || link == Link.NO_TOKEN || link == Link.REFUSED
        when {
            procs == null -> message(canvas, trouble ?: if (away) "Нет связи с Mac" else "Mac собирает данные…")
            // A stale list stays on the screen, greyed, with the reason under it.
            trouble != null -> toast(canvas, trouble, "", PanelView.BAD)
            away -> toast(canvas, "Нет связи с Mac", "", PanelView.BAD)
            lines.isEmpty() -> message(canvas, if (sort == Block.NET) "Сейчас сетью никто не пользуется" else if (sort == Block.GPU) "Сейчас видеоядро никто не нагружает" else "Нет данных")
        }
    }

    override fun onTap(x: Float, y: Float) {
        val i = tabs.indexOfFirst { x >= it.left - gap * 0.25f && x <= it.right + gap * 0.25f && y <= headerBottom + gap }
        if (i < 0 || TABS[i] == sort) return
        sort = TABS[i]
        // The old list, put in the new order, stands in until the Mac answers for the new measure.
        procs = procs?.sortedByDescending { key(it) }
        total = 0
        prepare()
        scrollTo(0, 0f)
        onSort(sort)
        invalidate()
    }

    private fun color(block: Block) = PanelView.COLORS[TABS.indexOf(block)]

    private companion object {
        val TABS = arrayOf(Block.CPU, Block.GPU, Block.RAM, Block.NET)
        val LABELS = arrayOf("CPU", "GPU", "RAM", "СЕТЬ")
    }
}

/** All recent notifications: the list on the left, the chosen one in full on the right. */
class AlertsPage(context: Context, host: Host) : Page(context, host) {
    override val title = "Уведомления"
    private var alerts: List<Alert> = emptyList()
    private var times: List<String> = emptyList()
    private var day = 0L
    private var empty = "Ожидание данных…"
    private var selected: String? = null
    // The same zone as the clock of the main screen, so the times can be compared with it.
    private val stamps = Stamps().apply { zone = host.places.active?.zone ?: TimeZone.getDefault() }

    // The chosen notification in full: its title lines, then the lines of its text.
    private val detail = ArrayList<String>()
    private var detailTitleLines = 0

    private val listWidth get() = w * 0.4f
    private val rowHeight get() = h * 0.15f
    private val inset get() = h * 0.04f
    private val detailLeft get() = pad + listWidth + gap
    private val detailTop get() = bodyTop + rowHeight * 0.05f + inset + h * 0.07f

    fun push(stats: Stats) {
        val list = stats.alerts ?: emptyList()
        val now = System.currentTimeMillis()
        val note = when {
            stats.alertsDenied -> "macOS не даёт серверу читать уведомления"
            stats.alerts == null -> "Уведомления на сервере выключены"
            else -> "Уведомлений за двое суток нет"
        }
        // Asked every second, the Mac mostly answers the same: nothing to redraw then.
        if (list == alerts && note == empty && stamps.day(now) == day) return
        alerts = list
        empty = note
        day = stamps.day(now)
        times = list.map { stamps.text(it.ts, now) }
        if (alerts.none { it.key == selected }) select(alerts.firstOrNull())
        prepareDetail()
        announce()
        invalidate()
    }

    private fun select(alert: Alert?) {
        selected = alert?.key
        scrollTo(1, 0f)
    }

    private fun chosen() = alerts.firstOrNull { it.key == selected }

    override fun summary(): String {
        val alert = chosen() ?: return empty
        return "Всего ${alerts.size}. Выбрано: ${alert.app}, ${alert.title}. ${alert.body}"
    }

    override fun arrange() = prepareDetail()

    /** Breaks the chosen notification into lines of the right pane; all of them, the pane scrolls. */
    private fun prepareDetail() {
        detail.clear()
        detailTitleLines = 0
        val alert = chosen() ?: return
        if (w <= 0f) return
        val width = w - pad - detailLeft - inset * 2
        text.typeface = bold
        text.textSize = h * TITLE_SIZE
        detail.addAll(text.wrap(alert.title, width, 4))
        detailTitleLines = detail.size
        text.typeface = regular
        text.textSize = h * BODY_SIZE
        for (paragraph in alert.body.split('\n')) detail.addAll(text.wrap(paragraph, width, 200))
    }

    override fun contentHeight(pane: Int) = when (pane) {
        0 -> alerts.size * rowHeight
        else -> detailTitleLines * h * TITLE_SIZE * 1.3f + h * 0.02f + (detail.size - detailTitleLines) * h * BODY_SIZE * 1.4f + h * 0.02f
    }

    override fun paneAt(x: Float) = if (x < pad + listWidth + gap * 0.5f) 0 else 1

    override fun viewTop(pane: Int) = if (pane == 0) bodyTop else detailTop

    override fun viewHeight(pane: Int) = if (pane == 0) bodyHeight else h - pad - inset * 0.6f - detailTop

    override fun barX(pane: Int) = if (pane == 0) pad + listWidth + gap * 0.5f else w - pad - inset * 0.45f

    override fun drawBody(canvas: Canvas) {
        val rh = rowHeight
        for (i in alerts.indices) {
            val alert = alerts[i]
            val top = bodyTop + rh * i
            if (top + rh < bodyTop + scroll || top > bodyTop + scroll + bodyHeight + pad) continue
            box.set(pad, top + rh * 0.05f, pad + listWidth, top + rh * 0.95f)
            val active = alert.key == selected
            card(canvas, rh * 0.18f, if (active) PanelView.TEAL else PanelView.BORDER, if (active) PanelView.CARD else PanelView.BACKGROUND)
            val inset = rh * 0.2f
            text.typeface = regular
            text.textSize = rh * 0.2f
            text.color = PanelView.MUTED
            text.textAlign = Paint.Align.RIGHT
            val time = times[i]
            canvas.drawText(time, box.right - inset, box.top + rh * 0.34f, text)
            val timeWidth = text.measureText(time)
            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.color = PanelView.TEAL
            canvas.drawText(text.fit(alert.app, box.width() - inset * 2.5f - timeWidth), box.left + inset, box.top + rh * 0.34f, text)
            text.textSize = rh * 0.23f
            text.color = Color.WHITE
            canvas.drawText(text.fit(alert.title, box.width() - inset * 2), box.left + inset, box.top + rh * 0.68f, text)
        }
    }

    override fun drawFixed(canvas: Canvas) {
        if (alerts.isEmpty()) {
            message(canvas, empty)
            return
        }
        val index = alerts.indexOfFirst { it.key == selected }
        if (index < 0) return
        val alert = alerts[index]
        box.set(detailLeft, bodyTop + rowHeight * 0.05f, w - pad, h - pad)
        val left = box.left
        val top = box.top
        val right = box.right
        card(canvas, h * 0.03f)
        val width = right - left - inset * 2

        text.typeface = regular
        text.textSize = h * 0.036f
        text.color = PanelView.MUTED
        text.textAlign = Paint.Align.RIGHT
        val time = times[index]
        canvas.drawText(time, right - inset, top + inset + h * 0.036f, text)
        val timeWidth = text.measureText(time)
        text.textAlign = Paint.Align.LEFT
        text.typeface = bold
        text.textSize = h * 0.042f
        text.color = PanelView.TEAL
        canvas.drawText(text.fit(alert.app, width - timeWidth - inset), left + inset, top + inset + h * 0.036f, text)

        // The text scrolls under the line with the application and the time.
        canvas.save()
        canvas.clipRect(left, detailTop, right, detailTop + viewHeight(1))
        var y = detailTop - scrollOf(1)
        text.textSize = h * TITLE_SIZE
        text.color = Color.WHITE
        for (i in detail.indices) {
            if (i == detailTitleLines) {
                y += h * 0.02f
                text.typeface = regular
                text.textSize = h * BODY_SIZE
                text.color = PanelView.SOFT_WHITE
            }
            y += text.textSize * (if (i < detailTitleLines) 1.3f else 1.4f)
            if (y > detailTop - text.textSize && y < h) canvas.drawText(detail[i], left + inset, y, text)
        }
        canvas.restore()
    }

    override fun onTap(x: Float, y: Float) {
        if (x > pad + listWidth || y < bodyTop) return
        val alert = alerts.getOrNull(((y - bodyTop + scroll) / rowHeight).toInt()) ?: return
        select(alert)
        prepareDetail()
        announce()
        invalidate()
    }

    private companion object {
        const val TITLE_SIZE = 0.052f
        const val BODY_SIZE = 0.044f
    }
}

/** "UTC+6", "UTC+5:30", "UTC−8". */
private fun offsetLabel(zone: TimeZone, at: Long): String {
    val minutes = zone.getOffset(at) / 60_000
    val sign = if (minutes < 0) "−" else "+"
    val rest = abs(minutes) % 60
    return "UTC$sign${abs(minutes) / 60}" + if (rest != 0) String.format(Locale.US, ":%02d", rest) else ""
}

/**
 * The user's cities. The chosen one sets the time zone of the clock and the place of the
 * weather; the first entry returns both to the phone's own zone and the city set on the Mac.
 */
class PlacesPage(context: Context, host: Host) : Page(context, host) {
    override val title = "Часовой пояс и город"
    private val timeFormat = SimpleDateFormat("HH:mm", Locale.US)
    private val dateFormat = SimpleDateFormat("EEEE, d MMMM", Locale("ru"))
    private val now = Date()
    private val rowHeight get() = h * 0.16f

    // The city removed a moment ago, which can still be put back.
    private var removed: City? = null
    private var removedIndex = 0
    private var removedChosen = false
    private val forget = Runnable {
        removed = null
        invalidate()
    }

    private val minute = object : Runnable {
        override fun run() {
            invalidate()
            postDelayed(this, 60_000L - System.currentTimeMillis() % 60_000L)
        }
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        post(minute)
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(minute)
        removeCallbacks(forget)
        super.onDetachedFromWindow()
    }

    override fun summary(): String {
        val places = host.places
        val chosen = places.active?.name ?: "как на телефоне и Mac"
        return "Выбрано: $chosen. Города: " + (places.saved.joinToString(", ") { it.name }.ifEmpty { "нет" })
    }

    override fun contentHeight(pane: Int) = if (pane != 0) 0f else (host.places.saved.size + 2) * rowHeight

    /** The middle of the cross that removes the city of the row ending at [right]. */
    private fun crossX(right: Float) = right - rowHeight * 0.3f - rowHeight * 0.07f

    override fun drawBody(canvas: Canvas) {
        val places = host.places
        val rh = rowHeight
        now.time = System.currentTimeMillis()
        for (i in 0..places.saved.size + 1) {
            val top = bodyTop + rh * i
            box.set(pad, top + rh * 0.05f, w - pad, top + rh * 0.95f)
            val inset = rh * 0.3f
            if (i == places.saved.size + 1) {
                card(canvas, rh * 0.2f, PanelView.BORDER, PanelView.BACKGROUND)
                text.typeface = bold
                text.textSize = rh * 0.26f
                text.color = PanelView.TEAL
                text.textAlign = Paint.Align.CENTER
                canvas.drawText("+  Добавить город", box.centerX(), box.centerY() + rh * 0.09f, text)
                text.textAlign = Paint.Align.LEFT
                continue
            }
            val city = places.saved.getOrNull(i - 1)
            val active = city?.id == places.active?.id
            val zone = city?.zone ?: TimeZone.getDefault()
            card(canvas, rh * 0.2f, if (active) PanelView.TEAL else PanelView.BORDER)

            // The mark of the chosen entry.
            val cx = box.left + inset
            stroke.color = if (active) PanelView.TEAL else PanelView.MUTED
            stroke.strokeWidth = rh * 0.025f
            canvas.drawCircle(cx, box.centerY(), rh * 0.1f, stroke)
            if (active) {
                fill.color = PanelView.TEAL
                canvas.drawCircle(cx, box.centerY(), rh * 0.055f, fill)
            }

            var right = box.right - inset
            if (city != null) {
                // Remove: a cross at the end of the row, well clear of the time.
                val r = rh * 0.1f
                val x = crossX(box.right)
                stroke.color = PanelView.MUTED
                stroke.strokeWidth = rh * 0.022f
                canvas.drawLine(x - r, box.centerY() - r, x + r, box.centerY() + r, stroke)
                canvas.drawLine(x - r, box.centerY() + r, x + r, box.centerY() - r, stroke)
            }
            // Every row keeps the room of the cross, so the times stand in one column.
            right -= rh * 0.7f
            timeFormat.timeZone = zone
            dateFormat.timeZone = zone
            text.typeface = Layout.LIGHT
            text.textSize = rh * 0.42f
            text.color = Color.WHITE
            text.textAlign = Paint.Align.RIGHT
            val time = timeFormat.format(now)
            canvas.drawText(time, right, box.centerY() + rh * 0.15f, text)
            val room = right - text.measureText(time) - rh * 0.3f - (cx + rh * 0.3f)

            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.textSize = rh * 0.26f
            canvas.drawText(text.fit(city?.name ?: "Как на телефоне и Mac", room), cx + rh * 0.3f, box.top + rh * 0.4f, text)
            text.typeface = regular
            text.textSize = rh * 0.19f
            text.color = PanelView.MUTED
            val note = if (city == null) "Часовой пояс телефона · погода города, заданного на Mac" else "${offsetLabel(zone, now.time)} · ${dateFormat.format(now)}"
            canvas.drawText(text.fit(note, room), cx + rh * 0.3f, box.top + rh * 0.7f, text)
        }
    }

    override fun drawFixed(canvas: Canvas) {
        removed?.let { toast(canvas, "Город «${it.name}» убран из списка", "Вернуть", PanelView.TEAL) }
    }

    override fun onTap(x: Float, y: Float) {
        val places = host.places
        val gone = removed
        if (gone != null && toastBox.contains(x, y)) {
            removeCallbacks(forget)
            removed = null
            host.places = places.restored(gone, removedIndex, removedChosen)
            announce()
            invalidate()
            return
        }
        if (y < bodyTop) return
        val i = ((y - bodyTop + scroll) / rowHeight).toInt()
        when {
            i == 0 -> {
                host.places = Places(places.saved, null)
                host.home()
            }
            i == places.saved.size + 1 -> host.open(CityPage(context, host))
            i in 1..places.saved.size -> {
                val city = places.saved[i - 1]
                // Only the cross itself removes; the time next to it chooses the city like the rest of the row.
                // From halfway between the time and the cross to the edge of the row.
                if (x > crossX(w - pad) - rowHeight * 0.17f) {
                    removed = city
                    removedIndex = i - 1
                    removedChosen = places.active?.id == city.id
                    host.places = places.without(city)
                    removeCallbacks(forget)
                    postDelayed(forget, UNDO_MS)
                    announce()
                    invalidate()
                } else {
                    host.places = places.with(city)
                    host.home()
                }
            }
        }
    }

    private companion object {
        const val UNDO_MS = 6000L
    }
}

/**
 * The cities to choose from, as a grid under the headings of their regions. Picking one saves
 * it, makes it the current one and returns to the list of the user's cities.
 */
class CityPage(context: Context, host: Host) : Page(context, host) {
    override val title = "Добавить город"
    private val tops = FloatArray(Places.ALL.size)
    private val columns = IntArray(Places.ALL.size)
    private val headTops = FloatArray(Places.REGIONS.size)
    private var total = 0f
    private val offsets = System.currentTimeMillis().let { now -> Places.ALL.map { offsetLabel(it.zone, now) } }
    private val added = BooleanArray(Places.ALL.size)
    private val cellHeight get() = h * 0.125f
    private val headHeight get() = h * 0.085f
    private val cellWidth get() = (w - pad * 2 - gap * 0.5f * (COLUMNS - 1)) / COLUMNS

    override fun summary() = "${Places.ALL.size} городов по регионам: " + Places.REGIONS.joinToString(", ") { it.name }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        val saved = host.places.saved
        for (i in added.indices) added[i] = saved.any { it.id == Places.ALL[i].id }
    }

    override fun arrange() {
        var y = 0f
        var n = 0
        for ((r, region) in Places.REGIONS.withIndex()) {
            headTops[r] = y
            y += headHeight
            for (k in region.cities.indices) {
                tops[n] = y + (k / COLUMNS) * cellHeight
                columns[n] = k % COLUMNS
                n++
            }
            y += (region.cities.size + COLUMNS - 1) / COLUMNS * cellHeight
        }
        total = y
    }

    override fun contentHeight(pane: Int) = if (pane == 0) total else 0f

    override fun drawBody(canvas: Canvas) {
        val ch = cellHeight
        for (r in headTops.indices) {
            val top = bodyTop + headTops[r]
            if (top + headHeight < bodyTop + scroll || top > scroll + h) continue
            heading(canvas, Places.REGIONS[r].name.uppercase(Locale("ru")), top + headHeight * 0.72f)
        }
        for (i in tops.indices) {
            val top = bodyTop + tops[i]
            if (top + ch < bodyTop + scroll || top > scroll + h) continue
            val left = pad + (cellWidth + gap * 0.5f) * columns[i]
            box.set(left, top + ch * 0.06f, left + cellWidth, top + ch * 0.94f)
            card(canvas, ch * 0.2f, if (added[i]) PanelView.TEAL else PanelView.BORDER)
            val inset = ch * 0.26f
            text.typeface = regular
            text.textSize = ch * 0.22f
            text.color = PanelView.MUTED
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(offsets[i], box.right - inset, box.centerY() + ch * 0.08f, text)
            val room = box.width() - inset * 2.5f - text.measureText(offsets[i])
            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.textSize = ch * 0.27f
            text.color = if (added[i]) PanelView.TEAL else Color.WHITE
            canvas.drawText(text.fit(Places.ALL[i].name, room), box.left + inset, box.centerY() + ch * 0.09f, text)
        }
    }

    override fun onTap(x: Float, y: Float) {
        if (y < bodyTop) return
        val column = min(COLUMNS - 1, ((x - pad) / (cellWidth + gap * 0.5f)).toInt())
        val at = y - bodyTop + scroll
        val i = tops.indices.firstOrNull { columns[it] == column && at >= tops[it] && at < tops[it] + cellHeight } ?: return
        host.places = host.places.with(Places.ALL[i])
        // Back to the user's cities, where the next one can be added at once.
        host.back()
    }

    private companion object {
        const val COLUMNS = 3
    }
}

/**
 * Every block the main screen can show, grouped by its part of the screen. A row is dragged by
 * its grip to change the order within the group; a tap anywhere else on it shows or hides the block.
 */
class BlocksPage(context: Context, host: Host) : Page(context, host) {
    override val title = "Блоки главного экрана"
    // The blocks in the order of the screen: zone by zone, each in the user's order.
    private val list = ArrayList<Block>()
    private val tops = FloatArray(Block.values().size)
    private val headTops = FloatArray(Zone.values().size)
    private var total = 0f
    private var dragging = -1
    private var dragY = 0f
    private var grabOffset = 0f

    // Never lower than a finger needs (48dp), whatever the screen.
    private val rowHeight get() = max(h * 0.12f, 48f * resources.displayMetrics.density)
    private val headHeight get() = rowHeight * 0.62f
    private val gripEnd get() = pad + rowHeight * 1.05f

    override fun summary() = list.joinToString(", ") { "${it.title} — ${if (it in host.setup.hidden) "скрыт" else "показан"}" }

    override fun arrange() {
        list.clear()
        var y = 0f
        for (zone in Zone.values()) {
            headTops[zone.ordinal] = y
            y += headHeight
            for (block in host.setup.order) {
                if (block.zone != zone) continue
                tops[list.size] = y
                list.add(block)
                y += rowHeight
            }
        }
        total = y
        announce()
    }

    override fun contentHeight(pane: Int) = if (pane == 0) total else 0f

    override fun drawHeader(canvas: Canvas) {
        text.color = PanelView.MUTED
        text.textAlign = Paint.Align.RIGHT
        // On a narrow screen the hint gives way to the title.
        text.typeface = bold
        text.textSize = h * 0.058f
        val room = w - pad * 2 - h * 0.065f - text.measureText(title) - gap
        text.typeface = regular
        text.textSize = h * 0.034f
        canvas.drawText(text.fit("Порядок меняется перетаскиванием за значок слева", room), w - pad, pad + h * 0.062f, text)
        text.textAlign = Paint.Align.LEFT
    }

    override fun drawBody(canvas: Canvas) {
        val setup = host.setup
        for (zone in Zone.values()) {
            heading(canvas, zone.title.uppercase(Locale("ru")), bodyTop + headTops[zone.ordinal] + headHeight * 0.74f)
        }
        for (i in list.indices) {
            if (i != dragging) drawRow(canvas, list[i], bodyTop + tops[i], list[i] !in setup.hidden, false)
        }
        if (dragging in list.indices) {
            val block = list[dragging]
            drawRow(canvas, block, bodyTop + liftedTop(), block !in setup.hidden, true)
        }
    }

    private fun drawRow(canvas: Canvas, block: Block, top: Float, on: Boolean, lifted: Boolean) {
        val rh = rowHeight
        box.set(pad, top + rh * 0.06f, w - pad, top + rh * 0.94f)
        val left = box.left
        val right = box.right
        val mid = box.centerY()
        card(canvas, rh * 0.2f, if (lifted) PanelView.TEAL else PanelView.BORDER, if (lifted) 0xFF171A21.toInt() else PanelView.CARD)
        val inset = rh * 0.32f

        // The grip.
        stroke.color = PanelView.MUTED
        stroke.strokeWidth = rh * 0.03f
        stroke.strokeCap = Paint.Cap.ROUND
        for (k in -1..1) canvas.drawLine(left + inset, mid + k * rh * 0.09f, left + inset + rh * 0.26f, mid + k * rh * 0.09f, stroke)

        // The switch.
        val sh = rh * 0.4f
        val sw = sh * 1.9f
        box.set(right - inset - sw, mid - sh / 2, right - inset, mid + sh / 2)
        fill.color = if (on) PanelView.TEAL else PanelView.BORDER
        canvas.drawRoundRect(box, sh / 2, sh / 2, fill)
        fill.color = if (on) PanelView.BACKGROUND else PanelView.MUTED
        canvas.drawCircle(if (on) box.right - sh / 2 else box.left + sh / 2, mid, sh * 0.36f, fill)

        val x = left + inset + rh * 0.6f
        val room = right - inset - sw - rh * 0.3f - x
        text.textAlign = Paint.Align.LEFT
        text.typeface = bold
        text.textSize = rh * 0.28f
        text.color = if (on) Color.WHITE else PanelView.DIM_WHITE
        canvas.drawText(block.title, x, mid + rh * 0.1f, text)
        val titleWidth = text.measureText(block.title)
        text.typeface = regular
        text.textSize = rh * 0.22f
        text.color = PanelView.MUTED
        canvas.drawText(text.fit(block.note, room - titleWidth - rh * 0.3f), x + titleWidth + rh * 0.3f, mid + rh * 0.09f, text)
    }

    /** The row whose slot holds the screen point [y]; -1 between and outside rows. */
    private fun rowAt(y: Float): Int {
        val at = y - bodyTop + scroll
        for (i in list.indices) if (at >= tops[i] && at < tops[i] + rowHeight) return i
        return -1
    }

    private fun first(zone: Zone) = list.indexOfFirst { it.zone == zone }

    private fun last(zone: Zone) = list.indexOfLast { it.zone == zone }

    /** Where the dragged row is drawn: under the finger, but never outside its own group. */
    private fun liftedTop(): Float {
        val zone = list[dragging].zone
        return (dragY - grabOffset).coerceIn(tops[first(zone)], tops[last(zone)])
    }

    override fun onGrab(x: Float, y: Float): Boolean {
        val i = rowAt(y)
        if (i < 0 || x > gripEnd) return false
        dragging = i
        dragY = y - bodyTop + scroll
        grabOffset = dragY - tops[i]
        invalidate()
        return true
    }

    override fun onDrag(x: Float, y: Float) {
        if (dragging !in list.indices) return
        // Near the edge of the screen the list moves under the finger.
        if (y < bodyTop + rowHeight * 0.4f) scrollTo(0, scroll - rowHeight * 0.15f)
        if (y > h - rowHeight * 0.4f) scrollTo(0, scroll + rowHeight * 0.15f)
        dragY = y - bodyTop + scroll
        // The row moves to the slot its middle is over, within the blocks of its part of the screen.
        val zone = list[dragging].zone
        val start = first(zone)
        val target = (start + ((liftedTop() - tops[start] + rowHeight / 2) / rowHeight).toInt()).coerceIn(start, last(zone))
        if (target != dragging) {
            host.setup = host.setup.moved(list[dragging], list[target])
            arrange()
            dragging = target
        }
        invalidate()
    }

    override fun onDrop() {
        dragging = -1
        invalidate()
    }

    override fun onTap(x: Float, y: Float) {
        val i = rowAt(y)
        if (y < bodyTop || i < 0 || x <= gripEnd) return
        host.setup = host.setup.toggled(list[i])
        announce()
        invalidate()
    }
}
