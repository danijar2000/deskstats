package app.deskstats

import android.content.SharedPreferences

enum class Zone(val landscape: Int, val portrait: Int) {
    LEFT(R.string.zone_left, R.string.zone_top),
    RIGHT(R.string.zone_right, R.string.zone_middle),
    ROW(R.string.zone_row, R.string.zone_bottom),
}

enum class Block(val id: String, val title: Int, val zone: Zone, val note: Int) {
    CLOCK("clock", R.string.block_clock, Zone.LEFT, R.string.block_clock_note),
    WEATHER("weather", R.string.block_weather, Zone.LEFT, R.string.block_weather_note),
    ALERTS("alerts", R.string.block_alerts, Zone.RIGHT, R.string.block_alerts_note),
    BATTERIES("batteries", R.string.block_batteries, Zone.RIGHT, R.string.block_batteries_note),
    CPU("cpu", R.string.cpu, Zone.ROW, R.string.block_cpu_note),
    GPU("gpu", R.string.gpu, Zone.ROW, R.string.block_gpu_note),
    RAM("ram", R.string.ram, Zone.ROW, R.string.block_ram_note),
    NET("net", R.string.network, Zone.ROW, R.string.block_net_note),
    DISK("disk", R.string.block_disk, Zone.ROW, R.string.block_disk_note),
}

class BlockSetup(val order: List<Block>, val hidden: Set<Block>) {
    fun shown(vararg group: Block): List<Block> = order.filter { it in group && it !in hidden }

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
        private val ADDED_LATER = setOf(Block.DISK)

        fun load(prefs: SharedPreferences): BlockSetup {
            val order = ArrayList<Block>()
            val hidden = HashSet<Block>()
            for (part in (prefs.getString(KEY, null) ?: "").split(',')) {
                val block = Block.values().firstOrNull { it.id == part.removePrefix("-") } ?: continue
                if (block in order) continue
                order.add(block)
                if (part.startsWith("-")) hidden.add(block)
            }
            val saved = prefs.getString(KEY, null) != null
            for (block in Block.values()) {
                if (block in order) continue
                order.add(block)
                // A block added in an update starts hidden for someone who already arranged the screen.
                if (saved && block in ADDED_LATER) hidden.add(block)
            }
            return BlockSetup(order, hidden)
        }
    }
}
