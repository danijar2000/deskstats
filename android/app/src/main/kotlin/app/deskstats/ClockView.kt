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
import kotlin.math.min

/**
 * Where things sit on the screen; shared by the panel and the clock drawn above it.
 *
 * A thin bar on top carries the connection state and the gear. Below it are two zones side by
 * side — left (clock, weather) and right (notifications, batteries) — and a row of cards at the
 * bottom. A hidden block gives its room to its neighbours; an empty zone or row disappears.
 * Without the right zone the clock and the weather stand side by side over the whole width.
 */
class Layout(val w: Float, val h: Float, setup: BlockSetup) {
    val pad = h * 0.04f
    val gap = h * 0.028f
    val barHeight = h * 0.05f
    val gear = RectF(w - pad - barHeight, pad, w - pad, pad + barHeight)

    val cards: List<Block> = setup.shown(Block.CPU, Block.GPU, Block.RAM, Block.NET)
    val cardRects: List<RectF>
    val clock: RectF?
    val weather: RectF?
    val alerts: RectF?
    val batteries: RectF?

    /** The block has its zone to itself, so its content stands in the middle of it. */
    val clockCentred: Boolean
    val weatherCentred: Boolean
    /** The batteries are the only block of their zone and fill it as a list. */
    val batteriesAlone: Boolean
    /** Every block is hidden. */
    val empty: Boolean

    val clockSize: Float
    val secondsSize: Float
    val clockLeft: Float
    val clockBaseline: Float
    val dateBaseline: Float
    /** The size of the temperature, which the rest of the weather follows. */
    val weatherSize: Float
    /** What the clock's own view covers: the digits and the seconds. */
    val digits: RectF?

    init {
        val top = pad + barHeight + gap * 0.7f
        val left = setup.shown(Block.CLOCK, Block.WEATHER)
        val right = setup.shown(Block.ALERTS, Block.BATTERIES)
        empty = left.isEmpty() && right.isEmpty() && cards.isEmpty()
        // With nothing above them the cards take the whole screen.
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
            // Side by side, the first on the left; the clock needs the larger share.
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
            // First of two is on top, the second at the bottom; a block alone sits in the middle.
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

        // The clock is as tall as its block allows, but never wider than it (4:3 screens).
        val room = clock?.width() ?: 0f
        val blockHeight = clock?.height() ?: 0f
        val probe = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = LIGHT }
        val base = blockHeight * 0.64f
        probe.textSize = base
        val clockWidth = probe.measureText("00:00")
        probe.textSize = base * SECONDS
        val secondsWidth = probe.measureText("00") + base * 0.05f
        val scale = if (clockWidth + secondsWidth > 0f) min(1f, room / (clockWidth + secondsWidth)) else 1f
        clockSize = base * scale
        secondsSize = clockSize * SECONDS
        val total = (clockWidth + secondsWidth) * scale
        clockLeft = if (clock == null) 0f else if (clockCentred) clock.centerX() - total / 2 else clock.left
        // The digits and the date under them, centred in the block as one group.
        val group = clockSize * 1.2f
        clockBaseline = (clock?.top ?: 0f) + (blockHeight - group) / 2 + clockSize * 0.8f
        dateBaseline = clockBaseline + clockSize * 0.33f
        digits = if (clock == null) null else RectF(
            floor(clockLeft - clockSize * 0.08f), floor(clockBaseline - clockSize * 0.86f),
            ceil(clockLeft + total + clockSize * 0.1f), ceil(clockBaseline + clockSize * 0.1f),
        )
    }

    companion object {
        private const val SECONDS = 0.2125f
        private const val WIDE_CLOCK = 0.56f
        val LIGHT: Typeface = Typeface.create("sans-serif-light", Typeface.NORMAL) ?: Typeface.DEFAULT
    }
}

/**
 * The phone's own clock. It is a separate view, no larger than the digits, so that the second
 * hand redraws only this corner of the window, not the whole panel with its charts.
 */
class ClockView(context: Context) : View(context) {
    private val text = Paint(Paint.ANTI_ALIAS_FLAG)
    private val timeFormat = SimpleDateFormat("HH:mm", Locale.US)
    private val secondsFormat = SimpleDateFormat("ss", Locale.US)
    private val now = Date()
    private var layout: Layout? = null
    private var originX = 0f
    private var originY = 0f

    /** The time zone of the chosen city; the phone's own when none is chosen. */
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

    /** Takes the place the panel's layout gives to the digits; no place when the clock is hidden. */
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
        now.time = System.currentTimeMillis()
        val away = link == Link.OFFLINE || link == Link.NO_TOKEN || link == Link.REFUSED
        canvas.translate(-originX, -originY)
        text.typeface = Layout.LIGHT
        text.textAlign = Paint.Align.LEFT
        text.textSize = l.clockSize
        text.color = if (away) PanelView.DIM_WHITE else 0xFFFFFFFF.toInt()
        val time = timeFormat.format(now)
        canvas.drawText(time, l.clockLeft - l.clockSize * 0.04f, l.clockBaseline, text)
        val width = text.measureText(time)
        text.textSize = l.secondsSize
        text.color = PanelView.MUTED
        canvas.drawText(secondsFormat.format(now), l.clockLeft + width + l.clockSize * 0.05f, l.clockBaseline - l.clockSize * 0.5f, text)
    }
}
