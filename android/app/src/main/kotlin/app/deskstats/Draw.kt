package app.deskstats

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.os.Build
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.max

/** Cuts the string with an ellipsis so that it fits into [width] at the paint's text size. */
fun Paint.fit(s: String, width: Float): String {
    if (width <= 0f) return ""
    if (measureText(s) <= width) return s
    val count = breakText(s, true, max(0f, width - measureText("…")), null)
    return s.substring(0, count).trimEnd() + "…"
}

/** Splits [s] into at most [maxLines] lines of [width], breaking at spaces where possible. */
fun Paint.wrap(s: String, width: Float, maxLines: Int): List<String> {
    val out = ArrayList<String>()
    var rest = s.trim()
    while (rest.isNotEmpty() && out.size < maxLines) {
        if (out.size == maxLines - 1) {
            out.add(fit(rest, width))
            break
        }
        var count = breakText(rest, true, width, null)
        if (count <= 0) break
        if (count < rest.length) {
            val space = rest.lastIndexOf(' ', count)
            if (space > 0) count = space
        }
        out.add(rest.substring(0, count).trimEnd())
        rest = rest.substring(count).trimStart()
    }
    return out
}

/** Letter spacing for the small capital headings; Android before 5.0 has none. */
fun Paint.spacing(value: Float) {
    if (Build.VERSION.SDK_INT >= 21) letterSpacing = value
}

/** A small "›" with its point at [x], [y]: the mark of a block that opens something. */
fun Canvas.chevron(path: Path, paint: Paint, x: Float, y: Float, arm: Float) {
    path.rewind()
    path.moveTo(x - arm, y - arm)
    path.lineTo(x, y)
    path.lineTo(x - arm, y + arm)
    drawPath(path, paint)
}

/** Half-transparent when [on]: how the panel shows what is left from before the link was lost. */
fun faded(color: Int, on: Boolean): Int = if (on) (color and 0x00FFFFFF) or 0x73000000 else color

object Fmt {
    fun number(pattern: String, value: Float): String = String.format(Locale.US, pattern, value)

    fun gb(bytes: Long): String = String.format(Locale.US, "%.1f", bytes / 1073741824.0)

    /** "7.1 ГБ" or "742 МБ". */
    fun size(bytes: Long): String =
        if (bytes >= 1073741824L) "${gb(bytes)} ГБ" else String.format(Locale.US, "%.0f МБ", bytes / 1048576.0)

    fun rate(bytesPerSecond: Float): String = when {
        bytesPerSecond >= 1_048_576f -> number("%.1f", bytesPerSecond / 1048576f) + " МБ/с"
        bytesPerSecond >= 1024f -> number("%.0f", bytesPerSecond / 1024f) + " КБ/с"
        else -> number("%.0f", bytesPerSecond) + " Б/с"
    }

    /** One decimal below ten percent, none above: "4.2%", "44%". */
    fun percent(value: Float): String = number(if (value < 10f) "%.1f" else "%.0f", value) + "%"
}

/**
 * The time of a notification, written the same way everywhere and in the zone the clock shows:
 * "14:05" today, "вчера 14:05", otherwise "3 нояб. 14:05".
 */
class Stamps {
    private val time = SimpleDateFormat("HH:mm", Locale.US)
    private val full = SimpleDateFormat("d MMM HH:mm", Locale("ru"))
    private val date = Date()

    var zone: TimeZone = TimeZone.getDefault()
        set(value) {
            field = value
            time.timeZone = value
            full.timeZone = value
        }

    /** The number of the calendar day in [zone]; whole days are compared, so a new year is no special case. */
    fun day(ts: Long): Long = (ts + zone.getOffset(ts)) / 86_400_000L

    fun text(ts: Long, now: Long): String {
        date.time = ts
        return when (day(now) - day(ts)) {
            0L -> time.format(date)
            1L -> "вчера ${time.format(date)}"
            else -> full.format(date)
        }
    }
}
