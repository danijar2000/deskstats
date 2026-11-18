package app.deskstats

import android.content.SharedPreferences

/** A part of the main screen; blocks are ordered within their zone. */
enum class Zone(val title: String) {
    LEFT("Слева"),
    RIGHT("Справа"),
    ROW("Нижний ряд"),
}

/** A block of the main screen. [zone] says where it lives. */
enum class Block(val id: String, val title: String, val zone: Zone, val note: String) {
    CLOCK("clock", "Часы и дата", Zone.LEFT, "Время телефона в выбранном часовом поясе"),
    WEATHER("weather", "Погода", Zone.LEFT, "Погода в выбранном городе"),
    ALERTS("alerts", "Уведомления", Zone.RIGHT, "Уведомления Mac и iPhone"),
    BATTERIES("batteries", "Заряд устройств", Zone.RIGHT, "Мышь, клавиатура, трекпад"),
    CPU("cpu", "CPU", Zone.ROW, "Загрузка процессора"),
    GPU("gpu", "GPU", Zone.ROW, "Загрузка видеоядра"),
    RAM("ram", "RAM", Zone.ROW, "Занятая память"),
    NET("net", "Сеть", Zone.ROW, "Скорость сети"),
}

/** Which blocks the main screen shows and in what order. Stored as "clock,weather,-alerts,…". */
class BlockSetup(val order: List<Block>, val hidden: Set<Block>) {
    /** The visible ones of [group], in the user's order. */
    fun shown(vararg group: Block): List<Block> = order.filter { it in group && it !in hidden }

    /** Puts [block] where [target] is; the blocks between them shift by one. */
    fun moved(block: Block, target: Block): BlockSetup {
        val list = ArrayList(order)
        val to = list.indexOf(target)
        list.remove(block)
        list.add(to.coerceIn(0, list.size), block)
        return BlockSetup(list, hidden)
    }

    fun toggled(block: Block): BlockSetup = BlockSetup(order, if (block in hidden) hidden - block else hidden + block)

    fun save(prefs: SharedPreferences) {
        prefs.edit().putString(KEY, order.joinToString(",") { (if (it in hidden) "-" else "") + it.id }).apply()
    }

    companion object {
        private const val KEY = "blocks"

        fun load(prefs: SharedPreferences): BlockSetup {
            val order = ArrayList<Block>()
            val hidden = HashSet<Block>()
            for (part in (prefs.getString(KEY, null) ?: "").split(',')) {
                val block = Block.values().firstOrNull { it.id == part.removePrefix("-") } ?: continue
                if (block in order) continue
                order.add(block)
                if (part.startsWith("-")) hidden.add(block)
            }
            // Blocks added in a newer version of the app show up at the end.
            for (block in Block.values()) if (block !in order) order.add(block)
            return BlockSetup(order, hidden)
        }
    }
}
