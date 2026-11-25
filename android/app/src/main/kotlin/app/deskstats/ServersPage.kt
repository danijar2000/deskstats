package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint

/**
 * The Macs: those the phone is paired with (tap one to show it, × forgets it, with undo) and, below them, the Macs that
 * announce themselves on this Wi‑Fi (tap one to pair with it, confirmed on the Mac).
 */
class ServersPage(context: Context, host: Host) : Page(context, host) {
    override val title: String = context.getString(R.string.servers_title)
    private val rowHeight get() = u * 0.16f
    private val headHeight get() = u * 0.09f
    private var hintLines: List<String> = emptyList()

    private sealed class Row(val top: Float, val height: Float)
    private class Heading(top: Float, height: Float, val label: String) : Row(top, height)
    private class Paired(top: Float, height: Float, val server: Server) : Row(top, height)
    private class Found(top: Float, height: Float, val mac: Nearby) : Row(top, height)
    private class Searching(top: Float, height: Float) : Row(top, height)

    private var rows: List<Row> = emptyList()
    private var rowsEnd = 0f
    private var heard: List<Nearby> = emptyList()

    private var removed: Server? = null
    private var removedIndex = 0
    private var removedChosen = false
    private var removedName = ""
    private val forget = Runnable {
        removed = null
        invalidate()
    }

    // The list of Macs nearby changes as they come and go: look again now and then while the page is open.
    private var ticks = 0
    private val tick = object : Runnable {
        override fun run() {
            if (visibility == VISIBLE) {
                host.discover()
                if (ticks++ % SCAN_EVERY == 0) host.scan()
                resumed()
            }
            postDelayed(this, TICK_MS)
        }
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        ticks = 0
        post(tick)
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(forget)
        removeCallbacks(tick)
        super.onDetachedFromWindow()
    }

    private fun name(server: Server) = host.servers.displayName(server) { context.getString(R.string.server_unnamed, it) }

    private fun name(mac: Nearby) = mac.name.ifEmpty { mac.model.ifEmpty { mac.host } }

    override fun summary(): String {
        val servers = host.servers
        val none = context.getString(R.string.none)
        val shown = context.getString(R.string.servers_a11y, servers.active?.let(::name) ?: none, servers.list.joinToString(", ") { name(it) }.ifEmpty { none })
        val found = heard.filter { mac -> servers.byId(mac.id) == null }
        return if (found.isEmpty()) shown else "$shown. ${context.getString(R.string.servers_nearby)}: ${found.joinToString(", ") { name(it) }}"
    }

    private val hintSize get() = u * 0.036f

    override fun arrange() {
        text.typeface = regular
        text.textSize = hintSize
        hintLines = text.wrap(context.getString(R.string.servers_hint), w - pad * 2 - u * 0.016f, 6)
        heard = host.nearby
        val servers = host.servers
        val list = ArrayList<Row>()
        var y = 0f
        if (servers.list.isNotEmpty()) {
            list.add(Heading(y, headHeight, context.getString(R.string.servers_paired)))
            y += headHeight
            for (server in servers.list) {
                list.add(Paired(y, rowHeight, server))
                y += rowHeight
            }
            y += gap
        }
        list.add(Heading(y, headHeight, context.getString(R.string.servers_nearby)))
        y += headHeight
        val found = heard.filter { servers.byId(it.id) == null }
        if (found.isEmpty()) {
            list.add(Searching(y, rowHeight * 0.6f))
            y += rowHeight * 0.6f
        }
        for (mac in found) {
            list.add(Found(y, rowHeight, mac))
            y += rowHeight
        }
        rows = list
        rowsEnd = y
    }

    override fun contentHeight(pane: Int) =
        if (pane != 0) 0f else rowsEnd + rowHeight * 0.3f + hintLines.size * hintSize * 1.35f + rowHeight * 0.3f

    private fun crossX(right: Float) = right - rowHeight * 0.3f - rowHeight * 0.07f

    override fun drawBody(canvas: Canvas) {
        val p = Ui.p
        val servers = host.servers
        val rh = rowHeight
        for (row in rows) {
            val top = bodyTop + row.top
            when (row) {
                is Heading -> heading(canvas, row.label, top + row.height * 0.72f)
                is Searching -> {
                    text.typeface = regular
                    text.textSize = hintSize
                    text.color = p.muted
                    text.textAlign = Paint.Align.LEFT
                    canvas.drawText(text.fit(context.getString(R.string.servers_searching), w - pad * 2), pad + u * 0.008f, top + row.height * 0.55f, text)
                }
                is Paired -> {
                    val server = row.server
                    box.set(pad, top + rh * 0.05f, w - pad, top + rh * 0.95f)
                    val active = server.id == servers.active?.id
                    card(canvas, rh * 0.2f, if (active) p.selectionMark() else p.border)
                    val cx = box.left + rh * 0.3f
                    radio(canvas, cx, box.centerY(), rh * 0.1f, active)
                    val r = rh * 0.1f
                    val x = crossX(box.right)
                    stroke.color = p.muted
                    stroke.strokeWidth = rh * 0.022f
                    canvas.drawLine(x - r, box.centerY() - r, x + r, box.centerY() + r, stroke)
                    canvas.drawLine(x - r, box.centerY() + r, x + r, box.centerY() - r, stroke)
                    val there = heard.firstOrNull { it.id == server.id }?.host ?: server.host
                    val note = if (there != null) context.getString(R.string.servers_wifi, there) else context.getString(R.string.servers_no_wifi)
                    lines(canvas, name(server), note, cx + rh * 0.3f, box.right - rh * 0.3f - rh * 0.7f)
                }
                is Found -> {
                    val mac = row.mac
                    box.set(pad, top + rh * 0.05f, w - pad, top + rh * 0.95f)
                    card(canvas, rh * 0.2f, p.border)
                    val left = box.left + rh * 0.3f
                    forward(canvas, box.right - rh * 0.3f, box.centerY(), rh * 0.09f)
                    val note = listOf(mac.model, mac.host).filter { it.isNotEmpty() && it != name(mac) }.joinToString(" · ")
                    lines(canvas, name(mac), note, left, box.right - rh * 0.6f)
                }
            }
        }
        text.typeface = regular
        text.textSize = hintSize
        text.color = p.muted
        text.textAlign = Paint.Align.LEFT
        val first = bodyTop + rowsEnd + rh * 0.3f + hintSize
        for (i in hintLines.indices) canvas.drawText(hintLines[i], pad + u * 0.008f, first + i * hintSize * 1.35f, text)
    }

    private fun lines(canvas: Canvas, title: String, note: String, left: Float, right: Float) {
        val rh = rowHeight
        val room = right - left
        text.textAlign = Paint.Align.LEFT
        text.typeface = bold
        text.textSize = rh * 0.26f
        text.color = Ui.p.text
        canvas.drawText(text.fit(title, room), left, box.top + rh * 0.4f, text)
        text.typeface = regular
        text.textSize = rh * 0.19f
        text.color = Ui.p.muted
        canvas.drawText(text.fit(note, room), left, box.top + rh * 0.7f, text)
    }

    override fun drawFixed(canvas: Canvas) {
        removed?.let { toast(canvas, context.getString(R.string.servers_removed, removedName), context.getString(R.string.undo), Ui.p.accent) }
    }

    override fun onTap(x: Float, y: Float) {
        val servers = host.servers
        val gone = removed
        if (gone != null && toastBox.contains(x, y)) {
            removeCallbacks(forget)
            removed = null
            host.changeServers { it.restored(gone, removedIndex, removedChosen) }
            resumed()
            return
        }
        if (y < bodyTop) return
        val at = y - bodyTop + scroll
        when (val row = rows.firstOrNull { at >= it.top && at < it.top + it.height }) {
            is Paired -> {
                val server = row.server
                if (x > crossX(w - pad) - rowHeight * 0.17f) {
                    removed = server
                    removedIndex = servers.list.indexOfFirst { it.id == server.id }
                    removedChosen = servers.active?.id == server.id
                    removedName = name(server)
                    host.changeServers { it.without(server.id) }
                    removeCallbacks(forget)
                    postDelayed(forget, UNDO_MS)
                    resumed()
                } else {
                    host.changeServers { it.select(server.id) }
                    host.home()
                }
            }
            is Found -> host.open(PairPage(context, host, row.mac))
            else -> {}
        }
    }

    private companion object {
        const val UNDO_MS = 6000L
        const val TICK_MS = 3000L
        const val SCAN_EVERY = 5
    }
}
