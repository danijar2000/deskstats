package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.os.Build
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.max

fun Paint.fit(s: String, width: Float): String {
    if (width <= 0f) return ""
    if (measureText(s) <= width) return s
    val count = breakText(s, true, max(0f, width - measureText("…")), null)
    return s.substring(0, count).trimEnd() + "…"
}

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

fun Paint.spacing(value: Float) {
    if (Build.VERSION.SDK_INT >= 21) letterSpacing = if (Ui.language == "zh" || Ui.language == "ja") value * 0.3f else value
}

fun Canvas.chevron(path: Path, paint: Paint, x: Float, y: Float, arm: Float) {
    path.rewind()
    path.moveTo(x - arm, y - arm)
    path.lineTo(x, y)
    path.lineTo(x - arm, y + arm)
    drawPath(path, paint)
}

fun faded(color: Int, on: Boolean): Int = if (on) (color and 0x00FFFFFF) or 0x73000000 else color

fun String.capitalized(): String = if (isEmpty()) this else substring(0, 1).uppercase(Ui.locale) + substring(1)

fun String.heading(): String = uppercase(Ui.locale)

object Fmt {
    private var gb = "GB"
    private var mb = "MB"
    private var bs = "B/s"
    private var kbs = "KB/s"
    private var mbs = "MB/s"

    fun init(context: Context) {
        gb = context.getString(R.string.unit_gb)
        mb = context.getString(R.string.unit_mb)
        bs = context.getString(R.string.unit_bs)
        kbs = context.getString(R.string.unit_kbs)
        mbs = context.getString(R.string.unit_mbs)
    }

    val gbUnit get() = gb

    fun number(pattern: String, value: Float): String = String.format(Ui.locale, pattern, value)

    fun gb(bytes: Long): String = String.format(Ui.locale, "%.1f", bytes / 1073741824.0)

    fun size(bytes: Long): String =
        if (bytes >= 1073741824L) "${gb(bytes)} $gb" else String.format(Ui.locale, "%.0f %s", bytes / 1048576.0, mb)

    fun rate(bytesPerSecond: Float): String = when {
        bytesPerSecond >= 1_048_576f -> number("%.1f", bytesPerSecond / 1048576f) + " " + mbs
        bytesPerSecond >= 1024f -> number("%.0f", bytesPerSecond / 1024f) + " " + kbs
        else -> number("%.0f", bytesPerSecond) + " " + bs
    }

    fun percent(value: Float): String = number(if (value < 10f) "%.1f" else "%.0f", value) + "%"
}

class Stamps(context: Context) {
    private val time = SimpleDateFormat("HH:mm", Locale.US)
    private val full = SimpleDateFormat(context.getString(R.string.fmt_stamp), Ui.locale)
    private val yesterday = context.getString(R.string.yesterday_at)
    private val date = Date()

    var zone: TimeZone = TimeZone.getDefault()
        set(value) {
            field = value
            time.timeZone = value
            full.timeZone = value
        }

    fun day(ts: Long): Long = (ts + zone.getOffset(ts)) / 86_400_000L

    fun text(ts: Long, now: Long): String {
        date.time = ts
        return when (day(now) - day(ts)) {
            0L -> time.format(date)
            1L -> String.format(yesterday, time.format(date))
            else -> full.format(date)
        }
    }
}
