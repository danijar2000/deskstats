package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint

/** The paired Macs: tap one to show it, × forgets it (with undo). New Macs are added over the USB cable. */
class ServersPage(context: Context, host: Host) : Page(context, host) {
    override val title: String = context.getString(R.string.servers_title)
    private val rowHeight get() = u * 0.16f
    private var hintLines: List<String> = emptyList()

    private var removed: Server? = null
    private var removedIndex = 0
    private var removedChosen = false
    private var removedName = ""
    private val forget = Runnable {
        removed = null
        invalidate()
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(forget)
        super.onDetachedFromWindow()
    }

    private fun name(server: Server) = host.servers.displayName(server) { context.getString(R.string.server_unnamed, it) }

    override fun summary(): String {
        val servers = host.servers
        val none = context.getString(R.string.none)
        return context.getString(R.string.servers_a11y, servers.active?.let(::name) ?: none, servers.list.joinToString(", ") { name(it) }.ifEmpty { none })
    }

    private val hintSize get() = u * 0.036f

    override fun arrange() {
        text.typeface = regular
        text.textSize = hintSize
        hintLines = text.wrap(context.getString(R.string.servers_hint), w - pad * 2 - u * 0.016f, 5)
    }

    override fun contentHeight(pane: Int) =
        if (pane != 0) 0f else host.servers.list.size * rowHeight + rowHeight * 0.3f + hintLines.size * hintSize * 1.35f + rowHeight * 0.3f

    private fun crossX(right: Float) = right - rowHeight * 0.3f - rowHeight * 0.07f

    override fun drawBody(canvas: Canvas) {
        val p = Ui.p
        val servers = host.servers
        val rh = rowHeight
        for ((i, server) in servers.list.withIndex()) {
            val top = bodyTop + rh * i
            box.set(pad, top + rh * 0.05f, w - pad, top + rh * 0.95f)
            val inset = rh * 0.3f
            val active = server.id == servers.active?.id
            card(canvas, rh * 0.2f, if (active) p.selectionMark() else p.border)

            val cx = box.left + inset
            radio(canvas, cx, box.centerY(), rh * 0.1f, active)

            val r = rh * 0.1f
            val x = crossX(box.right)
            stroke.color = p.muted
            stroke.strokeWidth = rh * 0.022f
            canvas.drawLine(x - r, box.centerY() - r, x + r, box.centerY() + r, stroke)
            canvas.drawLine(x - r, box.centerY() + r, x + r, box.centerY() - r, stroke)
            val room = box.right - inset - rh * 0.7f - (cx + rh * 0.3f)

            text.textAlign = Paint.Align.LEFT
            text.typeface = bold
            text.textSize = rh * 0.26f
            text.color = p.text
            canvas.drawText(text.fit(name(server), room), cx + rh * 0.3f, box.top + rh * 0.4f, text)
            text.typeface = regular
            text.textSize = rh * 0.19f
            text.color = p.muted
            val note = if (server.hasWifi) context.getString(R.string.servers_wifi, server.host) else context.getString(R.string.servers_no_wifi)
            canvas.drawText(text.fit(note, room), cx + rh * 0.3f, box.top + rh * 0.7f, text)
        }
        text.typeface = regular
        text.textSize = hintSize
        text.color = p.muted
        text.textAlign = Paint.Align.LEFT
        val first = bodyTop + servers.list.size * rh + rh * 0.3f + hintSize
        for (i in hintLines.indices) canvas.drawText(hintLines[i], pad + u * 0.008f, first + i * hintSize * 1.35f, text)
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
            announce()
            invalidate()
            return
        }
        if (y < bodyTop) return
        val i = ((y - bodyTop + scroll) / rowHeight).toInt()
        val server = servers.list.getOrNull(i) ?: return
        if (x > crossX(w - pad) - rowHeight * 0.17f) {
            removed = server
            removedIndex = i
            removedChosen = servers.active?.id == server.id
            removedName = name(server)
            host.changeServers { it.without(server.id) }
            removeCallbacks(forget)
            postDelayed(forget, UNDO_MS)
            announce()
            invalidate()
        } else {
            host.changeServers { it.select(server.id) }
            host.home()
        }
    }

    private companion object {
        const val UNDO_MS = 6000L
    }
}
