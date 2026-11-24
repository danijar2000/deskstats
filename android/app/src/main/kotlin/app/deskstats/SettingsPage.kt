package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import kotlin.math.max

class SettingsPage(context: Context, host: Host) : Page(context, host) {
    private enum class Kind { LANGUAGE, THEME, ORIENTATION, BLOCKS, WIFI, ALERTS, INTERVAL, WEATHER }

    private class Item(val kind: Kind, val title: String, val hint: String, val choices: List<String>) {
        var top = 0f
        var height = 0f
        var stacked = false
        var hintLines: List<String> = emptyList()
        val chips = ArrayList<RectF>()
    }

    override val title: String = context.getString(R.string.settings_title)
    private val groups = listOf(context.getString(R.string.group_look), context.getString(R.string.group_mac))
    private val headTops = FloatArray(2)
    private val items = listOf(
        Item(Kind.LANGUAGE, context.getString(R.string.set_language), "", emptyList()),
        Item(Kind.THEME, context.getString(R.string.set_theme), "", listOf(R.string.theme_dark, R.string.theme_light, R.string.theme_neon).map(context::getString)),
        Item(Kind.ORIENTATION, context.getString(R.string.set_orientation), "", listOf(R.string.orient_landscape, R.string.orient_portrait, R.string.orient_auto).map(context::getString)),
        Item(Kind.BLOCKS, context.getString(R.string.blocks_title), "", emptyList()),
        Item(Kind.WIFI, context.getString(R.string.set_wifi), context.getString(R.string.set_wifi_hint), emptyList()),
        Item(Kind.ALERTS, context.getString(R.string.set_alerts), context.getString(R.string.set_alerts_hint), emptyList()),
        Item(Kind.INTERVAL, context.getString(R.string.set_interval), "", Options.INTERVALS.map { seconds(it) }),
        Item(Kind.WEATHER, context.getString(R.string.set_weather), "", Options.WEATHER.map { minutes(it) }),
    )
    private val version = context.getString(R.string.version, appVersion(context))
    private var total = 0f

    private val rowHeight get() = max(u * 0.13f, 52f * resources.displayMetrics.density)
    private val headHeight get() = rowHeight * 0.6f
    private val inset get() = rowHeight * 0.3f

    private fun seconds(ms: Int) = context.getString(R.string.fmt_seconds, Fmt.number(if (ms % 1000 == 0) "%.0f" else "%.1f", ms / 1000f))

    private fun minutes(min: Int) =
        if (min % 60 == 0) context.getString(R.string.fmt_hours, min / 60) else context.getString(R.string.fmt_minutes, min)

    private fun selected(kind: Kind): Int {
        val o = host.options
        return when (kind) {
            Kind.THEME -> Options.THEMES.indexOf(o.theme)
            Kind.ORIENTATION -> Options.ORIENTATIONS.indexOf(o.orientation)
            Kind.INTERVAL -> Options.INTERVALS.indexOf(o.intervalMs)
            Kind.WEATHER -> Options.WEATHER.indexOf(o.weatherMin)
            else -> -1
        }
    }

    private fun value(kind: Kind): String = when (kind) {
        Kind.LANGUAGE -> languageName(context, host.options.language)
        Kind.BLOCKS -> context.getString(R.string.blocks_shown, Block.values().size - host.setup.hidden.size, Block.values().size)
        else -> ""
    }

    override fun summary(): String {
        val on = context.getString(R.string.a11y_on)
        val off = context.getString(R.string.a11y_off)
        return items.joinToString(". ") { item ->
            val v = when (item.kind) {
                Kind.WIFI -> if (host.options.lan) on else off
                Kind.ALERTS -> if (host.options.alerts) on else off
                Kind.LANGUAGE, Kind.BLOCKS -> value(item.kind)
                else -> item.choices.getOrElse(selected(item.kind)) { "" }
            }
            "${item.title}: $v"
        }
    }

    private fun chipTextSize() = rowHeight * 0.24f

    override fun arrange() {
        val rh = rowHeight
        var y = 0f
        val width = w - pad * 2
        for ((i, item) in items.withIndex()) {
            if (i == 0 || i == 4) {
                headTops[if (i == 0) 0 else 1] = y
                y += headHeight
            }
            item.top = y
            item.chips.clear()
            item.stacked = false
            var height = rh
            if (item.choices.isNotEmpty()) {
                text.typeface = bold
                text.textSize = chipTextSize()
                val swatch = if (item.kind == Kind.THEME) rh * 0.3f else 0f
                val segment = item.choices.maxOf { text.measureText(it) } + rh * 0.5f + swatch
                text.textSize = rh * 0.27f
                val titleWidth = text.measureText(item.title)
                val chipsWidth = segment * item.choices.size
                item.stacked = inset * 2 + titleWidth + rh * 0.4f + chipsWidth > width
                val chipH = rh * 0.56f
                val segW = if (item.stacked) (width - inset * 2) / item.choices.size else segment
                if (item.stacked) height = rh * 1.75f
                val left = if (item.stacked) pad + inset else w - pad - inset - segW * item.choices.size
                val mid = if (item.stacked) y + rh * 0.06f + rh * 1.18f else y + rh / 2
                for (k in item.choices.indices) item.chips.add(RectF(left + segW * k, mid - chipH / 2, left + segW * (k + 1), mid + chipH / 2))
            } else if (item.hint.isNotEmpty()) {
                text.typeface = regular
                text.textSize = rh * 0.2f
                item.hintLines = text.wrap(item.hint, width - inset * 2 - rh * 0.4f * 1.9f - rh * 0.3f, 2)
                height = rh * (if (item.hintLines.size > 1) 1.45f else 1.2f)
            }
            item.height = height
            y += height
        }
        total = y + rowHeight * 0.9f
    }

    override fun contentHeight(pane: Int) = if (pane == 0) total else 0f

    override fun drawBody(canvas: Canvas) {
        val p = Ui.p
        heading(canvas, groups[0], bodyTop + headTops[0] + headHeight * 0.72f)
        heading(canvas, groups[1], bodyTop + headTops[1] + headHeight * 0.72f)
        val rh = rowHeight
        for (item in items) {
            val top = bodyTop + item.top
            box.set(pad, top + rh * 0.06f, w - pad, top + item.height - rh * 0.06f)
            card(canvas, rh * 0.2f)
            val left = box.left + inset
            val right = box.right - inset
            val titleMid = if (item.stacked) box.top + rh * 0.44f else box.centerY()
            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.textSize = rh * 0.27f
            text.color = p.text
            var room = right - left
            when (item.kind) {
                Kind.WIFI, Kind.ALERTS -> {
                    val on = if (item.kind == Kind.WIFI) host.options.lan else host.options.alerts
                    switch(canvas, right, box.centerY(), rh * 0.4f, on)
                    room -= rh * 0.4f * 1.9f + rh * 0.3f
                }
                Kind.LANGUAGE, Kind.BLOCKS -> {
                    forward(canvas, right, box.centerY(), rh * 0.09f)
                    text.typeface = regular
                    text.textSize = rh * 0.24f
                    text.color = p.muted
                    text.textAlign = Paint.Align.RIGHT
                    val v = text.fit(value(item.kind), (right - left) * 0.5f)
                    canvas.drawText(v, right - rh * 0.3f, box.centerY() + rh * 0.085f, text)
                    room -= text.measureText(v) + rh * 0.6f
                    text.textAlign = Paint.Align.LEFT
                    text.typeface = bold
                    text.textSize = rh * 0.27f
                    text.color = p.text
                }
                else -> if (!item.stacked) room = (item.chips.firstOrNull()?.left ?: right) - left - rh * 0.3f
            }
            if (item.hint.isEmpty()) {
                canvas.drawText(text.fit(item.title, room), left, titleMid + rh * 0.095f, text)
            } else {
                val lines = item.hintLines
                val start = box.centerY() - (rh * 0.34f + lines.size * rh * 0.26f) / 2
                canvas.drawText(text.fit(item.title, room), left, start + rh * 0.27f, text)
                text.typeface = regular
                text.textSize = rh * 0.2f
                text.color = p.muted
                for (k in lines.indices) canvas.drawText(lines[k], left, start + rh * 0.34f + rh * 0.26f * (k + 1) - rh * 0.02f, text)
            }
            if (item.chips.isNotEmpty()) drawChips(canvas, item, top - item.top)
        }
        text.typeface = regular
        text.textSize = u * 0.03f
        text.color = p.dim
        text.textAlign = Paint.Align.CENTER
        canvas.drawText(version, w / 2, bodyTop + total - rowHeight * 0.3f, text)
        text.textAlign = Paint.Align.LEFT
    }

    private fun drawChips(canvas: Canvas, item: Item, dy: Float) {
        val p = Ui.p
        val first = item.chips.first()
        val last = item.chips.last()
        val height = first.height()
        box.set(first.left, first.top + dy, last.right, last.bottom + dy)
        fill.color = p.background
        canvas.drawRoundRect(box, height / 2, height / 2, fill)
        stroke.color = p.border
        stroke.strokeWidth = max(1f, u * 0.002f)
        canvas.drawRoundRect(box, height / 2, height / 2, stroke)
        val chosen = selected(item.kind)
        val swatch = item.kind == Kind.THEME
        for ((k, chip) in item.chips.withIndex()) {
            val on = k == chosen
            box.set(chip.left + height * 0.08f, chip.top + dy + height * 0.08f, chip.right - height * 0.08f, chip.bottom + dy - height * 0.08f)
            if (on) {
                fill.color = p.chipOn
                canvas.drawRoundRect(box, box.height() / 2, box.height() / 2, fill)
            }
            text.typeface = if (on) bold else regular
            text.textSize = chipTextSize()
            text.color = if (on) p.chipOnText else p.muted
            text.textAlign = Paint.Align.CENTER
            var cx = box.centerX()
            if (swatch) {
                val r = height * 0.17f
                val label = text.measureText(item.choices[k])
                val sx = cx - (label + r * 2 + height * 0.18f) / 2 + r
                val palette = Palette.of(Options.THEMES[k])
                fill.color = palette.swatch[0]
                canvas.drawCircle(sx, box.centerY(), r, fill)
                stroke.color = palette.swatch[1]
                stroke.strokeWidth = r * 0.45f
                canvas.drawCircle(sx, box.centerY(), r, stroke)
                cx = sx + r + height * 0.18f + label / 2
            }
            canvas.drawText(text.fit(item.choices[k], box.width() * 0.94f), cx, box.centerY() + text.textSize * 0.36f, text)
        }
        text.textAlign = Paint.Align.LEFT
    }

    override fun onTap(x: Float, y: Float) {
        if (y < bodyTop) return
        val at = y - bodyTop + scroll
        val item = items.firstOrNull { at >= it.top && at < it.top + it.height } ?: return
        val o = host.options
        when (item.kind) {
            Kind.LANGUAGE -> host.open(LanguagePage(context, host))
            Kind.BLOCKS -> host.open(BlocksPage(context, host))
            Kind.WIFI -> host.options = o.copy(lan = !o.lan)
            Kind.ALERTS -> host.options = o.copy(alerts = !o.alerts)
            else -> {
                val k = item.chips.indexOfFirst { x >= it.left && x < it.right && at >= it.top - rowHeight * 0.2f && at <= it.bottom + rowHeight * 0.2f }
                if (k < 0) return
                host.options = when (item.kind) {
                    Kind.THEME -> o.copy(theme = Options.THEMES[k])
                    Kind.ORIENTATION -> o.copy(orientation = Options.ORIENTATIONS[k])
                    Kind.INTERVAL -> o.copy(intervalMs = Options.INTERVALS[k])
                    else -> o.copy(weatherMin = Options.WEATHER[k])
                }
            }
        }
        announce()
        invalidate()
    }

    companion object {
        @Suppress("DEPRECATION")
        fun appVersion(context: Context): String = try {
            context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: ""
        } catch (e: Exception) {
            ""
        }

        fun languageName(context: Context, code: String): String = when (code) {
            "" -> context.getString(R.string.lang_system_named, ownName(context, Ui.systemLanguage()))
            else -> ownName(context, code)
        }

        fun ownName(context: Context, code: String): String = context.getString(
            when (code) {
                "ru" -> R.string.lang_ru
                "es" -> R.string.lang_es
                "de" -> R.string.lang_de
                "fr" -> R.string.lang_fr
                "pt" -> R.string.lang_pt
                "zh" -> R.string.lang_zh
                "ja" -> R.string.lang_ja
                else -> R.string.lang_en
            },
        )
    }
}

class LanguagePage(context: Context, host: Host) : Page(context, host) {
    override val title: String = context.getString(R.string.set_language)
    private val codes = listOf("") + Options.LANGUAGES
    private val names = codes.map { if (it.isEmpty()) context.getString(R.string.lang_system) else SettingsPage.ownName(context, it) }
    private val system = SettingsPage.ownName(context, Ui.systemLanguage())
    private val rowHeight get() = max(u * 0.12f, 48f * resources.displayMetrics.density)

    override fun summary() = names.joinToString(", ")

    override fun contentHeight(pane: Int) = if (pane == 0) codes.size * rowHeight else 0f

    override fun drawBody(canvas: Canvas) {
        val p = Ui.p
        val rh = rowHeight
        val current = host.options.language
        for (i in codes.indices) {
            val top = bodyTop + rh * i
            if (top + rh < bodyTop + scroll || top > scroll + h) continue
            box.set(pad, top + rh * 0.06f, w - pad, top + rh * 0.94f)
            val on = codes[i] == current
            card(canvas, rh * 0.2f, if (on) p.selectionMark() else p.border)
            val inset = rh * 0.32f
            radio(canvas, box.left + inset, box.centerY(), rh * 0.12f, on)
            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.textSize = rh * 0.28f
            text.color = p.text
            val x = box.left + inset + rh * 0.4f
            canvas.drawText(names[i], x, box.centerY() + rh * 0.1f, text)
            if (codes[i].isEmpty()) {
                val width = text.measureText(names[i])
                text.typeface = regular
                text.textSize = rh * 0.24f
                text.color = p.muted
                canvas.drawText(text.fit(system, box.right - inset - x - width - rh * 0.3f), x + width + rh * 0.3f, box.centerY() + rh * 0.09f, text)
            }
        }
    }

    override fun onTap(x: Float, y: Float) {
        if (y < bodyTop) return
        val i = ((y - bodyTop + scroll) / rowHeight).toInt()
        if (i !in codes.indices) return
        if (codes[i] != host.options.language) host.options = host.options.copy(language = codes[i]) else host.back()
    }
}
