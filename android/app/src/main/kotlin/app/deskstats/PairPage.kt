package app.deskstats

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint

/** Pairs with a Mac over Wi‑Fi: shows the code the Mac shows too and waits until its user clicks Allow. */
class PairPage(context: Context, host: Host, private val mac: Nearby) : Page(context, host) {
    override val title: String = mac.name.ifEmpty { mac.model.ifEmpty { mac.host } }

    private enum class State { CONNECTING, CODE, DENIED, BUSY, FAILED, TIMEOUT }

    private var state = State.CONNECTING
    private var code = ""
    private var client: PairClient? = null

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        begin()
    }

    override fun onDetachedFromWindow() {
        client?.cancelled = true
        client = null
        super.onDetachedFromWindow()
    }

    private fun begin() {
        client?.cancelled = true
        val c = PairClient(mac)
        client = c
        show(State.CONNECTING)
        val language = Ui.language
        Thread({
            val (started, refused) = c.start(language)
            if (c.cancelled) return@Thread
            if (started == null) {
                post { if (client === c) show(stateOf(refused)) }
                return@Thread
            }
            post {
                if (client === c) {
                    code = started.code
                    show(State.CODE)
                }
            }
            val result = c.finish(started) { token -> post { if (client === c) host.pairedOverWifi(mac, token) } }
            if (result != PairClient.Result.PAIRED) post { if (client === c) show(stateOf(result)) }
        }, "pair").start()
    }

    private fun stateOf(result: PairClient.Result?) = when (result) {
        PairClient.Result.DENIED -> State.DENIED
        PairClient.Result.BUSY -> State.BUSY
        PairClient.Result.TIMEOUT -> State.TIMEOUT
        else -> State.FAILED
    }

    private fun show(new: State) {
        state = new
        announce()
        invalidate()
    }

    private fun line(): String = when (state) {
        State.CONNECTING -> context.getString(R.string.pair_connecting, title)
        State.CODE -> context.getString(R.string.pair_code)
        State.DENIED -> context.getString(R.string.pair_denied)
        State.BUSY -> context.getString(R.string.pair_busy)
        State.FAILED -> context.getString(R.string.pair_failed)
        State.TIMEOUT -> context.getString(R.string.pair_timeout)
    }

    private val spacedCode get() = if (code.length == 6) code.substring(0, 3) + " " + code.substring(3) else code

    override fun summary() = if (state == State.CODE) "${context.getString(R.string.pair_code_a11y, spacedCode)}. ${line()}" else line()

    override fun drawBody(canvas: Canvas) {
        val p = Ui.p
        val mid = bodyTop + bodyHeight * 0.42f
        if (state == State.CODE) {
            text.typeface = bold
            text.textAlign = Paint.Align.CENTER
            text.textSize = u * 0.2f
            text.color = p.text
            text.spacing(0.08f)
            canvas.drawText(spacedCode, w / 2, mid, text)
            text.spacing(0f)
        }
        text.typeface = regular
        text.textAlign = Paint.Align.CENTER
        text.textSize = u * 0.045f
        text.color = p.muted
        val lines = text.wrap(line(), w - pad * 4, 4)
        val first = if (state == State.CODE) mid + u * 0.12f else mid
        for (i in lines.indices) canvas.drawText(lines[i], w / 2, first + i * text.textSize * 1.35f, text)
        text.textAlign = Paint.Align.LEFT
    }

    override fun onTap(x: Float, y: Float) {
        if (y > bodyTop && state != State.CONNECTING && state != State.CODE) begin()
    }
}
