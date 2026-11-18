package app.deskstats

import android.content.SharedPreferences
import java.util.Locale
import java.util.TimeZone

/**
 * A city the clock and the weather can be switched to. Places without daylight saving time use
 * a fixed offset ("GMT+06:00") instead of a region name, so the time is right also on phones
 * whose time zone data predates a change of that region's rules.
 */
class City(val id: String, val name: String, val zoneId: String, val lat: Float, val lon: Float) {
    // Looked up once: the lists ask for it for every row of every frame.
    val zone: TimeZone by lazy { TimeZone.getTimeZone(zoneId) }
}

/** A heading of the city grid and the cities under it. */
class Region(val name: String, vararg val cities: City)

/** The user's cities and the chosen one; null means the phone's own time zone and the server's city. */
class Places(val saved: List<City>, val active: City?) {
    fun save(prefs: SharedPreferences) {
        prefs.edit().putString(SAVED, saved.joinToString(",") { it.id }).putString(ACTIVE, active?.id ?: "").apply()
    }

    fun with(city: City): Places = Places(if (saved.any { it.id == city.id }) saved else saved + city, city)

    fun without(city: City): Places = Places(saved.filter { it.id != city.id }, active?.takeIf { it.id != city.id })

    /** Puts a removed [city] back at [index], chosen again if it was. */
    fun restored(city: City, index: Int, chosen: Boolean): Places {
        if (saved.any { it.id == city.id }) return this
        val list = ArrayList(saved)
        list.add(index.coerceIn(0, list.size), city)
        return Places(list, if (chosen) city else active)
    }

    companion object {
        private const val SAVED = "places"
        private const val ACTIVE = "place"

        fun load(prefs: SharedPreferences): Places {
            val saved = (prefs.getString(SAVED, null) ?: "").split(',').mapNotNull { id -> ALL.firstOrNull { it.id == id } }.distinctBy { it.id }
            val active = prefs.getString(ACTIVE, null)?.let { id -> saved.firstOrNull { it.id == id } }
            return Places(saved, active)
        }

        /** The Russian name of a city the Mac names in Latin letters ("Bishkek"); others stay as they are. */
        fun named(city: String): String {
            val id = city.trim().lowercase(Locale.US).replace(' ', '-')
            return ALL.firstOrNull { it.id == id }?.name ?: city
        }

        private fun c(id: String, name: String, zone: String, lat: Double, lon: Double) = City(id, name, zone, lat.toFloat(), lon.toFloat())

        val REGIONS: List<Region> = listOf(
            Region(
                "Кыргызстан",
                c("bishkek", "Бишкек", "GMT+06:00", 42.87, 74.59),
                c("osh", "Ош", "GMT+06:00", 40.53, 72.80),
                c("jalal-abad", "Джалал-Абад", "GMT+06:00", 40.93, 73.00),
                c("karakol", "Каракол", "GMT+06:00", 42.49, 78.39),
                c("cholpon-ata", "Чолпон-Ата", "GMT+06:00", 42.65, 77.08),
                c("naryn", "Нарын", "GMT+06:00", 41.43, 76.00),
                c("talas", "Талас", "GMT+06:00", 42.52, 72.24),
                c("batken", "Баткен", "GMT+06:00", 40.06, 70.82),
                c("tokmok", "Токмок", "GMT+06:00", 42.84, 75.30),
            ),
            Region(
                "Центральная Азия",
                c("almaty", "Алматы", "GMT+05:00", 43.24, 76.89),
                c("astana", "Астана", "GMT+05:00", 51.17, 71.43),
                c("shymkent", "Шымкент", "GMT+05:00", 42.32, 69.59),
                c("tashkent", "Ташкент", "GMT+05:00", 41.30, 69.24),
                c("samarkand", "Самарканд", "GMT+05:00", 39.65, 66.96),
                c("dushanbe", "Душанбе", "GMT+05:00", 38.56, 68.79),
                c("ashgabat", "Ашхабад", "GMT+05:00", 37.95, 58.38),
            ),
            Region(
                "Россия",
                c("moscow", "Москва", "GMT+03:00", 55.75, 37.62),
                c("saint-petersburg", "Санкт-Петербург", "GMT+03:00", 59.94, 30.31),
                c("kazan", "Казань", "GMT+03:00", 55.79, 49.12),
                c("sochi", "Сочи", "GMT+03:00", 43.59, 39.72),
                c("yekaterinburg", "Екатеринбург", "GMT+05:00", 56.84, 60.61),
                c("novosibirsk", "Новосибирск", "GMT+07:00", 55.03, 82.92),
                c("krasnoyarsk", "Красноярск", "GMT+07:00", 56.01, 92.87),
                c("irkutsk", "Иркутск", "GMT+08:00", 52.29, 104.30),
                c("vladivostok", "Владивосток", "GMT+10:00", 43.12, 131.89),
            ),
            Region(
                "Кавказ и Турция",
                c("baku", "Баку", "GMT+04:00", 40.41, 49.87),
                c("tbilisi", "Тбилиси", "GMT+04:00", 41.72, 44.79),
                c("yerevan", "Ереван", "GMT+04:00", 40.18, 44.51),
                c("istanbul", "Стамбул", "GMT+03:00", 41.01, 28.98),
                c("antalya", "Анталья", "GMT+03:00", 36.90, 30.70),
            ),
            Region(
                "Ближний Восток",
                c("dubai", "Дубай", "GMT+04:00", 25.20, 55.27),
                c("doha", "Доха", "GMT+03:00", 25.29, 51.53),
                c("riyadh", "Эр-Рияд", "GMT+03:00", 24.71, 46.68),
                c("tel-aviv", "Тель-Авив", "Asia/Jerusalem", 32.09, 34.78),
            ),
            Region(
                "Европа",
                c("minsk", "Минск", "GMT+03:00", 53.90, 27.57),
                c("kyiv", "Киев", "Europe/Kiev", 50.45, 30.52),
                c("london", "Лондон", "Europe/London", 51.51, -0.13),
                c("paris", "Париж", "Europe/Paris", 48.86, 2.35),
                c("berlin", "Берлин", "Europe/Berlin", 52.52, 13.40),
                c("amsterdam", "Амстердам", "Europe/Amsterdam", 52.37, 4.90),
                c("madrid", "Мадрид", "Europe/Madrid", 40.42, -3.70),
                c("lisbon", "Лиссабон", "Europe/Lisbon", 38.72, -9.14),
                c("rome", "Рим", "Europe/Rome", 41.90, 12.50),
                c("zurich", "Цюрих", "Europe/Zurich", 47.37, 8.54),
                c("vienna", "Вена", "Europe/Vienna", 48.21, 16.37),
                c("prague", "Прага", "Europe/Prague", 50.08, 14.44),
                c("warsaw", "Варшава", "Europe/Warsaw", 52.23, 21.01),
                c("stockholm", "Стокгольм", "Europe/Stockholm", 59.33, 18.07),
                c("helsinki", "Хельсинки", "Europe/Helsinki", 60.17, 24.94),
                c("athens", "Афины", "Europe/Athens", 37.98, 23.73),
            ),
            Region(
                "Азия",
                c("islamabad", "Исламабад", "GMT+05:00", 33.69, 73.05),
                c("delhi", "Дели", "GMT+05:30", 28.61, 77.21),
                c("mumbai", "Мумбаи", "GMT+05:30", 19.08, 72.88),
                c("dhaka", "Дакка", "GMT+06:00", 23.81, 90.41),
                c("bangkok", "Бангкок", "GMT+07:00", 13.76, 100.50),
                c("phuket", "Пхукет", "GMT+07:00", 7.88, 98.39),
                c("hanoi", "Ханой", "GMT+07:00", 21.03, 105.85),
                c("jakarta", "Джакарта", "GMT+07:00", -6.21, 106.85),
                c("bali", "Бали", "GMT+08:00", -8.65, 115.22),
                c("singapore", "Сингапур", "GMT+08:00", 1.35, 103.82),
                c("kuala-lumpur", "Куала-Лумпур", "GMT+08:00", 3.14, 101.69),
                c("hong-kong", "Гонконг", "GMT+08:00", 22.32, 114.17),
                c("beijing", "Пекин", "GMT+08:00", 39.90, 116.41),
                c("shanghai", "Шанхай", "GMT+08:00", 31.23, 121.47),
                c("urumqi", "Урумчи", "GMT+08:00", 43.83, 87.62),
                c("ulaanbaatar", "Улан-Батор", "GMT+08:00", 47.89, 106.91),
                c("seoul", "Сеул", "GMT+09:00", 37.57, 126.98),
                c("tokyo", "Токио", "GMT+09:00", 35.68, 139.69),
            ),
            Region(
                "Австралия и Океания",
                c("sydney", "Сидней", "Australia/Sydney", -33.87, 151.21),
                c("melbourne", "Мельбурн", "Australia/Melbourne", -37.81, 144.96),
                c("auckland", "Окленд", "Pacific/Auckland", -36.85, 174.76),
            ),
            Region(
                "Африка",
                c("nairobi", "Найроби", "GMT+03:00", -1.29, 36.82),
                c("johannesburg", "Йоханнесбург", "GMT+02:00", -26.20, 28.05),
            ),
            Region(
                "Америка",
                c("new-york", "Нью-Йорк", "America/New_York", 40.71, -74.01),
                c("washington", "Вашингтон", "America/New_York", 38.91, -77.04),
                c("miami", "Майами", "America/New_York", 25.76, -80.19),
                c("toronto", "Торонто", "America/Toronto", 43.65, -79.38),
                c("chicago", "Чикаго", "America/Chicago", 41.88, -87.63),
                c("denver", "Денвер", "America/Denver", 39.74, -104.99),
                c("los-angeles", "Лос-Анджелес", "America/Los_Angeles", 34.05, -118.24),
                c("san-francisco", "Сан-Франциско", "America/Los_Angeles", 37.77, -122.42),
                c("seattle", "Сиэтл", "America/Los_Angeles", 47.61, -122.33),
                c("vancouver", "Ванкувер", "GMT-07:00", 49.28, -123.12),
                c("mexico-city", "Мехико", "GMT-06:00", 19.43, -99.13),
                c("sao-paulo", "Сан-Паулу", "GMT-03:00", -23.55, -46.63),
                c("buenos-aires", "Буэнос-Айрес", "GMT-03:00", -34.60, -58.38),
            ),
        )

        val ALL: List<City> = REGIONS.flatMap { it.cities.asList() }
    }
}
