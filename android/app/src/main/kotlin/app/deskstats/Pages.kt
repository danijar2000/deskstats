package app.deskstats

import android.content.Context
import android.graphics.Canvas
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

abstract class Page(context: Context, protected val host: Host) : View(context) {
    interface Host {
        fun back()
        fun home()
        fun open(page: Page)
        var places: Places
        var setup: BlockSetup
        var options: Options
        val servers: Servers
        fun changeServers(change: (Servers) -> Servers)
    }

    protected abstract val title: String
    protected val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    protected val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    protected val text = Paint(Paint.ANTI_ALIAS_FLAG)
    protected val regular: Typeface = Typeface.DEFAULT
    protected val bold: Typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
    protected val box = RectF()
    protected val toastBox = RectF()
    private val chevron = Path()

    protected var h = 0f
    protected var w = 0f
    protected var u = 0f
    protected val portrait get() = h > w
    protected var pad = 0f
    protected var gap = 0f
    protected var headerBottom = 0f
    protected val bodyTop get() = headerBottom + gap
    protected val bodyHeight get() = h - pad - bodyTop
    protected val titleSize get() = u * 0.058f

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

    protected open fun contentHeight(pane: Int) = 0f

    protected open fun paneAt(x: Float, y: Float) = 0

    protected open fun viewTop(pane: Int) = bodyTop

    protected open fun viewHeight(pane: Int) = bodyHeight

    protected open fun barX(pane: Int) = w - pad * 0.5f

    protected open fun headerExtra() = 0f

    protected open fun drawHeader(canvas: Canvas) {}

    protected abstract fun drawBody(canvas: Canvas)

    protected open fun drawFixed(canvas: Canvas) {}

    protected open fun onTap(x: Float, y: Float) {}

    protected open fun onGrab(x: Float, y: Float) = false

    protected open fun onDrag(x: Float, y: Float) {}

    protected open fun onDrop() {}

    protected open fun arrange() {}

    protected open fun summary() = ""

    protected fun announce() {
        val body = summary()
        contentDescription = if (body.isEmpty()) title else "$title. $body"
    }

    open fun resumed() {
        arrange()
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
        u = min(w, h)
        pad = u * 0.04f
        gap = u * 0.028f
        headerBottom = pad + u * 0.1f
        headerBottom += headerExtra()
        arrange()
    }

    private fun maxScroll(pane: Int) = max(0f, contentHeight(pane) - viewHeight(pane))

    protected fun scrollOf(pane: Int) = scrolls[pane]

    protected fun scrollTo(pane: Int, value: Float) {
        scrolls[pane] = value.coerceIn(0f, maxScroll(pane))
    }

    protected open fun bodyBottom() = h

    override fun onDraw(canvas: Canvas) {
        val p = Ui.p
        for (i in 0 until PANES) scrolls[i] = scrolls[i].coerceIn(0f, maxScroll(i))
        val mid = pad + u * 0.05f
        val arm = u * 0.022f
        chevron.rewind()
        chevron.moveTo(pad + arm * 1.4f, mid - arm)
        chevron.lineTo(pad + arm * 0.4f, mid)
        chevron.lineTo(pad + arm * 1.4f, mid + arm)
        stroke.color = p.muted
        stroke.strokeWidth = u * 0.007f
        stroke.strokeCap = Paint.Cap.ROUND
        stroke.strokeJoin = Paint.Join.ROUND
        canvas.drawPath(chevron, stroke)
        text.typeface = bold
        text.textAlign = Paint.Align.LEFT
        text.textSize = titleSize
        text.color = p.text
        Ui.glow(text, p.text, u * 0.012f)
        canvas.drawText(text.fit(title, w - pad * 2 - u * 0.065f), pad + u * 0.065f, mid + u * 0.021f, text)
        Ui.noGlow(text)
        drawHeader(canvas)

        canvas.save()
        canvas.clipRect(0f, bodyTop, w, bodyBottom())
        canvas.translate(0f, -scroll)
        drawBody(canvas)
        canvas.restore()
        toastBox.setEmpty()
        drawFixed(canvas)

        fill.shader = null
        fill.color = p.scrollMark
        for (i in 0 until PANES) {
            val range = maxScroll(i)
            if (range <= 0f) continue
            val view = viewHeight(i)
            val length = max(u * 0.06f, view * view / contentHeight(i))
            val top = viewTop(i) + (view - length) * scrolls[i] / range
            val half = u * 0.003f
            box.set(barX(i) - half, top, barX(i) + half, top + length)
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
                pane = if (event.y > bodyTop) paneAt(event.x, event.y) else -1
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
                val slop = u * 0.025f
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
                    !moved && event.y < pad + u * 0.1f && event.x < w * 0.35f -> host.back()
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

    protected fun card(canvas: Canvas, radius: Float, border: Int = Ui.p.border, background: Int = Ui.p.card) {
        fill.shader = null
        fill.color = background
        canvas.drawRoundRect(box, radius, radius, fill)
        stroke.color = border
        stroke.strokeWidth = max(1f, u * 0.002f) * (if (border == Ui.p.border) 1f else 1.5f)
        canvas.drawRoundRect(box, radius, radius, stroke)
    }

    protected fun message(canvas: Canvas, line: String) {
        text.typeface = regular
        text.textAlign = Paint.Align.CENTER
        text.textSize = u * 0.045f
        text.color = Ui.p.muted
        val lines = text.wrap(line, w - pad * 2, 3)
        for (i in lines.indices) canvas.drawText(lines[i], w / 2, bodyTop + bodyHeight * 0.45f + i * text.textSize * 1.35f, text)
        text.textAlign = Paint.Align.LEFT
    }

    protected fun heading(canvas: Canvas, label: String, baseline: Float) {
        text.typeface = bold
        text.textSize = u * 0.03f
        text.spacing(0.14f)
        text.color = Ui.p.muted
        text.textAlign = Paint.Align.LEFT
        canvas.drawText(text.fit(label.heading(), w - pad * 2), pad + u * 0.008f, baseline, text)
        text.spacing(0f)
    }

    protected fun toast(canvas: Canvas, line: String, action: String, color: Int) {
        val p = Ui.p
        val height = u * 0.1f
        val inset = height * 0.4f
        text.typeface = bold
        text.textSize = height * 0.36f
        val actionWidth = if (action.isEmpty()) 0f else text.measureText(action) + inset
        text.typeface = regular
        val shown = text.fit(line, w - pad * 2 - inset * 2 - actionWidth)
        val width = text.measureText(shown) + inset * 2 + actionWidth
        toastBox.set((w - width) / 2, h - pad - height, (w + width) / 2, h - pad)
        box.set(toastBox)
        card(canvas, height / 2, color, p.raised)
        text.textAlign = Paint.Align.LEFT
        text.color = p.soft
        canvas.drawText(shown, box.left + inset, box.centerY() + text.textSize * 0.36f, text)
        if (action.isNotEmpty()) {
            text.typeface = bold
            text.color = color
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(action, box.right - inset, box.centerY() + text.textSize * 0.36f, text)
            text.textAlign = Paint.Align.LEFT
        }
    }

    protected fun switch(canvas: Canvas, right: Float, mid: Float, height: Float, on: Boolean) {
        val p = Ui.p
        val sw = height * 1.9f
        box.set(right - sw, mid - height / 2, right, mid + height / 2)
        fill.shader = null
        fill.color = if (on) p.chipOn else p.switchOff
        canvas.drawRoundRect(box, height / 2, height / 2, fill)
        fill.color = if (on) p.knobOn else p.knobOff
        canvas.drawCircle(if (on) box.right - height / 2 else box.left + height / 2, mid, height * 0.36f, fill)
    }

    protected fun radio(canvas: Canvas, cx: Float, cy: Float, r: Float, on: Boolean) {
        val p = Ui.p
        stroke.color = if (on) p.selectionMark() else p.muted
        stroke.strokeWidth = r * 0.25f
        canvas.drawCircle(cx, cy, r, stroke)
        if (on) {
            fill.color = p.selectionMark()
            canvas.drawCircle(cx, cy, r * 0.55f, fill)
        }
    }

    protected fun forward(canvas: Canvas, x: Float, y: Float, arm: Float) {
        stroke.color = Ui.p.chevron
        stroke.strokeWidth = arm * 0.4f
        stroke.strokeCap = Paint.Cap.ROUND
        stroke.strokeJoin = Paint.Join.ROUND
        canvas.chevron(chevron, stroke, x, y, arm)
    }

    private companion object {
        const val PANES = 2
    }
}

fun Palette.selectionMark() = if (glow) accent else selection

class Proc(val name: String, val cpu: Float, val ram: Long, val down: Float, val up: Float, val gpu: Float, val count: Int)

class ProcsPage(context: Context, host: Host, sort: Block, private val onSort: (Block) -> Unit) : Page(context, host) {
    private class Line(val name: String, val value: String, val parts: String, val share: Float)

    override val title: String = context.getString(R.string.procs_title)
    var sort = sort
        private set
    private var procs: List<Proc>? = null
    private var lines: List<Line> = emptyList()
    private var footer = ""
    private var total = 0
    private var problem: String? = null
    private val tabs = Array(TABS.size) { RectF() }
    private val labels = TABS.map { context.getString(it.title).heading() }
    private var rowHeight = 0f
    private val columns get() = if (portrait) 1 else 2

    var link = Link.ONLINE
        set(value) {
            if (field != value) {
                field = value
                invalidate()
            }
        }

    fun push(sortId: String, list: List<Proc>, total: Int) {
        if (sortId != sort.id) return
        procs = list
        this.total = total
        problem = null
        prepare()
        invalidate()
    }

    fun fail(code: Int) {
        problem = if (code == 0) context.getString(R.string.procs_no_answer) else context.getString(R.string.procs_refused, code)
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
        val list = if (sort == Block.NET || sort == Block.GPU) all.filter { key(it) > 0f } else all
        val top = list.maxOfOrNull { key(it) } ?: 0f
        val cpu = context.getString(R.string.cpu)
        val ram = context.getString(R.string.ram)
        val gpu = context.getString(R.string.gpu)
        val net = context.getString(R.string.procs_net)
        val count = context.getString(R.string.procs_count)
        val cores = context.getString(R.string.procs_cores)
        lines = list.map { p ->
            val value = when (sort) {
                Block.RAM -> Fmt.size(p.ram)
                Block.NET -> Fmt.rate(p.down + p.up)
                Block.GPU -> Fmt.percent(p.gpu)
                else -> Fmt.percent(p.cpu)
            }
            val parts = ArrayList<String>(5)
            if (sort == Block.NET) parts.add("↓ ${Fmt.rate(p.down)}  ↑ ${Fmt.rate(p.up)}")
            if (sort != Block.CPU) parts.add("$cpu " + Fmt.percent(p.cpu))
            if (sort != Block.RAM) parts.add("$ram " + Fmt.size(p.ram))
            if (sort != Block.GPU && p.gpu >= 0.1f) parts.add("$gpu " + Fmt.percent(p.gpu))
            if (sort != Block.NET && p.down + p.up >= 1024f) parts.add(String.format(net, Fmt.rate(p.down + p.up)))
            if (p.count > 1) parts.add(String.format(count, p.count))
            // The Mac counts CPU per core, as Activity Monitor does.
            if (sort == Block.CPU && p.cpu >= 100f) parts.add(0, String.format(cores, Fmt.number("%.1f", p.cpu / 100f)))
            Line(p.name, value, parts.joinToString(" · "), if (top > 0f) (key(p) / top).coerceIn(0f, 1f) else 0f)
        }
        footer = if (list === all && total > all.size) context.getString(R.string.procs_shown, all.size, total) else ""
        announce()
    }

    override fun summary(): String {
        val what = labels[TABS.indexOf(sort)]
        problem?.let { return it }
        if (procs == null) return context.getString(R.string.procs_a11y_loading, what)
        return context.getString(R.string.procs_a11y, what, lines.take(5).joinToString(", ") { "${it.name} ${it.value}" })
    }

    override fun headerExtra() = if (portrait) u * 0.1f else 0f

    override fun arrange() {
        val rows = max(1, (bodyHeight / (u * (if (portrait) 0.135f else 0.118f))).toInt())
        rowHeight = bodyHeight / rows
        val between = gap * 0.5f
        val tabHeight = u * 0.075f
        if (portrait) {
            val tabWidth = (w - pad * 2 - between * (TABS.size - 1)) / TABS.size
            val top = pad + u * 0.1f + u * 0.01f
            for (i in TABS.indices) {
                val left = pad + i * (tabWidth + between)
                tabs[i].set(left, top, left + tabWidth, top + tabHeight)
            }
            return
        }
        text.typeface = bold
        text.textSize = titleSize
        val titleEnd = pad + u * 0.065f + text.measureText(title) + gap
        val tabWidth = min(u * 0.19f, (w - pad - titleEnd - between * (TABS.size - 1)) / TABS.size)
        val top = pad + u * 0.05f - tabHeight / 2
        for (i in TABS.indices) {
            val right = w - pad - (TABS.size - 1 - i) * (tabWidth + between)
            tabs[i].set(right - tabWidth, top, right, top + tabHeight)
        }
    }

    override fun contentHeight(pane: Int) = if (pane != 0) 0f else (lines.size + columns - 1) / columns * rowHeight + if (footer.isEmpty()) 0f else rowHeight * 0.6f

    override fun drawHeader(canvas: Canvas) {
        val p = Ui.p
        for (i in TABS.indices) {
            val block = TABS[i]
            val active = block == sort
            box.set(tabs[i])
            card(canvas, box.height() / 2, if (active) color(block) else p.border, if (active) p.card else p.background)
            text.typeface = bold
            text.textAlign = Paint.Align.CENTER
            text.textSize = min(box.height() * 0.42f, box.width() * 0.2f)
            text.spacing(0.1f)
            val label = text.measureText(labels[i])
            if (label > box.width() * 0.84f) text.textSize *= box.width() * 0.84f / label
            text.color = if (active) color(block) else p.muted
            if (active) Ui.glow(text, text.color, box.height() * 0.15f)
            canvas.drawText(text.fit(labels[i], box.width() * 0.9f), box.centerX(), box.centerY() + text.textSize * 0.36f, text)
            Ui.noGlow(text)
            text.spacing(0f)
        }
        text.textAlign = Paint.Align.LEFT
    }

    override fun drawBody(canvas: Canvas) {
        if (procs == null || lines.isEmpty()) return
        val p = Ui.p
        val stale = link != Link.ONLINE || problem != null
        val n = columns
        val columnWidth = (w - pad * 2 - gap * (n - 1)) / n
        val accent = color(sort)
        for (i in lines.indices) {
            val line = lines[i]
            val left = pad + (columnWidth + gap) * (i % n)
            val y = bodyTop + rowHeight * (i / n)
            if (y + rowHeight < bodyTop + scroll || y > scroll + h) continue
            box.set(left, y + rowHeight * 0.06f, left + columnWidth, y + rowHeight * 0.94f)
            val rh = box.height()
            card(canvas, rh * 0.22f)
            val inset = rh * 0.26f
            val idleRow = line.share <= 0f

            text.typeface = bold
            text.textSize = rh * 0.3f
            text.textAlign = Paint.Align.RIGHT
            text.color = if (stale || idleRow) p.dim else accent
            canvas.drawText(line.value, box.right - inset, box.top + rh * 0.42f, text)
            val valueWidth = text.measureText(line.value)

            text.textAlign = Paint.Align.LEFT
            text.color = if (stale) p.dim else p.text
            canvas.drawText(text.fit(line.name, box.width() - inset * 2 - valueWidth - rh * 0.2f), box.left + inset, box.top + rh * 0.42f, text)

            text.typeface = regular
            text.textSize = rh * 0.21f
            text.color = p.muted
            canvas.drawText(text.fit(line.parts, box.width() - inset * 2), box.left + inset, box.top + rh * 0.72f, text)

            val barTop = box.top + rh * 0.84f
            val barHeight = rh * 0.05f
            val barWidth = box.width() - inset * 2
            fill.color = p.grid
            box.set(left + inset, barTop, left + inset + barWidth, barTop + barHeight)
            canvas.drawRoundRect(box, barHeight / 2, barHeight / 2, fill)
            if (!idleRow) {
                fill.color = if (stale) p.dim else accent
                box.right = box.left + max(barHeight, barWidth * line.share)
                canvas.drawRoundRect(box, barHeight / 2, barHeight / 2, fill)
            }
        }
        if (footer.isNotEmpty()) {
            text.typeface = regular
            text.textSize = u * 0.032f
            text.color = p.dim
            text.textAlign = Paint.Align.CENTER
            canvas.drawText(footer, w / 2, bodyTop + (lines.size + n - 1) / n * rowHeight + rowHeight * 0.4f, text)
            text.textAlign = Paint.Align.LEFT
        }
    }

    override fun drawFixed(canvas: Canvas) {
        val trouble = problem
        val away = link == Link.OFFLINE || link == Link.NO_TOKEN || link == Link.REFUSED
        val offline = context.getString(R.string.no_connection)
        when {
            procs == null -> message(canvas, trouble ?: if (away) offline else context.getString(R.string.procs_loading))
            trouble != null -> toast(canvas, trouble, "", Ui.p.bad)
            away -> toast(canvas, offline, "", Ui.p.bad)
            lines.isEmpty() -> message(
                canvas,
                context.getString(if (sort == Block.NET) R.string.procs_net_idle else if (sort == Block.GPU) R.string.procs_gpu_idle else R.string.no_data),
            )
        }
    }

    override fun onTap(x: Float, y: Float) {
        val i = tabs.indexOfFirst { x >= it.left - gap * 0.25f && x <= it.right + gap * 0.25f && y >= it.top - gap && y <= it.bottom + gap }
        if (i < 0 || TABS[i] == sort) return
        sort = TABS[i]
        procs = procs?.sortedByDescending { key(it) }
        total = 0
        prepare()
        scrollTo(0, 0f)
        onSort(sort)
        invalidate()
    }

    private fun color(block: Block) = Ui.p.metrics[TABS.indexOf(block)]

    private companion object {
        val TABS = arrayOf(Block.CPU, Block.GPU, Block.RAM, Block.NET)
    }
}

class AlertsPage(context: Context, host: Host) : Page(context, host) {
    override val title: String = context.getString(R.string.block_alerts)
    private var alerts: List<Alert> = emptyList()
    private var times: List<String> = emptyList()
    private var day = 0L
    private var empty = context.getString(R.string.alerts_page_waiting)
    private var selected: String? = null
    private val stamps = Stamps(context).apply { zone = host.places.active?.zone ?: TimeZone.getDefault() }

    private val detail = ArrayList<String>()
    private var detailTitleLines = 0

    private val listWidth get() = if (portrait) w - pad * 2 else w * 0.4f
    private val listHeight get() = if (portrait) (bodyHeight * 0.42f / rowHeight).toInt().coerceAtLeast(1) * rowHeight else bodyHeight
    private val rowHeight get() = u * 0.15f
    private val inset get() = u * 0.04f
    private val detailLeft get() = if (portrait) pad else pad + listWidth + gap
    private val detailBoxTop get() = if (portrait) bodyTop + listHeight + gap * 0.5f else bodyTop + rowHeight * 0.05f
    private val detailTop get() = detailBoxTop + inset + u * 0.07f

    fun push(stats: Stats) {
        val list = if (host.options.alerts) stats.alerts.orEmpty() else emptyList()
        val now = System.currentTimeMillis()
        val note = context.getString(
            when {
                !host.options.alerts -> R.string.alerts_off
                stats.alertsDenied -> R.string.alerts_page_denied
                stats.alerts == null -> R.string.alerts_server_off
                else -> R.string.alerts_page_none
            },
        )
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
        return context.getString(R.string.alerts_a11y, alerts.size, alert.app, alert.title, alert.body)
    }

    override fun arrange() = prepareDetail()

    private fun prepareDetail() {
        detail.clear()
        detailTitleLines = 0
        val alert = chosen() ?: return
        if (w <= 0f) return
        val width = w - pad - detailLeft - inset * 2
        text.typeface = bold
        text.textSize = u * TITLE_SIZE
        detail.addAll(text.wrap(alert.title, width, 4))
        detailTitleLines = detail.size
        text.typeface = regular
        text.textSize = u * BODY_SIZE
        for (paragraph in alert.body.split('\n')) detail.addAll(text.wrap(paragraph, width, 200))
    }

    override fun contentHeight(pane: Int) = when (pane) {
        0 -> alerts.size * rowHeight
        else -> detailTitleLines * u * TITLE_SIZE * 1.3f + u * 0.02f + (detail.size - detailTitleLines) * u * BODY_SIZE * 1.4f + u * 0.02f
    }

    override fun paneAt(x: Float, y: Float) =
        if (portrait) (if (y < bodyTop + listHeight) 0 else 1) else if (x < pad + listWidth + gap * 0.5f) 0 else 1

    override fun viewTop(pane: Int) = if (pane == 0) bodyTop else detailTop

    override fun viewHeight(pane: Int) = if (pane == 0) listHeight else h - pad - inset * 0.6f - detailTop

    override fun barX(pane: Int) = if (pane == 0 && !portrait) pad + listWidth + gap * 0.5f else if (pane == 0) w - pad * 0.5f else w - pad - inset * 0.45f

    override fun bodyBottom() = if (portrait) bodyTop + listHeight else h

    override fun drawBody(canvas: Canvas) {
        val p = Ui.p
        val rh = rowHeight
        for (i in alerts.indices) {
            val alert = alerts[i]
            val top = bodyTop + rh * i
            if (top + rh < bodyTop + scroll || top > bodyTop + scroll + bodyHeight + pad) continue
            box.set(pad, top + rh * 0.05f, pad + listWidth, top + rh * 0.95f)
            val active = alert.key == selected
            card(canvas, rh * 0.18f, if (active) p.selectionMark() else p.border, if (active) p.card else p.background)
            val inset = rh * 0.2f
            text.typeface = regular
            text.textSize = rh * 0.2f
            text.color = p.muted
            text.textAlign = Paint.Align.RIGHT
            val time = times[i]
            canvas.drawText(time, box.right - inset, box.top + rh * 0.34f, text)
            val timeWidth = text.measureText(time)
            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.color = p.accent
            canvas.drawText(text.fit(alert.app, box.width() - inset * 2.5f - timeWidth), box.left + inset, box.top + rh * 0.34f, text)
            text.textSize = rh * 0.23f
            text.color = p.text
            canvas.drawText(text.fit(alert.title, box.width() - inset * 2), box.left + inset, box.top + rh * 0.68f, text)
        }
    }

    override fun drawFixed(canvas: Canvas) {
        val p = Ui.p
        if (alerts.isEmpty()) {
            message(canvas, empty)
            return
        }
        val index = alerts.indexOfFirst { it.key == selected }
        if (index < 0) return
        val alert = alerts[index]
        box.set(detailLeft, detailBoxTop, w - pad, h - pad)
        val left = box.left
        val top = box.top
        val right = box.right
        card(canvas, u * 0.03f)
        val width = right - left - inset * 2

        text.typeface = regular
        text.textSize = u * 0.036f
        text.color = p.muted
        text.textAlign = Paint.Align.RIGHT
        val time = times[index]
        canvas.drawText(time, right - inset, top + inset + u * 0.036f, text)
        val timeWidth = text.measureText(time)
        text.textAlign = Paint.Align.LEFT
        text.typeface = bold
        text.textSize = u * 0.042f
        text.color = p.accent
        canvas.drawText(text.fit(alert.app, width - timeWidth - inset), left + inset, top + inset + u * 0.036f, text)

        canvas.save()
        canvas.clipRect(left, detailTop, right, detailTop + viewHeight(1))
        var y = detailTop - scrollOf(1)
        text.textSize = u * TITLE_SIZE
        text.color = p.text
        for (i in detail.indices) {
            if (i == detailTitleLines) {
                y += u * 0.02f
                text.typeface = regular
                text.textSize = u * BODY_SIZE
                text.color = p.soft
            }
            y += text.textSize * (if (i < detailTitleLines) 1.3f else 1.4f)
            if (y > detailTop - text.textSize && y < h) canvas.drawText(detail[i], left + inset, y, text)
        }
        canvas.restore()
    }

    override fun onTap(x: Float, y: Float) {
        if (x > pad + listWidth || y < bodyTop || y > bodyTop + listHeight) return
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

private fun offsetLabel(zone: TimeZone, at: Long): String {
    val minutes = zone.getOffset(at) / 60_000
    val sign = if (minutes < 0) "−" else "+"
    val rest = abs(minutes) % 60
    return "UTC$sign${abs(minutes) / 60}" + if (rest != 0) String.format(Locale.US, ":%02d", rest) else ""
}

class PlacesPage(context: Context, host: Host) : Page(context, host) {
    override val title: String = context.getString(R.string.places_title)
    private val timeFormat = SimpleDateFormat("HH:mm", Locale.US)
    private val dateFormat = SimpleDateFormat(context.getString(R.string.fmt_date), Ui.locale)
    private val now = Date()
    private val rowHeight get() = u * 0.16f

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
        val none = context.getString(R.string.none)
        return context.getString(R.string.places_a11y, places.active?.name ?: none, places.saved.joinToString(", ") { it.name }.ifEmpty { none })
    }

    private val hintHeight get() = if (host.places.saved.isEmpty()) rowHeight * 0.9f else 0f

    override fun contentHeight(pane: Int) = if (pane != 0) 0f else hintHeight + (host.places.saved.size + 1) * rowHeight

    private fun crossX(right: Float) = right - rowHeight * 0.3f - rowHeight * 0.07f

    override fun drawBody(canvas: Canvas) {
        val p = Ui.p
        val places = host.places
        val rh = rowHeight
        now.time = System.currentTimeMillis()
        val origin = bodyTop + hintHeight
        if (places.saved.isEmpty()) {
            text.typeface = regular
            text.textSize = u * 0.04f
            text.color = p.muted
            text.textAlign = Paint.Align.LEFT
            val lines = text.wrap(context.getString(R.string.places_empty), w - pad * 2, 2)
            for (i in lines.indices) canvas.drawText(lines[i], pad + u * 0.008f, bodyTop + rh * 0.35f + i * text.textSize * 1.3f, text)
        }
        for (i in 0..places.saved.size) {
            val top = origin + rh * i
            box.set(pad, top + rh * 0.05f, w - pad, top + rh * 0.95f)
            val inset = rh * 0.3f
            if (i == places.saved.size) {
                card(canvas, rh * 0.2f, p.border, p.background)
                text.typeface = bold
                text.textSize = rh * 0.26f
                text.color = p.accent
                text.textAlign = Paint.Align.CENTER
                canvas.drawText("+  " + context.getString(R.string.add_city), box.centerX(), box.centerY() + rh * 0.09f, text)
                text.textAlign = Paint.Align.LEFT
                continue
            }
            val city = places.saved[i]
            val active = city.id == places.active?.id
            val zone = city.zone
            card(canvas, rh * 0.2f, if (active) p.selectionMark() else p.border)

            val cx = box.left + inset
            radio(canvas, cx, box.centerY(), rh * 0.1f, active)

            var right = box.right - inset
            val r = rh * 0.1f
            val x = crossX(box.right)
            stroke.color = p.muted
            stroke.strokeWidth = rh * 0.022f
            canvas.drawLine(x - r, box.centerY() - r, x + r, box.centerY() + r, stroke)
            canvas.drawLine(x - r, box.centerY() + r, x + r, box.centerY() - r, stroke)
            right -= rh * 0.7f
            timeFormat.timeZone = zone
            dateFormat.timeZone = zone
            text.typeface = Layout.LIGHT
            text.textSize = rh * 0.42f
            text.color = p.text
            text.textAlign = Paint.Align.RIGHT
            val time = timeFormat.format(now)
            canvas.drawText(time, right, box.centerY() + rh * 0.15f, text)
            val room = right - text.measureText(time) - rh * 0.3f - (cx + rh * 0.3f)

            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.textSize = rh * 0.26f
            canvas.drawText(text.fit(city.name, room), cx + rh * 0.3f, box.top + rh * 0.4f, text)
            text.typeface = regular
            text.textSize = rh * 0.19f
            text.color = p.muted
            val note = "${offsetLabel(zone, now.time)} · ${dateFormat.format(now).capitalized()}"
            canvas.drawText(text.fit(note, room), cx + rh * 0.3f, box.top + rh * 0.7f, text)
        }
    }

    override fun drawFixed(canvas: Canvas) {
        removed?.let { toast(canvas, context.getString(R.string.places_removed, it.name), context.getString(R.string.undo), Ui.p.accent) }
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
        val at = y - bodyTop + scroll - hintHeight
        if (at < 0f) return
        val i = (at / rowHeight).toInt()
        when {
            i == places.saved.size -> host.open(CityPage(context, host))
            i in places.saved.indices -> {
                val city = places.saved[i]
                if (x > crossX(w - pad) - rowHeight * 0.17f) {
                    removed = city
                    removedIndex = i
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

class CityPage(context: Context, host: Host) : Page(context, host) {
    override val title: String = context.getString(R.string.add_city)
    private val tops = FloatArray(Places.ALL.size)
    private val columns = IntArray(Places.ALL.size)
    private val headTops = FloatArray(Places.REGIONS.size)
    private val regionNames = Places.REGIONS.map { context.getString(it.name) }
    private var total = 0f
    private val offsets = System.currentTimeMillis().let { now -> Places.ALL.map { offsetLabel(it.zone, now) } }
    private val added = BooleanArray(Places.ALL.size)
    private val count get() = if (portrait) 2 else 3
    private val cellHeight get() = u * 0.125f
    private val headHeight get() = u * 0.085f
    private val cellWidth get() = (w - pad * 2 - gap * 0.5f * (count - 1)) / count

    override fun summary() = context.getString(R.string.city_a11y, Places.ALL.size, regionNames.joinToString(", "))

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        val saved = host.places.saved
        for (i in added.indices) added[i] = saved.any { it.id == Places.ALL[i].id }
    }

    override fun arrange() {
        var y = 0f
        var n = 0
        val c = count
        for ((r, region) in Places.REGIONS.withIndex()) {
            headTops[r] = y
            y += headHeight
            for (k in region.cities.indices) {
                tops[n] = y + (k / c) * cellHeight
                columns[n] = k % c
                n++
            }
            y += (region.cities.size + c - 1) / c * cellHeight
        }
        total = y
    }

    override fun contentHeight(pane: Int) = if (pane == 0) total else 0f

    override fun drawBody(canvas: Canvas) {
        val p = Ui.p
        val ch = cellHeight
        for (r in headTops.indices) {
            val top = bodyTop + headTops[r]
            if (top + headHeight < bodyTop + scroll || top > scroll + h) continue
            heading(canvas, regionNames[r], top + headHeight * 0.72f)
        }
        for (i in tops.indices) {
            val top = bodyTop + tops[i]
            if (top + ch < bodyTop + scroll || top > scroll + h) continue
            val left = pad + (cellWidth + gap * 0.5f) * columns[i]
            box.set(left, top + ch * 0.06f, left + cellWidth, top + ch * 0.94f)
            card(canvas, ch * 0.2f, if (added[i]) p.selectionMark() else p.border)
            val inset = ch * 0.26f
            text.typeface = regular
            text.textSize = ch * 0.22f
            text.color = p.muted
            text.textAlign = Paint.Align.RIGHT
            canvas.drawText(offsets[i], box.right - inset, box.centerY() + ch * 0.08f, text)
            val room = box.width() - inset * 2.5f - text.measureText(offsets[i])
            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.textSize = ch * 0.27f
            text.color = if (added[i]) p.accent else p.text
            canvas.drawText(text.fit(Places.ALL[i].name, room), box.left + inset, box.centerY() + ch * 0.09f, text)
        }
    }

    override fun onTap(x: Float, y: Float) {
        if (y < bodyTop) return
        val column = min(count - 1, ((x - pad) / (cellWidth + gap * 0.5f)).toInt())
        val at = y - bodyTop + scroll
        val i = tops.indices.firstOrNull { columns[it] == column && at >= tops[it] && at < tops[it] + cellHeight } ?: return
        host.places = host.places.with(Places.ALL[i])
        host.back()
    }
}

class BlocksPage(context: Context, host: Host) : Page(context, host) {
    override val title: String = context.getString(R.string.blocks_title)
    private val list = ArrayList<Block>()
    private val tops = FloatArray(Block.values().size)
    private val headTops = FloatArray(Zone.values().size)
    private val titles = Block.values().map { context.getString(it.title) }
    private val notes = Block.values().map { context.getString(it.note) }
    private val hint = context.getString(R.string.blocks_hint)
    private var total = 0f
    private var dragging = -1
    private var dragY = 0f
    private var grabOffset = 0f

    private val rowHeight get() = max(u * (if (portrait) 0.15f else 0.12f), 48f * resources.displayMetrics.density)
    private val headHeight get() = rowHeight * 0.62f
    private val hintHeight get() = if (portrait) rowHeight * 0.5f else 0f
    private val gripEnd get() = pad + rowHeight * 1.05f

    override fun summary(): String {
        val shown = context.getString(R.string.a11y_shown)
        val hidden = context.getString(R.string.a11y_hidden)
        return list.joinToString(", ") { "${titles[it.ordinal]}: ${if (it in host.setup.hidden) hidden else shown}" }
    }

    override fun arrange() {
        list.clear()
        var y = hintHeight
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
        if (portrait) return
        text.color = Ui.p.muted
        text.textAlign = Paint.Align.RIGHT
        text.typeface = bold
        text.textSize = titleSize
        val room = w - pad * 2 - u * 0.065f - text.measureText(title) - gap
        text.typeface = regular
        text.textSize = u * 0.034f
        canvas.drawText(text.fit(hint, room), w - pad, pad + u * 0.062f, text)
        text.textAlign = Paint.Align.LEFT
    }

    override fun drawBody(canvas: Canvas) {
        val setup = host.setup
        if (portrait) {
            text.typeface = regular
            text.textSize = u * 0.034f
            text.color = Ui.p.muted
            text.textAlign = Paint.Align.LEFT
            canvas.drawText(text.fit(hint, w - pad * 2), pad + u * 0.008f, bodyTop + hintHeight * 0.55f, text)
        }
        for (zone in Zone.values()) {
            heading(canvas, context.getString(if (portrait) zone.portrait else zone.landscape), bodyTop + headTops[zone.ordinal] + headHeight * 0.74f)
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
        val p = Ui.p
        val rh = rowHeight
        box.set(pad, top + rh * 0.06f, w - pad, top + rh * 0.94f)
        val left = box.left
        val right = box.right
        val mid = box.centerY()
        card(canvas, rh * 0.2f, if (lifted) p.selectionMark() else p.border, if (lifted) p.raised else p.card)
        val inset = rh * 0.32f

        stroke.color = p.muted
        stroke.strokeWidth = rh * 0.03f
        stroke.strokeCap = Paint.Cap.ROUND
        for (k in -1..1) canvas.drawLine(left + inset, mid + k * rh * 0.09f, left + inset + rh * 0.26f, mid + k * rh * 0.09f, stroke)

        val sh = rh * 0.4f
        switch(canvas, right - inset, mid, sh, on)

        val x = left + inset + rh * 0.6f
        val room = right - inset - sh * 1.9f - rh * 0.3f - x
        val title = titles[block.ordinal]
        val note = notes[block.ordinal]
        text.textAlign = Paint.Align.LEFT
        text.typeface = bold
        text.textSize = rh * (if (portrait) 0.24f else 0.28f)
        text.color = if (on) p.text else p.dim
        if (portrait) {
            canvas.drawText(text.fit(title, room), x, mid - rh * 0.03f, text)
            text.typeface = regular
            text.textSize = rh * 0.19f
            text.color = p.muted
            canvas.drawText(text.fit(note, room), x, mid + rh * 0.23f, text)
            return
        }
        canvas.drawText(text.fit(title, room), x, mid + rh * 0.1f, text)
        val titleWidth = text.measureText(title)
        text.typeface = regular
        text.textSize = rh * 0.22f
        text.color = p.muted
        canvas.drawText(text.fit(note, room - titleWidth - rh * 0.3f), x + titleWidth + rh * 0.3f, mid + rh * 0.09f, text)
    }

    private fun rowAt(y: Float): Int {
        val at = y - bodyTop + scroll
        for (i in list.indices) if (at >= tops[i] && at < tops[i] + rowHeight) return i
        return -1
    }

    private fun first(zone: Zone) = list.indexOfFirst { it.zone == zone }

    private fun last(zone: Zone) = list.indexOfLast { it.zone == zone }

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
        if (y < bodyTop + rowHeight * 0.4f) scrollTo(0, scroll - rowHeight * 0.15f)
        if (y > h - rowHeight * 0.4f) scrollTo(0, scroll + rowHeight * 0.15f)
        dragY = y - bodyTop + scroll
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
