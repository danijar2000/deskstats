package app.deskstats

import android.content.Context
import android.content.SharedPreferences
import java.util.TimeZone

class City(val id: String, private val english: String, val zoneId: String, val lat: Float, val lon: Float) {
    var index = 0
        internal set

    val zone: TimeZone by lazy { TimeZone.getTimeZone(zoneId) }

    val name: String get() = Places.names?.getOrNull(index)?.takeIf { it.isNotEmpty() } ?: english
}

class Region(val name: Int, vararg val cities: City)

class Places(val saved: List<City>, val active: City?) {
    fun save(prefs: SharedPreferences) {
        prefs.edit().putString(SAVED, saved.joinToString(",") { it.id }).putString(ACTIVE, active?.id ?: "").apply()
    }

    fun with(city: City): Places = Places(if (saved.any { it.id == city.id }) saved else saved + city, city)

    fun without(city: City): Places {
        val rest = saved.filter { it.id != city.id }
        return Places(rest, active?.takeIf { it.id != city.id } ?: rest.firstOrNull())
    }

    fun restored(city: City, index: Int, chosen: Boolean): Places {
        if (saved.any { it.id == city.id }) return this
        val list = ArrayList(saved)
        list.add(index.coerceIn(0, list.size), city)
        return Places(list, if (chosen || active == null) city else active)
    }

    companion object {
        private const val SAVED = "places"
        private const val ACTIVE = "place"

        var names: Array<String>? = null
            private set

        fun localize(context: Context) {
            val id = when (Ui.language) {
                "ru" -> R.array.cities_ru
                "es" -> R.array.cities_es
                "de" -> R.array.cities_de
                "fr" -> R.array.cities_fr
                "pt" -> R.array.cities_pt
                "zh" -> R.array.cities_zh
                "ja" -> R.array.cities_ja
                else -> 0
            }
            names = if (id == 0) null else context.resources.getStringArray(id).takeIf { it.size == ALL.size }
        }

        fun load(prefs: SharedPreferences): Places {
            if (!prefs.contains(SAVED)) {
                val now = System.currentTimeMillis()
                val offset = TimeZone.getDefault().getOffset(now)
                val home = ALL.firstOrNull { it.zone.getOffset(now) == offset }
                return Places(listOfNotNull(home), home).also { it.save(prefs) }
            }
            val saved = (prefs.getString(SAVED, null) ?: "").split(',').mapNotNull { id -> ALL.firstOrNull { it.id == id } }.distinctBy { it.id }
            val active = prefs.getString(ACTIVE, null)?.let { id -> saved.firstOrNull { it.id == id } } ?: saved.firstOrNull()
            return Places(saved, active)
        }

        private fun c(id: String, name: String, zone: String, lat: Double, lon: Double) = City(id, name, zone, lat.toFloat(), lon.toFloat())

        val REGIONS: List<Region> = listOf(
            Region(
                R.string.region_kyrgyzstan,
                c("bishkek", "Bishkek", "GMT+06:00", 42.87, 74.59),
                c("osh", "Osh", "GMT+06:00", 40.53, 72.80),
                c("jalal-abad", "Jalal-Abad", "GMT+06:00", 40.93, 73.00),
                c("karakol", "Karakol", "GMT+06:00", 42.49, 78.39),
                c("cholpon-ata", "Cholpon-Ata", "GMT+06:00", 42.65, 77.08),
                c("naryn", "Naryn", "GMT+06:00", 41.43, 76.00),
                c("talas", "Talas", "GMT+06:00", 42.52, 72.24),
                c("batken", "Batken", "GMT+06:00", 40.06, 70.82),
                c("tokmok", "Tokmok", "GMT+06:00", 42.84, 75.30),
            ),
            Region(
                R.string.region_central_asia,
                c("almaty", "Almaty", "GMT+05:00", 43.24, 76.89),
                c("astana", "Astana", "GMT+05:00", 51.17, 71.43),
                c("shymkent", "Shymkent", "GMT+05:00", 42.32, 69.59),
                c("tashkent", "Tashkent", "GMT+05:00", 41.30, 69.24),
                c("samarkand", "Samarkand", "GMT+05:00", 39.65, 66.96),
                c("dushanbe", "Dushanbe", "GMT+05:00", 38.56, 68.79),
                c("ashgabat", "Ashgabat", "GMT+05:00", 37.95, 58.38),
            ),
            Region(
                R.string.region_russia,
                c("moscow", "Moscow", "GMT+03:00", 55.75, 37.62),
                c("saint-petersburg", "Saint Petersburg", "GMT+03:00", 59.94, 30.31),
                c("kazan", "Kazan", "GMT+03:00", 55.79, 49.12),
                c("sochi", "Sochi", "GMT+03:00", 43.59, 39.72),
                c("yekaterinburg", "Yekaterinburg", "GMT+05:00", 56.84, 60.61),
                c("novosibirsk", "Novosibirsk", "GMT+07:00", 55.03, 82.92),
                c("krasnoyarsk", "Krasnoyarsk", "GMT+07:00", 56.01, 92.87),
                c("irkutsk", "Irkutsk", "GMT+08:00", 52.29, 104.30),
                c("vladivostok", "Vladivostok", "GMT+10:00", 43.12, 131.89),
            ),
            Region(
                R.string.region_caucasus,
                c("baku", "Baku", "GMT+04:00", 40.41, 49.87),
                c("tbilisi", "Tbilisi", "GMT+04:00", 41.72, 44.79),
                c("yerevan", "Yerevan", "GMT+04:00", 40.18, 44.51),
                c("istanbul", "Istanbul", "GMT+03:00", 41.01, 28.98),
                c("antalya", "Antalya", "GMT+03:00", 36.90, 30.70),
            ),
            Region(
                R.string.region_middle_east,
                c("dubai", "Dubai", "GMT+04:00", 25.20, 55.27),
                c("doha", "Doha", "GMT+03:00", 25.29, 51.53),
                c("riyadh", "Riyadh", "GMT+03:00", 24.71, 46.68),
                c("tel-aviv", "Tel Aviv", "Asia/Jerusalem", 32.09, 34.78),
            ),
            Region(
                R.string.region_europe,
                c("minsk", "Minsk", "GMT+03:00", 53.90, 27.57),
                c("kyiv", "Kyiv", "Europe/Kiev", 50.45, 30.52),
                c("london", "London", "Europe/London", 51.51, -0.13),
                c("paris", "Paris", "Europe/Paris", 48.86, 2.35),
                c("berlin", "Berlin", "Europe/Berlin", 52.52, 13.40),
                c("amsterdam", "Amsterdam", "Europe/Amsterdam", 52.37, 4.90),
                c("madrid", "Madrid", "Europe/Madrid", 40.42, -3.70),
                c("lisbon", "Lisbon", "Europe/Lisbon", 38.72, -9.14),
                c("rome", "Rome", "Europe/Rome", 41.90, 12.50),
                c("zurich", "Zurich", "Europe/Zurich", 47.37, 8.54),
                c("vienna", "Vienna", "Europe/Vienna", 48.21, 16.37),
                c("prague", "Prague", "Europe/Prague", 50.08, 14.44),
                c("warsaw", "Warsaw", "Europe/Warsaw", 52.23, 21.01),
                c("stockholm", "Stockholm", "Europe/Stockholm", 59.33, 18.07),
                c("helsinki", "Helsinki", "Europe/Helsinki", 60.17, 24.94),
                c("athens", "Athens", "Europe/Athens", 37.98, 23.73),
            ),
            Region(
                R.string.region_asia,
                c("islamabad", "Islamabad", "GMT+05:00", 33.69, 73.05),
                c("delhi", "Delhi", "GMT+05:30", 28.61, 77.21),
                c("mumbai", "Mumbai", "GMT+05:30", 19.08, 72.88),
                c("dhaka", "Dhaka", "GMT+06:00", 23.81, 90.41),
                c("bangkok", "Bangkok", "GMT+07:00", 13.76, 100.50),
                c("phuket", "Phuket", "GMT+07:00", 7.88, 98.39),
                c("hanoi", "Hanoi", "GMT+07:00", 21.03, 105.85),
                c("jakarta", "Jakarta", "GMT+07:00", -6.21, 106.85),
                c("bali", "Bali", "GMT+08:00", -8.65, 115.22),
                c("singapore", "Singapore", "GMT+08:00", 1.35, 103.82),
                c("kuala-lumpur", "Kuala Lumpur", "GMT+08:00", 3.14, 101.69),
                c("hong-kong", "Hong Kong", "GMT+08:00", 22.32, 114.17),
                c("beijing", "Beijing", "GMT+08:00", 39.90, 116.41),
                c("shanghai", "Shanghai", "GMT+08:00", 31.23, 121.47),
                c("urumqi", "Urumqi", "GMT+08:00", 43.83, 87.62),
                c("ulaanbaatar", "Ulaanbaatar", "GMT+08:00", 47.89, 106.91),
                c("seoul", "Seoul", "GMT+09:00", 37.57, 126.98),
                c("tokyo", "Tokyo", "GMT+09:00", 35.68, 139.69),
            ),
            Region(
                R.string.region_oceania,
                c("sydney", "Sydney", "Australia/Sydney", -33.87, 151.21),
                c("melbourne", "Melbourne", "Australia/Melbourne", -37.81, 144.96),
                c("auckland", "Auckland", "Pacific/Auckland", -36.85, 174.76),
            ),
            Region(
                R.string.region_africa,
                c("nairobi", "Nairobi", "GMT+03:00", -1.29, 36.82),
                c("johannesburg", "Johannesburg", "GMT+02:00", -26.20, 28.05),
            ),
            Region(
                R.string.region_america,
                c("new-york", "New York", "America/New_York", 40.71, -74.01),
                c("washington", "Washington", "America/New_York", 38.91, -77.04),
                c("miami", "Miami", "America/New_York", 25.76, -80.19),
                c("toronto", "Toronto", "America/Toronto", 43.65, -79.38),
                c("chicago", "Chicago", "America/Chicago", 41.88, -87.63),
                c("denver", "Denver", "America/Denver", 39.74, -104.99),
                c("los-angeles", "Los Angeles", "America/Los_Angeles", 34.05, -118.24),
                c("san-francisco", "San Francisco", "America/Los_Angeles", 37.77, -122.42),
                c("seattle", "Seattle", "America/Los_Angeles", 47.61, -122.33),
                c("vancouver", "Vancouver", "GMT-07:00", 49.28, -123.12),
                c("mexico-city", "Mexico City", "GMT-06:00", 19.43, -99.13),
                c("sao-paulo", "São Paulo", "GMT-03:00", -23.55, -46.63),
                c("buenos-aires", "Buenos Aires", "GMT-03:00", -34.60, -58.38),
            ),
        )

        val ALL: List<City> = REGIONS.flatMap { it.cities.asList() }.also { list -> list.forEachIndexed { i, city -> city.index = i } }
    }
}
