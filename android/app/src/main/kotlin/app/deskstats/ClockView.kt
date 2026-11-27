package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

class Layout(val w: Float, val h: Float, setup: BlockSetup) {
    val portrait = h > w
    val u = min(w, h)
    val pad = u * 0.04f
    val gap = u * 0.028f
    val barHeight = u * 0.05f
    val gear = RectF(w - pad - barHeight, pad, w - pad, pad + barHeight)

    val cards: List<Block> = setup.shown(Block.CPU, Block.GPU, Block.RAM, Block.NET, Block.DISK)
    val cardRects: List<RectF>
    val clock: RectF?
    val weather: RectF?
    val alerts: RectF?
    val batteries: RectF?

    val clockCentred: Boolean
    val weatherCentred: Boolean
    val batteriesAlone: Boolean
    val empty: Boolean

    val clockSize: Float
    val secondsSize: Float
    val clockLeft: Float
    val clockBaseline: Float
    val dateBaseline: Float
    val weatherSize: Float
    val digits: RectF?

    init {
        val top = pad + barHeight + gap * 0.7f
        val left = setup.shown(Block.CLOCK, Block.WEATHER)
        val right = setup.shown(Block.ALERTS, Block.BATTERIES)
        empty = left.isEmpty() && right.isEmpty() && cards.isEmpty()

        if (portrait) {
            val sections = ArrayList<Any>()
            sections.addAll(left)
            sections.addAll(right)
            if (cards.isNotEmpty()) sections.add(cards)
            val cardRows = if (cards.isEmpty()) 1 else (cards.size + perRowOf(cards.size) - 1) / perRowOf(cards.size)
            fun base(s: Any): Float = when (s) {
                Block.CLOCK -> h * 0.175f
                Block.WEATHER -> h * 0.095f
                Block.BATTERIES -> h * 0.065f
                Block.ALERTS -> 0f
                else -> h * 0.14f * cardRows + gap * (cardRows - 1)
            }
            val avail = h - pad - top - gap * (sections.size - 1).coerceAtLeast(0)
            val fixed = sections.sumOf { base(it).toDouble() }.toFloat()
            val flexible = Block.ALERTS in right
            val scale = if (flexible || fixed <= 0f) 1f else min(avail / fixed, 2.6f)
            val alertsHeight = if (flexible) max(avail - fixed, h * 0.12f) else 0f
            val used = if (flexible) avail else fixed * scale
            var y = top + (avail - used) / 2
            val rects = HashMap<Any, RectF>()
            for (s in sections) {
                val height = if (s == Block.ALERTS) alertsHeight else base(s) * scale
                rects[s] = RectF(pad, y, w - pad, y + height)
                y += height + gap
            }
            clock = rects[Block.CLOCK]
            weather = rects[Block.WEATHER]
            alerts = rects[Block.ALERTS]
            batteries = rects[Block.BATTERIES]
            val row = rects[cards]
            cardRects = if (row == null) emptyList() else {
                val perRow = perRowOf(cards.size)
                val cardHeight = (row.height() - gap * (cardRows - 1)) / cardRows
                cards.indices.map { i ->
                    val r = i / perRow
                    val inRow = if (r == cardRows - 1) cards.size - perRow * r else perRow
                    val c = i - r * perRow
                    val cw = (row.width() - gap * (inRow - 1)) / inRow
                    val x = row.left + (cw + gap) * c
                    val yy = row.top + (cardHeight + gap) * r
                    RectF(x, yy, x + cw, yy + cardHeight)
                }
            }
            weatherSize = (weather?.height() ?: 0f) * 0.5f
            clockCentred = true
            weatherCentred = true
            batteriesAlone = batteries != null && batteries.height() > h * 0.12f
        } else {
            val rowHeight = if (left.isEmpty() && right.isEmpty()) h - pad - top else h * 0.29f
            val rowTop = h - pad - rowHeight
            val bottom = if (cards.isEmpty()) h - pad else rowTop - gap
            val cardWidth = if (cards.isEmpty()) 0f else (w - pad * 2 - gap * (cards.size - 1)) / cards.size
            cardRects = cards.indices.map { i -> RectF(pad + (cardWidth + gap) * i, rowTop, pad + (cardWidth + gap) * i + cardWidth, rowTop + rowHeight) }

            val height = bottom - top
            val split = pad + w * 0.45f
            val rightStart = if (left.isEmpty()) pad else split
            val wide = right.isEmpty()

            if (wide && left.size == 2) {
                val cut = pad + (w - pad * 2) * (if (left[0] == Block.CLOCK) WIDE_CLOCK else 1f - WIDE_CLOCK)
                val first = RectF(pad, top, cut - gap, bottom)
                val second = RectF(cut + gap, top, w - pad, bottom)
                clock = if (left[0] == Block.CLOCK) first else second
                weather = if (left[0] == Block.CLOCK) second else first
                weatherSize = height * 0.24f
            } else if (wide) {
                clock = if (Block.CLOCK in left) RectF(pad, top, w - pad, bottom) else null
                weather = if (Block.WEATHER in left) RectF(pad, top, w - pad, bottom) else null
                weatherSize = height * 0.3f
            } else {
                fun place(block: Block, size: Float): RectF? = when (left.indexOf(block)) {
                    -1 -> null
                    0 -> if (left.size == 1) RectF(pad, top + (height - size) / 2, split - gap, top + (height + size) / 2) else RectF(pad, top, split - gap, top + size)
                    else -> RectF(pad, bottom - size, split - gap, bottom)
                }
                clock = place(Block.CLOCK, height * 0.62f)
                weather = place(Block.WEATHER, height * 0.36f)
                weatherSize = height * 0.36f * 0.52f
            }
            clockCentred = wide && left.size == 1
            weatherCentred = wide && left.size == 1

            batteriesAlone = right.size == 1 && right[0] == Block.BATTERIES
            val batteryHeight = height * 0.2f
            batteries = when {
                Block.BATTERIES !in right -> null
                batteriesAlone -> RectF(rightStart, top, w - pad, bottom)
                right[0] == Block.BATTERIES -> RectF(rightStart, top, w - pad, top + batteryHeight)
                else -> RectF(rightStart, bottom - batteryHeight, w - pad, bottom)
            }
            alerts = when {
                Block.ALERTS !in right -> null
                batteries == null -> RectF(rightStart, top, w - pad, bottom)
                right[0] == Block.ALERTS -> RectF(rightStart, top, w - pad, batteries.top - gap)
                else -> RectF(rightStart, batteries.bottom + gap, w - pad, bottom)
            }
        }

        val room = clock?.width() ?: 0f
        val blockHeight = clock?.height() ?: 0f
        val probe = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = LIGHT }
        val base = blockHeight * 0.64f
        probe.textSize = base
        val clockWidth = probe.measureText("00:00")
        probe.textSize = base * SECONDS
        val secondsWidth = probe.measureText("00") + base * 0.05f
        val scale = if (clockWidth + secondsWidth > 0f) min(1f, room * 0.98f / (clockWidth + secondsWidth)) else 1f
        clockSize = base * scale
        secondsSize = clockSize * SECONDS
        val total = (clockWidth + secondsWidth) * scale
        clockLeft = if (clock == null) 0f else if (clockCentred) clock.centerX() - total / 2 else clock.left
        val group = clockSize * 1.2f
        clockBaseline = (clock?.top ?: 0f) + (blockHeight - group) / 2 + clockSize * 0.8f
        dateBaseline = clockBaseline + clockSize * 0.33f
        digits = if (clock == null) null else RectF(
            floor(clockLeft - clockSize * 0.12f), floor(clockBaseline - clockSize * 0.9f),
            ceil(clockLeft + total + clockSize * 0.14f), ceil(clockBaseline + clockSize * 0.14f),
        )
    }

    companion object {
        private const val SECONDS = 0.2125f
        private const val WIDE_CLOCK = 0.56f
        val LIGHT: Typeface = Typeface.create("sans-serif-light", Typeface.NORMAL) ?: Typeface.DEFAULT
    }
}

class ClockView(context: Context) : View(context) {
    private val text = Paint(Paint.ANTI_ALIAS_FLAG)
    private val timeFormat = SimpleDateFormat("HH:mm", Locale.US)
    private val secondsFormat = SimpleDateFormat("ss", Locale.US)
    private val now = Date()
    private var layout: Layout? = null
    private var originX = 0f
    private var originY = 0f

    var zone: TimeZone = TimeZone.getDefault()
        set(value) {
            field = value
            timeFormat.timeZone = value
            invalidate()
        }

    var link = Link.WAITING
        set(value) {
            if (field != value) {
                field = value
                invalidate()
            }
        }

    private val tick = object : Runnable {
        override fun run() {
            invalidate()
            postDelayed(this, 1000L - System.currentTimeMillis() % 1000L)
        }
    }

    fun place(l: Layout) {
        layout = l
        val r = l.digits
        originX = r?.left ?: 0f
        originY = r?.top ?: 0f
        // Old FrameLayouts honour margins only together with a gravity.
        layoutParams = FrameLayout.LayoutParams(r?.width()?.toInt() ?: 0, r?.height()?.toInt() ?: 0, Gravity.TOP).apply {
            leftMargin = originX.toInt()
            topMargin = originY.toInt()
        }
        invalidate()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        post(tick)
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(tick)
        super.onDetachedFromWindow()
    }

    override fun onDraw(canvas: Canvas) {
        val l = layout ?: return
        if (l.digits == null) return
        val p = Ui.p
        now.time = System.currentTimeMillis()
        val away = link == Link.OFFLINE || link == Link.NO_TOKEN || link == Link.REFUSED
        canvas.translate(-originX, -originY)
        text.typeface = Layout.LIGHT
        text.textAlign = Paint.Align.LEFT
        text.textSize = l.clockSize
        text.color = if (away) p.dim else p.clock
        Ui.glow(text, text.color, if (away) 0f else l.clockSize * 0.07f)
        val time = timeFormat.format(now)
        canvas.drawText(time, l.clockLeft - l.clockSize * 0.04f, l.clockBaseline, text)
        val width = text.measureText(time)
        text.textSize = l.secondsSize
        text.color = p.seconds
        Ui.glow(text, text.color, if (away) 0f else l.secondsSize * 0.25f)
        canvas.drawText(secondsFormat.format(now), l.clockLeft + width + l.clockSize * 0.05f, l.clockBaseline - l.clockSize * 0.5f, text)
        Ui.noGlow(text)
    }
}

/** Cards in one row when the screen stands upright: two, or three when there are five or six. */
private fun perRowOf(cards: Int): Int = when {
    cards > 4 -> 3
    cards > 2 -> 2
    else -> maxOf(cards, 1)
}
